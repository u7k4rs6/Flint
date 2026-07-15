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
pub const SYS_READ_LINE: u64 = 3;
pub const SYS_SHELL_DISPATCH: u64 = 4;

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
/// every page in the range is mapped, every page is user-accessible, and
/// (when `require_writable`, for a syscall about to copy data *to* the
/// caller, like `SYS_READ_LINE`) every page is writable too. Bounds and
/// overflow-checks the range before it is used for anything, per Doc 3
/// section 4. Never dereferences `addr` itself.
fn validate_user_range(addr: u64, len: u64, require_writable: bool) -> Result<(), ()> {
    if len == 0 {
        return Ok(());
    }
    let end = addr.checked_add(len).ok_or(())?;
    let start_page = Page::<Size4KiB>::containing_address(VirtAddr::new(addr));
    let end_page = Page::<Size4KiB>::containing_address(VirtAddr::new(end - 1));

    let mut required = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
    if require_writable {
        required |= PageTableFlags::WRITABLE;
    }

    crate::memory::paging::with_mapper(|mapper| {
        for page in Page::range_inclusive(start_page, end_page) {
            match mapper.translate(page.start_address()) {
                TranslateResult::Mapped { flags, .. } => {
                    if !flags.contains(required) {
                        return Err(());
                    }
                }
                _ => return Err(()),
            }
        }
        Ok(())
    })
}

// ===== Fault-safe user copies (Doc 3 sections 4, 7; the threat table's
// "fault mid-copy across the boundary" row) =====
//
// `validate_user_range` above closes the gap between "the argument looks
// like a valid pointer" and "the argument is actually safe to touch," but
// it is still a check-then-use: nothing stops the mapping it just walked
// from being revoked before the copy that follows actually runs (a classic
// TOCTOU window -- see COMPLEXITY.md's user-pointer-check entry). Nothing
// in Flint revokes a mapping mid-syscall today (single process, single
// core, interrupts disabled for the whole `int 0x80` body -- see below),
// but Doc 3 requires the copy path itself handle a fault gracefully and
// return an error rather than panic, architecturally, not "only if
// currently reachable."
//
// The mechanism: each of `copy_from_user_byte`/`copy_to_user_byte` records
// (in `RECOVERY_IP`) the address of its own fallthrough label immediately
// before doing the one instruction that might fault, so `page_fault_handler`
// (`interrupts.rs`) can recognize "this specific kernel-mode fault happened
// inside a recoverable copy" and redirect execution there -- via
// `InterruptStackFrame::as_mut()` rewriting the saved `rip` -- instead of
// falling through to its normal panic. This is sound as a single global
// (not per-CPU -- Flint has no SMP, see DECISIONS.md) pair of flags
// specifically because `int 0x80` runs with interrupts disabled for its
// entire body (already true today, see `sys_read_line`'s doc comment
// below): no timer/keyboard IRQ can preempt an in-flight copy and race
// `RECOVERY_IP`/`COPY_FAULTED`; only the copy's own synchronous fault can
// touch them, and exceptions aren't gated by IF.
static RECOVERY_IP: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static COPY_FAULTED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Called only from `page_fault_handler`. Atomically reads and clears the
/// current recovery address: `Some(ip)` means the fault that just happened
/// is inside a recoverable copy and should redirect to `ip` instead of
/// panicking; `None` means it's a genuine kernel bug.
pub(crate) fn take_copy_recovery_ip() -> Option<u64> {
    let ip = RECOVERY_IP.swap(0, core::sync::atomic::Ordering::Relaxed);
    if ip == 0 {
        None
    } else {
        Some(ip)
    }
}

/// Called only from `page_fault_handler`, immediately before redirecting to
/// the address `take_copy_recovery_ip` returned -- tells the waiting
/// `copy_from_user_byte`/`copy_to_user_byte` call that its access faulted,
/// so it returns `Err` instead of treating whatever garbage ended up in its
/// output register as real data.
pub(crate) fn mark_copy_faulted() {
    COPY_FAULTED.store(true, core::sync::atomic::Ordering::Relaxed);
}

