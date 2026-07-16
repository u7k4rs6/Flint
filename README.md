# Flint

A small x86-64 kernel, written in Rust, that boots under QEMU: a physical
and virtual memory manager, a kernel heap, a preemptive scheduler, a
syscall boundary with per-process address spaces, and a user-space shell.
See `SUMMARY.md`, `PROGRESS.md`, and `DECISIONS.md` for the full build
record.

## Prerequisites

- The pinned `nightly` Rust toolchain (`rust-toolchain.toml` fetches it
  automatically via `rustup`), with the `rust-src` and `llvm-tools`
  components.
- `qemu-system-x86_64` (`apt-get install -y qemu-system-x86` on Debian/
  Ubuntu).
- `cargo install bootimage`.

## Commands

Four commands, and nothing else is required to interact with the project.

- **Build:** `cargo build`
- **Run:** `cargo run` -- boots straight into the interactive shell over
  serial (try `help`). Runs until you type `exit`, after which the kernel
  halts; Ctrl-C to stop earlier.
- **Test:** `cargo test` -- boots the in-kernel test harness under QEMU;
  each test reports over serial and the run exits with a pass/fail status.
  Run a single integration test with `cargo test --test <name>` (`basic_boot`,
  `stack_overflow`, `task_stack_overflow`, `null_page`, `register_dump`,
  `user_mode`, `shell`).
- **Debug:** `make debug` -- builds and launches QEMU halted with its gdb
  stub (`-s -S`) and interrupt logging, for the finicky bugs (the context
  switch, the ring-3/syscall boundary). Attach with
  `gdb -ex "target remote :1234"` in another shell once QEMU prints its
  startup banner and waits.
