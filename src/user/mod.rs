//! Ring 3 setup: mapping an isolated, minimal user address range and
//! transitioning into it. No process model or ELF loader yet (M8 stretch)
//! -- this builds exactly enough to run the hand-written demo programs in
//! `program.rs` and `shell_program.rs` and prove the ring 3 / syscall /
//! validation machinery actually works end to end.

pub mod program;
pub mod shell_program;

use crate::memory::paging::map_page;
use x86_64::structures::paging::{Mapper, Page, PageTableFlags, Size4KiB};
use x86_64::VirtAddr;

/// Deliberately far from every other mapped region (the kernel image, the
/// heap at `0x_4444_4444_0000`, the lazy demand-paging region at
/// `0x_5555_5555_0000`, and the bootloader's physical-memory offset
/// mapping): a distinct top-level (PML4) page-table slot, so mapping code
/// here can never share -- and therefore never accidentally weaken or be
/// weakened by -- an existing mapping's page-table entries at any level.
/// W xor X holds by construction: the code page is mapped without
/// `WRITABLE`; the stack (and shell line buffer) pages are mapped with
/// `WRITABLE` and `NO_EXECUTE`. Code lives in its own separate PML4 slot
/// from the writable pages too, for the same reason (see DECISIONS.md
/// M6). The stack and the buffer share a slot with each other on purpose:
/// both want the exact same flags (`WRITABLE`, `USER_ACCESSIBLE`,
/// `NO_EXECUTE`), so sharing intermediate page-table entries between them
/// carries none of the cross-mapping risk that motivated separating code
/// from data in the first place.
const USER_CODE_ADDR: u64 = 0x_2000_0000_0000;
const USER_STACK_TOP: u64 = 0x_3000_0000_1000;
const USER_STACK_SIZE: u64 = 4096;

/// Maps a code page at `USER_CODE_ADDR` and a stack page at
/// `USER_STACK_TOP`, copies `copy_len` bytes from `program_addr` into the
/// code page, then makes it read-only + executable. Returns
/// `(entry_point, stack_top)` ready for `enter_user_mode`.
fn map_program(program_addr: *const u8, copy_len: usize) -> (VirtAddr, VirtAddr) {
    let code_page: Page<Size4KiB> = Page::containing_address(VirtAddr::new(USER_CODE_ADDR));
    // Mapped WRITABLE only long enough to copy the program's bytes in
    // below: even ring 0 cannot write through a page table entry that
    // lacks WRITABLE (CR0.WP applies to supervisor writes too), so the
    // copy would itself page-fault if the leaf were already read-only.
    // `update_flags` drops WRITABLE again right after, before this page
    // is ever reachable from ring 3, so W xor X still holds for the code
    // ring 3 actually gets to run.
    let writable_code_flags =
        PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;
    map_page(code_page, writable_code_flags).expect("failed to map user code page");

    let stack_page: Page<Size4KiB> =
        Page::containing_address(VirtAddr::new(USER_STACK_TOP - USER_STACK_SIZE));
    let stack_flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::USER_ACCESSIBLE
        | PageTableFlags::NO_EXECUTE;
    map_page(stack_page, stack_flags).expect("failed to map user stack page");

    // SAFETY: the caller guarantees `[program_addr, program_addr +
    // copy_len)` is readable kernel memory (both call sites below forward
    // that to their own program modules' documented guarantees); `code_page`
    // was just mapped fresh above, writable, with exactly `copy_len`-or-more
    // bytes of room (one 4 KiB page).
    unsafe {
        let dst = USER_CODE_ADDR as *mut u8;
        core::ptr::copy_nonoverlapping(program_addr, dst, copy_len);
    }

    let read_exec_code_flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
    crate::memory::paging::with_mapper(|mapper| {
        // SAFETY: dropping only the WRITABLE bit on a page nothing else
        // references (it was just mapped by this same function) cannot
        // corrupt any other mapping or in-flight access.
        unsafe { mapper.update_flags(code_page, read_exec_code_flags) }
            .expect("failed to make user code page read-only")
            .flush();
    });

    (VirtAddr::new(USER_CODE_ADDR), VirtAddr::new(USER_STACK_TOP))
}

/// Maps the user code and stack pages and copies the M6 demo program's
/// machine code into place. Returns `(entry_point, stack_top)` ready for
/// `enter_user_mode`.
pub fn setup() -> (VirtAddr, VirtAddr) {
    // SAFETY: `program::user_entry`'s own doc comment guarantees the
    // first `COPY_LEN` bytes from its address are safe to read (kernel
    // .text, at least that many bytes of valid surrounding code).
    let src = program::user_entry as *const () as *const u8;
    map_program(src, program::COPY_LEN)
}

/// Maps the user code and stack pages for the interactive shell, copies
/// `shell_program::shell_entry`'s machine code into place, and maps its
/// line-read buffer as a writable, non-executable user page. Returns
/// `(entry_point, stack_top)` ready for `enter_user_mode`.
pub fn setup_shell() -> (VirtAddr, VirtAddr) {
    // SAFETY: `shell_program::shell_entry`'s own doc comment guarantees
    // the first `COPY_LEN` bytes from its address are safe to read.
    let src = shell_program::shell_entry as *const () as *const u8;
    let result = map_program(src, shell_program::COPY_LEN);

    let buf_page: Page<Size4KiB> =
        Page::containing_address(VirtAddr::new(shell_program::BUF_ADDR));
    let buf_flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::USER_ACCESSIBLE
        | PageTableFlags::NO_EXECUTE;
    map_page(buf_page, buf_flags).expect("failed to map shell line buffer page");

    result
}

/// Transitions from ring 0 to ring 3 at `entry`, with `stack_top` as the
/// user stack. Never returns to its caller -- control only comes back to
/// the kernel later, through a syscall or a fault.
///
/// # Safety
/// `entry` and `stack_top` must both be mapped, user-accessible pages
/// (`stack_top` writable and non-executable; `entry` executable), set up
/// by `setup` above. Must only be called once the GDT/TSS (`gdt::init`,
/// including `rsp0`) are already loaded, since the very first instruction
/// executed in ring 3 could immediately fault or syscall back into ring 0.
pub unsafe fn enter_user_mode(entry: VirtAddr, stack_top: VirtAddr) -> ! {
    let (user_cs, user_ds) = crate::gdt::user_selectors();
    let cs = user_cs.0 as u64;
    let ds = user_ds.0 as u64;

    // SAFETY: forwarded to this function's own contract above. Builds the
    // IRETQ frame by hand (one of the three unavoidable inline-asm spots,
    // Doc 2 section 1): DS/ES/FS/GS are loaded with the user data selector
    // *before* IRETQ, because immediately after it CPL becomes 3, and any
    // instruction that then used the still-loaded kernel (DPL 0) selector
    // would immediately #GP (a CPL-3 access to a DPL-0 segment is not
    // allowed). The stack push order is the reverse of what IRETQ pops
    // (SS, RSP, RFLAGS, CS, RIP), since `push` grows the stack downward.
    unsafe {
        core::arch::asm!(
            "mov ds, {ds:x}",
            "mov es, {ds:x}",
            "mov fs, {ds:x}",
            "mov gs, {ds:x}",
            "push {ds}",
            "push {stack_top}",
            "push {rflags}",
            "push {cs}",
            "push {entry}",
            "iretq",
            ds = in(reg) ds,
            cs = in(reg) cs,
            stack_top = in(reg) stack_top.as_u64(),
            entry = in(reg) entry.as_u64(),
            rflags = in(reg) 0x202u64,
            options(noreturn),
        );
    }
}