/// Reads one byte from a user address that `validate_user_range` has
/// already approved, but treats the read itself as still capable of
/// faulting (the TOCTOU window above) rather than trusting the earlier
/// check unconditionally. `#[inline(never)]` so the `2f` label inside the
/// asm block below names a stable, unique address `page_fault_handler` can
/// redirect to -- inlining could duplicate or relocate it unpredictably.
#[inline(never)]
fn copy_from_user_byte(ptr: u64) -> Result<u8, ()> {
    COPY_FAULTED.store(false, core::sync::atomic::Ordering::Relaxed);
    let recovery_slot = &RECOVERY_IP as *const core::sync::atomic::AtomicU64 as u64;
    let value: u64;
    // SAFETY: `ptr` was already range/mapped/user-accessible-checked by
    // this call's caller via `validate_user_range`; if the mapping was
    // revoked in the (currently unreachable, but architecturally possible)
    // window between that check and this read, the resulting page fault is
    // caught by `page_fault_handler`'s `take_copy_recovery_ip` check
    // (wired up specifically for this address range) rather than
    // dereferencing further or corrupting kernel state -- `movzx` either
    // completes and leaves a real byte in `{out}`, or never completes and
    // execution resumes at `2:` with `{out}` left undefined, which is why
    // the caller only trusts `value` when `COPY_FAULTED` is false.
    unsafe {
        core::arch::asm!(
            "lea {tmp}, [rip + 2f]",
            "mov qword ptr [{slot}], {tmp}",
            "movzx {out}, byte ptr [{ptr}]",
            "2:",
            tmp = out(reg) _,
            slot = in(reg) recovery_slot,
            out = out(reg) value,
            ptr = in(reg) ptr,
            options(nostack),
        );
    }
    // Idempotent on the fault path (the handler already cleared it as part
    // of the redirect) and necessary on the success path (nothing else
    // would clear a stale recovery address before the *next* unrelated
    // fault, kernel or otherwise, could be misattributed to this copy).
    RECOVERY_IP.store(0, core::sync::atomic::Ordering::Relaxed);
    if COPY_FAULTED.load(core::sync::atomic::Ordering::Relaxed) {
        Err(())
    } else {
        Ok(value as u8)
    }
}

/// Writes one byte to a user address `validate_user_range` has already
/// approved as writable, with the same fault-recovery discipline as
/// `copy_from_user_byte`.
#[inline(never)]
fn copy_to_user_byte(ptr: u64, val: u8) -> Result<(), ()> {
    COPY_FAULTED.store(false, core::sync::atomic::Ordering::Relaxed);
    let recovery_slot = &RECOVERY_IP as *const core::sync::atomic::AtomicU64 as u64;
    // SAFETY: same reasoning as `copy_from_user_byte`, for the write
    // direction -- `validate_user_range(..., require_writable: true)` is
    // this call's caller's contract.
    unsafe {
        core::arch::asm!(
            "lea {tmp}, [rip + 2f]",
            "mov qword ptr [{slot}], {tmp}",
            "mov byte ptr [{ptr}], {val}",
            "2:",
            tmp = out(reg) _,
            slot = in(reg) recovery_slot,
            ptr = in(reg) ptr,
            val = in(reg_byte) val,
            options(nostack),
        );
    }
    RECOVERY_IP.store(0, core::sync::atomic::Ordering::Relaxed);
    if COPY_FAULTED.load(core::sync::atomic::Ordering::Relaxed) {
        Err(())
    } else {
        Ok(())
    }
}

fn sys_write(ptr: u64, len: u64) -> u64 {
    if len > MAX_WRITE_LEN {
        crate::log_warn!("SYS_WRITE rejected: length {} exceeds max", len);
        return SYSCALL_ERROR;
    }
    if validate_user_range(ptr, len, false).is_err() {
        crate::log_warn!(
            "SYS_WRITE rejected: pointer {:#x} len {} is not a valid user range",
            ptr,
            len
        );
        return SYSCALL_ERROR;
    }

    // `validate_user_range` just confirmed every page in `[ptr, ptr + len)`
    // is present and user-accessible, but the copy itself still goes
    // through `copy_from_user_byte` rather than a bulk, unchecked read --
    // the copy-in half of the checked copy-in/copy-out pattern Doc 3
    // section 4 requires, fault-safe against the mapping being revoked in
    // the window between that check and this loop. `len` is already
    // bounded by `MAX_WRITE_LEN` above, so a fixed-size kernel buffer
    // covers every valid call.
    let mut buf = [0u8; MAX_WRITE_LEN as usize];
    for (i, slot) in buf.iter_mut().enumerate().take(len as usize) {
        match copy_from_user_byte(ptr + i as u64) {
            Ok(byte) => *slot = byte,
            Err(()) => {
                crate::log_warn!(
                    "SYS_WRITE rejected: pointer {:#x} faulted mid-copy at offset {}",
                    ptr,
                    i
                );
                return SYSCALL_ERROR;
            }
        }
    }
    let bytes = &buf[..len as usize];
    match core::str::from_utf8(bytes) {
        Ok(s) => crate::serial_println!("[user] {}", s),
        Err(_) => crate::serial_println!("[user] <{} non-utf8 bytes>", len),
    }
    len
}

