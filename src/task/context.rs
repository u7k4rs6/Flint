//! The context switch: the single most error-prone routine in the kernel
//! (Doc 2 section 6.2) -- a wrong offset here corrupts state silently
//! rather than faulting cleanly, which is why it is one of the three places
//! inline assembly is unavoidable (Doc 2 section 1) and why its layout is
//! written out explicitly below rather than left implicit.
//!
//! Complexity and tradeoff: O(1) -- a fixed sequence of register
//! save/restores, no loop, no allocation. Every kernel thread in Flint
//! shares the kernel's single address space (no per-task CR3 yet; that
//! waits for M6's user processes), so there is no TLB flush on this path
//! either -- that cost only appears once switching also changes CR3.

use core::arch::naked_asm;
use core::mem::size_of;

/// The layout `switch`'s asm pushes onto the outgoing stack and pops off
/// the incoming one. Field order matters: `#[repr(C)]` places fields at
/// increasing addresses in declaration order, and the stack grows down, so
/// this must read top-to-bottom exactly as `switch` pushes bottom-to-top
/// (last pushed = lowest address = first field here).
#[repr(C)]
struct SavedContext {
    rflags: u64,
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    rbx: u64,
    rbp: u64,
    /// Not a saved register: this is the address `switch`'s trailing `ret`
    /// resumes at. For a fresh task it is the task's entry point; for a
    /// previously-running task it is the return address inside `switch`
    /// itself, pushed there implicitly by the `call` that reaches this
    /// function the *next* time this task is switched out.
    resume_address: u64,
}

/// Builds the initial saved-context frame for a brand new task, so that the
/// first `switch` into it lands on `entry` exactly as if a normal task had
/// been switched away from and back to.
///
/// # Safety
/// `[stack_top - size_of::<SavedContext>(), stack_top)` must be valid,
/// writable, exclusively-owned memory (the top of a freshly allocated task
/// stack), and `stack_top` must already be 16-byte aligned per the x86-64
/// SysV ABI `switch` relies on for anything the entry function itself
/// calls.
pub unsafe fn init_stack(stack_top: u64, entry: extern "C" fn() -> !) -> u64 {
    let ctx_addr = stack_top - size_of::<SavedContext>() as u64;
    let ctx = ctx_addr as *mut SavedContext;
    // SAFETY: forwarded to this function's contract above; `ctx` is
    // 8-byte aligned because `stack_top` is 16-byte aligned and
    // `size_of::<SavedContext>()` is a multiple of 8.
    unsafe {
        (*ctx).rflags = 0x202; // IF set (bit 9) plus the always-set bit 1
        (*ctx).r15 = 0;
        (*ctx).r14 = 0;
        (*ctx).r13 = 0;
        (*ctx).r12 = 0;
        (*ctx).rbx = 0;
        (*ctx).rbp = 0;
        (*ctx).resume_address = entry as usize as u64;
    }
    ctx_addr
}

/// Saves the outgoing task's callee-saved registers and stack pointer to
/// `*old_rsp`, then loads `new_rsp` and resumes whatever was saved there.
/// Does not return to its caller in the usual sense: control returns here
/// only the *next* time this same task is switched back in, at which point
/// it falls through to `ret` as if this call had simply returned.
///
/// # Safety
/// `old_rsp` must be a valid, exclusively-owned `*mut u64` (the calling
/// task's own saved-rsp slot). `new_rsp` must be either a stack pointer
/// this function previously saved via some other task's `old_rsp`, or the
/// result of `init_stack` for a task that has never run -- any other value
/// reads garbage as a `SavedContext` and corrupts control flow. The caller
/// must not be holding any lock that the newly-resumed task (or anything
/// scheduled before this task runs again) would need.
#[unsafe(naked)]
pub unsafe extern "C" fn switch(old_rsp: *mut u64, new_rsp: u64) {
    naked_asm!(
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "pushfq",
        "mov [rdi], rsp",
        "mov rsp, rsi",
        "popfq",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",
    )
}
