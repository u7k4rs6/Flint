# Flint: progress

## Current milestone
M1, M2, and M3 complete and green. Starting M4 (paging + kernel heap).

## What boots
- `cargo run` boots to long mode via the `bootloader` 0.9 crate, reaches `kmain` in `src/main.rs`, and prints a banner over COM1 serial (`Flint kernel booting...` / `Flint vX.Y.Z -- boot OK`).
- `flint::init()` loads the GDT/TSS (with the double-fault IST stack), loads the IDT, remaps and unmasks the PIC, and enables interrupts. The timer (IRQ0) increments a tick counter; the keyboard (IRQ1) decodes scancodes and echoes to serial.
- Panic handler prints `KERNEL PANIC: <info>` over serial and halts (`hlt_loop`), for the non-test build.
- `flint::init_memory(boot_info)` walks the bootloader's memory map and builds the physical frame allocator (`src/memory/frame.rs`), an intrusive free list threaded through the free frames' own backing memory. Physical frame 0 is never handed out.

## Harness status
- `cargo test --lib`: `flint::trivial_assertion`, `flint::interrupts::tests::test_breakpoint_exception`, `flint::memory::tests::allocated_frames_are_distinct_and_nonzero`, `flint::memory::tests::freed_frame_is_reused` (allocates a frame, frees it, allocates again, asserts the LIFO free list hands back the same frame and the free count returns to where it started).
- `cargo test --test basic_boot`: `basic_boot::test_boots_and_prints` -- boots and prints.
- `cargo test --test stack_overflow` (harness = false, single deliberate test): recurses until the kernel stack overflows; asserts the double fault is caught by the handler running on its own IST stack (prints `[ok]` and exits via `isa-debug-exit`) instead of QEMU silently resetting (a triple fault). This is the Doc 3 section 5/7 acceptance gate.
- Bare `cargo test` runs all of the above plus doctests (0, since `no_std`) in one invocation, all green.
- QEMU is run headless (`-display none -serial stdio`) so serial is what the harness (and a human) actually observes.

## Next
- M4: page tables (map/unmap), page-fault handling (demand paging for valid-but-unmapped addresses), the kernel heap allocator (enables `alloc`).

## Commands
- Build: `cargo build`
- Boot interactively: `cargo run` (headless QEMU, serial to stdout; runs forever, Ctrl-C to stop -- there is no shutdown syscall yet)
- Run the harness: `cargo test`
- gdb-stub debugging: `qemu-system-x86_64 -drive format=raw,file=target/x86_64-flint/debug/bootimage-flint.bin -serial stdio -display none -s -S` then `gdb -ex "target remote :1234"` in another shell