const MAX_LINE_LEN: u64 = 1024;

type InputByteFn = fn() -> u8;

fn default_input_byte() -> u8 {
    crate::serial::SERIAL1.lock().receive()
}

static INPUT_SOURCE: spin::Mutex<InputByteFn> = spin::Mutex::new(default_input_byte);
static SCRIPTED_INPUT: spin::Mutex<&'static [u8]> = spin::Mutex::new(b"");

fn scripted_input_byte() -> u8 {
    let mut remaining = SCRIPTED_INPUT.lock();
    match remaining.split_first() {
        Some((&byte, rest)) => {
            *remaining = rest;
            byte
        }
        // Scripted input exhausted with no trailing line ending: report a
        // line ending rather than spin forever with an empty buffer.
        None => b'\n',
    }
}

/// Makes `SYS_READ_LINE` pull bytes from `bytes` instead of the real UART.
/// Exists so a test can drive the exact same ring-3/syscall/echo/dispatch
/// pipeline a human typing at a real terminal would, without depending on
/// external process stdin actually reaching this QEMU instance (which,
/// while it does work -- see DECISIONS.md -- would make a bare
/// `cargo test` hang and fail on this test whenever nothing happens to be
/// piped into it).
pub fn set_scripted_input(bytes: &'static [u8]) {
    *SCRIPTED_INPUT.lock() = bytes;
    *INPUT_SOURCE.lock() = scripted_input_byte;
}

/// Blocks (busy-waiting on the UART's own status register, not a CPU
/// spin) reading bytes until a line ending or `cap` is reached, echoing
/// each accepted byte back over serial as it arrives -- Doc 2 section 8's
/// "reads input over serial through a syscall [and] echoes it." Writes
/// into the caller's buffer only after validating it as a writable user
/// range (the copy-out half of the checked copy pattern Doc 3 section 4
/// requires; `SYS_WRITE` above only ever exercised copy-in).
///
/// Runs with interrupts disabled for its whole (potentially long, human
/// typing speed) duration, since `int 0x80` uses an interrupt gate: the
/// timer and scheduler are effectively paused while waiting for a line.
/// Acceptable for a single foreground interactive shell (M7's actual
/// scope); a real implementation would re-enable interrupts around the
/// blocking wait. See DECISIONS.md.
fn sys_read_line(ptr: u64, cap: u64) -> u64 {
    if cap == 0 || cap > MAX_LINE_LEN {
        crate::log_warn!("SYS_READ_LINE rejected: bad capacity {}", cap);
        return SYSCALL_ERROR;
    }
    if validate_user_range(ptr, cap, true).is_err() {
        crate::log_warn!(
            "SYS_READ_LINE rejected: buffer {:#x} cap {} is not a valid writable user range",
            ptr,
            cap
        );
        return SYSCALL_ERROR;
    }

    let mut len: u64 = 0;
    loop {
        let read_byte: InputByteFn = *INPUT_SOURCE.lock();
        let byte = read_byte();
        if byte == b'\r' || byte == b'\n' {
            crate::serial::SERIAL1.lock().send(b'\n');
            break;
        }
        if len < cap {
            // `validate_user_range` confirmed `[ptr, ptr + cap)` is mapped,
            // user-accessible, and writable, and `len < cap` keeps this
            // write inside that validated range -- but the write itself
            // still goes through `copy_to_user_byte` rather than a raw
            // pointer write, fault-safe against the mapping being revoked
            // since the check (Doc 3 section 4's copy-out half).
            if copy_to_user_byte(ptr + len, byte).is_err() {
                crate::log_warn!(
                    "SYS_READ_LINE rejected: buffer {:#x} faulted mid-copy at offset {}",
                    ptr,
                    len
                );
                return SYSCALL_ERROR;
            }
            crate::serial::SERIAL1.lock().send(byte);
            len += 1;
        }
        // Bytes beyond `cap` are still consumed (so the line ending is
        // found and the terminal's own echo stays in sync) but dropped.
    }
    len
}

