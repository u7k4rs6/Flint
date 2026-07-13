//! A tiny, hand-written ring-3 program -- Flint has no ELF loader yet
//! (that is M8 stretch), so "a first user program" means literal machine
//! code the kernel places directly into a freshly mapped, isolated user
//! page and jumps to. Written as position-independent assembly (only a
//! local, `rip`-relative-addressed data label and a local backward jump,
//! nothing depending on where the kernel happened to load it) specifically
//! so it survives being copied byte-for-byte to a different virtual
//! address than where it was compiled.

use core::arch::naked_asm;

/// A byte-for-byte copy of the address range starting at `user_entry`, of
/// at least this many bytes, is guaranteed to contain the whole program
/// (its few instructions plus the embedded message are far smaller than
/// this; the rest is harmless because the program's own infinite loop
/// ensures it never executes into the copied tail).
pub const COPY_LEN: usize = 256;

/// The message the program's first (valid) syscall writes. Kept in sync
/// with the length constant embedded in the asm below.
pub const MESSAGE_LEN: u64 = 9;

/// Doc 3's required negative case: this exact address (Flint's kernel heap
/// base, `memory::heap::HEAP_START`) is always mapped by the time this
/// program runs, but never marked user-accessible -- a deterministic,
/// guaranteed "present but not yours" pointer, rather than a guess at
/// whatever the bootloader happened to leave unmapped.
pub const HOSTILE_PTR: u64 = 0x_4444_4444_0000;

#[unsafe(naked)]
pub unsafe extern "C" fn user_entry() {
    naked_asm!(
        // SYS_WRITE(1) with a valid pointer into this program's own
        // (user-accessible) code page: proves a syscall crosses the ring
        // 3 -> ring 0 boundary and its effect is observable.
        "lea rdi, [rip + 2f]",
        "mov rsi, 9",
        "mov rax, 1",
        "int 0x80",
        // SYS_WRITE(1) with a hostile pointer (mapped kernel memory, not
        // user-accessible): the kernel must reject this and keep running.
        "mov rdi, 0x444444440000",
        "mov rsi, 8",
        "mov rax, 1",
        "int 0x80",
        // SYS_EXIT(2): ends the demo.
        "mov rax, 2",
        "int 0x80",
        "1:",
        "jmp 1b",
        "2:",
        ".ascii \"hi ring3!\"",
    )
}
