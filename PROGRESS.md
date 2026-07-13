# Flint: progress

## Current milestone
M1 complete and green. Test harness complete and green. Starting M2 (interrupts).

## What boots
- `cargo run` boots to long mode via the `bootloader` 0.9 crate, reaches `kmain` in `src/main.rs`, and prints a banner over COM1 serial (`Flint kernel booting...` / `Flint vX.Y.Z -- boot OK`).
- Panic handler prints `KERNEL PANIC: <info>` over serial and halts (`hlt_loop`), for the non-test build.

## Harness status
- `cargo test --lib` boots a dedicated test kernel (`src/lib.rs` `test_kernel_main`), runs `#[test_case]` functions, reports each over serial as `<name>... [ok]`, and exits QEMU via `isa-debug-exit` with the configured success code (33). Green: `flint::trivial_assertion`.
- `cargo test --test basic_boot` (integration test, `tests/basic_boot.rs`) boots its own kernel image and asserts the kernel reaches `test_main` and can print. Green: `basic_boot::test_boots_and_prints`.
- Bare `cargo test` runs both plus doctests (0, since `no_std`) in one invocation, all green.
- QEMU is run headless (`-display none -serial stdio`) so serial is what the harness (and a human) actually observes.

## Next
- M2: GDT + TSS + IDT, exception handlers, double-fault IST stack (with a deliberate stack-overflow test), PIC remap, timer interrupt, PS/2 keyboard.

## Commands
- Build: `cargo build`
- Boot interactively: `cargo run` (headless QEMU, serial to stdout; runs forever, Ctrl-C to stop -- there is no shutdown syscall yet)
- Run the harness: `cargo test`
- gdb-stub debugging: `qemu-system-x86_64 -drive format=raw,file=target/x86_64-flint/debug/bootimage-flint.bin -serial stdio -display none -s -S` then `gdb -ex "target remote :1234"` in another shell