fn sys_shell_dispatch(ptr: u64, len: u64) -> u64 {
    if len > MAX_LINE_LEN {
        crate::log_warn!("SYS_SHELL_DISPATCH rejected: length {} exceeds max", len);
        return SYSCALL_ERROR;
    }
    if validate_user_range(ptr, len, false).is_err() {
        crate::log_warn!(
            "SYS_SHELL_DISPATCH rejected: pointer {:#x} len {} is not a valid user range",
            ptr,
            len
        );
        return SYSCALL_ERROR;
    }

    // Same fault-safe copy-in as `sys_write`; `len` is already bounded by
    // `MAX_LINE_LEN` above.
    let mut buf = [0u8; MAX_LINE_LEN as usize];
    for (i, slot) in buf.iter_mut().enumerate().take(len as usize) {
        match copy_from_user_byte(ptr + i as u64) {
            Ok(byte) => *slot = byte,
            Err(()) => {
                crate::log_warn!(
                    "SYS_SHELL_DISPATCH rejected: pointer {:#x} faulted mid-copy at offset {}",
                    ptr,
                    i
                );
                return SYSCALL_ERROR;
            }
        }
    }
    let bytes = &buf[..len as usize];
    let line = match core::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => {
            crate::log_warn!("SYS_SHELL_DISPATCH: non-utf8 input ignored");
            return 0;
        }
    };

    // exit/quit end the ring-3 loop; signaled back to it (rax == 1) rather
    // than handled here, since only the ring-3 process itself can decide
    // to issue its own SYS_EXIT -- the kernel does not tear down tasks it
    // did not start tearing down (see DECISIONS.md).
    if line.trim() == "exit" || line.trim() == "quit" {
        crate::serial_println!("bye");
        return 1;
    }

    let response = crate::shell::dispatch(line);
    if !response.is_empty() {
        crate::serial_println!("{}", response);
    }
    0
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
    crate::log_info!("exited via SYS_EXIT");
    let hook = *EXIT_HOOK.lock();
    hook()
}

/// Dispatches a validated syscall number to its handler. Called only from
/// `syscall_entry`'s asm, never directly.
extern "C" fn syscall_dispatch(num: u64, arg1: u64, arg2: u64) -> u64 {
    match num {
        SYS_WRITE => sys_write(arg1, arg2),
        SYS_EXIT => sys_exit(),
        SYS_READ_LINE => sys_read_line(arg1, arg2),
        SYS_SHELL_DISPATCH => sys_shell_dispatch(arg1, arg2),
        _ => {
            crate::log_warn!("rejected unknown syscall number {}", num);
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

#[cfg(test)]
mod tests {
    use super::{copy_from_user_byte, copy_to_user_byte};
    use x86_64::structures::paging::{Page, PageTableFlags, Size4KiB};
    use x86_64::VirtAddr;

    /// Doc 3 sections 4/7's "a copy path that catches a mid-copy fault and
    /// returns an error rather than panicking" gate. Maps a page the same
    /// way a validated user pointer would look (present, user-accessible,
    /// writable), then deliberately unmaps it -- the TOCTOU window Doc 3
    /// requires a copy to survive, since nothing in Flint's normal
    /// operation ever does this on its own -- and proves
    /// `copy_from_user_byte`/`copy_to_user_byte` return `Err` instead of
    /// taking the kernel down with a page fault. Reaching the assertions
    /// *at all* is itself part of the proof: before this mechanism existed,
    /// the second `copy_from_user_byte` call below would have panicked the
    /// whole kernel (`page_fault_handler`'s unconditional final `panic!`),
    /// which would fail this test the same way any other kernel panic
    /// fails the harness, not return cleanly to this point.
    #[test_case]
    fn copy_helpers_return_err_instead_of_panicking_on_a_mid_copy_fault() {
        const TEST_ADDR: u64 = 0x_7777_7777_0000;
        let page: Page<Size4KiB> = Page::containing_address(VirtAddr::new(TEST_ADDR));
        let flags = PageTableFlags::PRESENT
            | PageTableFlags::WRITABLE
            | PageTableFlags::USER_ACCESSIBLE
            | PageTableFlags::NO_EXECUTE;
        crate::memory::paging::map_page(page, flags).expect("failed to map test page");

        // The happy path, on the still-mapped page: proves the fixup
        // machinery (recording and clearing a recovery point around every
        // access) doesn't break a normal, successful copy.
        assert!(copy_to_user_byte(TEST_ADDR, 0x42).is_ok());
        assert_eq!(copy_from_user_byte(TEST_ADDR), Ok(0x42));

        // Revoke the mapping -- the TOCTOU window: a pointer that was
        // valid at `validate_user_range` time no longer is.
        crate::memory::paging::unmap_page(page).expect("failed to unmap test page");

        assert_eq!(copy_from_user_byte(TEST_ADDR), Err(()));
        assert_eq!(copy_to_user_byte(TEST_ADDR, 0x43), Err(()));

        // The kernel is still alive, and the mechanism left no stale
        // global state behind: a later, unrelated copy on a freshly
        // mapped page still works correctly.
        crate::memory::paging::map_page(page, flags).expect("failed to remap test page");
        assert!(copy_to_user_byte(TEST_ADDR, 0x44).is_ok());
        assert_eq!(copy_from_user_byte(TEST_ADDR), Ok(0x44));
    }
}
