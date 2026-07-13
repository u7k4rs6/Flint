//! The interactive shell's ring-3 process loop: read a line via
//! `SYS_READ_LINE`, hand it to the kernel to parse and respond to via
//! `SYS_SHELL_DISPATCH`, repeat -- until the line was `exit`/`quit`, in
//! which case the dispatch syscall reports that back (`rax == 1`) and this
//! loop issues `SYS_EXIT` itself.
//!
//! Doc 2 section 8: "The shell is a user-space process, not a kernel
//! feature." What lives here, genuinely in ring 3, is the process: the
//! decision to keep looping, the control flow, the syscalls it chooses to
//! make. What lives behind `SYS_SHELL_DISPATCH` in the kernel is command
//! parsing (`shell::dispatch`) -- Flint has no ELF loader (M8 stretch), so
//! this loop is hand-written position-independent machine code, and
//! hand-encoding a full string-matching parser directly in assembly was
//! judged not worth the added risk this late in the build (see
//! DECISIONS.md).

use core::arch::naked_asm;

/// A byte-for-byte copy of this many bytes starting at `shell_entry` is
/// guaranteed to contain the whole loop; see `program::COPY_LEN` for why
/// this doesn't need to be exact.
pub const COPY_LEN: usize = 256;

/// The line-buffer address, hardcoded here and in `user::setup_shell`
/// (which maps it) -- this program has no way to receive it as a runtime
/// parameter without an ELF loader's argument-passing convention, so it
/// is a compile-time constant shared between the two, the same pattern
/// `program::HOSTILE_PTR` already uses.
pub const BUF_ADDR: u64 = 0x_3000_0000_2000;
pub const BUF_CAP: u64 = 256;

#[unsafe(naked)]
pub unsafe extern "C" fn shell_entry() {
    naked_asm!(
        "1:",
        // SYS_READ_LINE(3): rdi=buf, rsi=cap -> rax=line length
        "mov rdi, 0x300000002000",
        "mov rsi, 256",
        "mov rax, 3",
        "int 0x80",
        // SYS_SHELL_DISPATCH(4): rdi=buf, rsi=length(from the read above)
        // -> rax = 1 if the line was exit/quit, else 0
        "mov rsi, rax",
        "mov rdi, 0x300000002000",
        "mov rax, 4",
        "int 0x80",
        "cmp rax, 1",
        "je 2f",
        "jmp 1b",
        "2:",
        "mov rax, 2", // SYS_EXIT
        "int 0x80",
        "3:",
        "jmp 3b",
    )
}
