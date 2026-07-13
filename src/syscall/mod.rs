//! The syscall trust boundary (Doc 3 section 4): the one controlled doorway
//! from ring 3 into ring 0. Every argument that crosses it is treated as
//! hostile until validated -- this is the single most important safety
//! control in the kernel (Doc 2 section 7.2).
//!
//! `int 0x80` was chosen over `syscall`/`sysret` (Doc 2 section 7.2's other
//! listed option) because it is simpler to stand up correctly first: it
//! reuses the IDT and the existing ring-3 IRETQ transition machinery
//! instead of introducing three new MSRs (STAR, LSTAR, SFMASK) as an
//! additional place to get an offset wrong on the hardest milestone.
//! `syscall`/`sysret` is a natural follow-up once this path is solid.

use core::arch::naked_asm;
use x86_64::structures::idt::InterruptDescriptorTable;
use x86_64::structures::paging::mapper::TranslateResult;
use x86_64::structures::paging::{Page, PageTableFlags, Size4KiB, Translate};
use x86_64::VirtAddr;

pub const SYS_WRITE: u64 = 1;
pub const SYS_EXIT: u64 = 2;

/// Sentinel returned to ring 3 in `rax` when a syscall is rejected. Not
/// zero (a plausible "0 bytes written" success value), and not a value a
/// legitimate `SYS_WRITE` byte count could ever produce given `MAX_LEN`
/// below.
pub const SYSCALL_ERROR: u64 = u64::MAX;

const MAX_WRITE_LEN: u64 = 4096;

pub fn register(idt: &mut InterruptDescriptorTable) {
    // SAFETY: `syscall_entry` is a valid, present code address (a function
    // in this binary) that follows the manual GPR save/dispatch/restore
    // protocol its own definition documents; `set_handler_addr` requires
    // that contract, which is what makes this call unsafe.
    let options =
        unsafe { idt[0x80].set_handler_addr(VirtAddr::new(syscall_entry as *const () as u64)) };
    // Ring 3 is only allowed to raise vectors whose IDT gate DPL is 3 (else
    // the CPU raises #GP on `int 0x80` from CPL=3) -- this is what actually
    // opens the doorway.
    options.set_privilege_level(x86_64::PrivilegeLevel::Ring3);
}

/// Validates that `[addr, addr + len)` is entirely within user space:
/// every page in the range is mapped, and every page is user-accessible.
/// Bounds and overflow-checks the range before it is used for anything,
/// per Doc 3 section 4. Never dereferences `addr` itself.
fn validate_user_range(addr: u64, len: u64) -> Result<(), ()> {
    if len == 0 {
        return Ok(());
    }
    let end = addr.checked_add(len).ok_or(())?;
    let start_page = Page::<Size4KiB>::containing_address(VirtAddr::new(addr));
    let end_page = Page::<Size4KiB>::containing_address(VirtAddr::new(end - 1));

    crate::memory::paging::with_mapper(|mapper| {
        for page in Page::range_inclusive(start_page, end_page) {
            match mapper.translate(page.start_address()) {
                TranslateResult::Mapped { flags, .. } => {
                    if !flags.contains(PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE) {
                        return Err(());
                    }
                }
                _ => return Err(()),
            }
        }
        Ok(())
    })
}

fn sys_write(ptr: u64, len: u64) -> u64 {
    if len > MAX_WRITE_LEN {
        crate::serial_println!("[syscall] SYS_WRITE rejected: length {} exceeds max", len);
        return SYSCALL_ERROR;
    }
    if validate_user_range(ptr, len).is_err() {
        crate::serial_println!(
            "[syscall] SYS_WRITE rejected: pointer {:#x} len {} is not a valid user range",
            ptr,
            len
        );
        return SYSCALL_ERROR;
    }

    // SAFETY: `validate_user_range` just confirmed every page in
    // `[ptr, ptr + len)` is present and user-accessible, and `len` is
    // bounded above by `MAX_WRITE_LEN`, so this reads only memory the
    // calling user program was actually granted -- the copy-in half of
    // the checked copy-in/copy-out pattern Doc 3 section 4 requires.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
    match core::str::from_utf8(bytes) {
        Ok(s) => crate::serial_println!("[user] {}", s),
        Err(_) => crate::serial_println!("[user] <{} non-utf8 bytes>", len),
    }
    len
}

type ExitHook = fn() -> !;

fn default_exit_hook() -> ! {
    crate::hlt_loop();
}

static EXIT_HOOK: spin::Mutex<ExitHook> = spin::Mutex::new(default_exit_hook);

/// Overrides what `SYS_EXIT` does after logging (default: park in
/// `hlt_loop`, since a real multi-process kernel would tear down just this
/// task and resume the scheduler -- process teardown is machinery M6 does
/// not build, see DECISIONS.md). Exists so a test can observe "the user
/// program's `SYS_EXIT` was reached" by exiting QEMU instead, without
/// `sys_exit` itself needing to know anything about the test harness.
pub fn set_exit_hook(hook: ExitHook) {
    *EXIT_HOOK.lock() = hook;
}

/// Never returns to the caller: ends the (single, demo) user program.
fn sys_exit() -> ! {
    crate::serial_println!("[user] exited via SYS_EXIT");
    let hook = *EXIT_HOOK.lock();
    hook()
}

/// Dispatches a validated syscall number to its handler. Called only from
/// `syscall_entry`'s asm, never directly.
extern "C" fn syscall_dispatch(num: u64, arg1: u64, arg2: u64) -> u64 {
    match num {
        SYS_WRITE => sys_write(arg1, arg2),
        SYS_EXIT => sys_exit(),
        _ => {
            crate::serial_println!("[syscall] rejected unknown syscall number {}", num);
            SYSCALL_ERROR
        }
    }
}

/// The syscall entry stub: one of the three places inline assembly is
/// unavoidable (Doc 2 section 1). `int 0x80` gives us only the standard
/// hardware interrupt frame (IP/CS/FLAGS/SP/SS) -- the syscall number and
/// arguments a user program passes in `rax`/`rdi`/`rsi` are ordinary
/// general-purpose registers the CPU does not save on interrupt entry, so
/// an `extern "x86-interrupt" fn` (which never exposes GPRs to the handler
/// body) cannot read them. This naked stub saves exactly the registers it
/// needs, calls into normal Rust with them as arguments, writes the result
/// back into the saved `rax` slot, restores everything, and `iretq`s back
/// to ring 3.
///
/// Register layout after `push rax; push rdi; push rsi` (stack grows
/// down, so the *last* push is at the lowest address): `[rsp+0]`=rsi,
/// `[rsp+8]`=rdi, `[rsp+16]`=rax. `syscall_dispatch`'s return value is
/// written to `[rsp+16]` so the final `pop rax` loads it as the value
/// `iretq` hands back to the caller.
#[unsafe(naked)]
unsafe extern "C" fn syscall_entry() {
    naked_asm!(
        "push rax",
        "push rdi",
        "push rsi",
        "mov rdx, rsi", // arg2 (original rsi), read before it's overwritten
        "mov rsi, rdi", // arg1 (original rdi), read before it's overwritten
        "mov rdi, rax", // syscall number (original rax)
        "call {dispatch}",
        "mov [rsp + 16], rax",
        "pop rsi",
        "pop rdi",
        "pop rax",
        "iretq",
        dispatch = sym syscall_dispatch,
    )
}
