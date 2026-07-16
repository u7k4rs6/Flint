<div align="center">

# Flint

**A small x86-64 kernel, written in Rust, that boots to its own shell under QEMU.**

`no_std` · nightly rust · x86_64 · bootimage · serial console

<img src="assets/boot.svg" width="880" alt="Animated boot log: power applied, POST, then GDT, IDT, long mode, paging, kernel heap, scheduler and ring 3 each reporting OK, ending at a login prompt. A flame grows from a dot to a steady block as the kernel comes up.">

</div>

Flint starts at the reset vector and stops at a prompt. In between: a physical
and virtual memory manager, a kernel heap, a preemptive scheduler, a syscall
boundary with per-process address spaces, and a user-space shell.

Nothing here is a wrapper. The page tables are built by hand, the context
switch is inline assembly, and the shell is a real ring 3 process that has to
ask the kernel for everything it wants.

Colour means privilege everywhere below: **grey is hardware**, **ember is ring
0**, **violet is ring 3**.

## Four commands

Prerequisites: the pinned `nightly` toolchain (`rust-toolchain.toml` fetches it
through `rustup`) with the `rust-src` and `llvm-tools` components,
`qemu-system-x86_64` (`apt-get install -y qemu-system-x86` on Debian and
Ubuntu), and `cargo install bootimage`.

| | | |
|---|---|---|
| **build** | `cargo build` | Compiles the kernel for the custom bare-metal target. |
| **run** | `cargo run` | Boots straight into the shell over serial. Try `help`. Runs until you type `exit`, then the kernel halts. Ctrl-C to stop earlier. |
| **test** | `cargo test` | Boots the in-kernel test harness under QEMU. Each test reports over serial and the run exits with a pass or fail status. One at a time with `cargo test --test <name>`. |
| **debug** | `make debug` | Builds and launches QEMU halted on its gdb stub (`-s -S`) with interrupt logging, for the finicky bugs. Attach with `gdb -ex "target remote :1234"` once QEMU prints its banner and waits. |

## What is on the die

<img src="assets/chip.svg" width="880" alt="Diagram of Flint's subsystems laid out as blocks on a chip package: boot, GDT and TSS, IDT, long mode, frame allocator, paging, kernel heap, syscall gate, scheduler, context switch, shell and UART. A highlight sweeps across and lights each block in turn.">

Twelve blocks, in roughly the order they come alive. Everything left of the
syscall gate exists so that the two blocks on the right can be told *no*.

## Memory

Every user address is a lie the CPU tells four times before it means anything.
Flint walks all four levels itself, and keeps one address space per process.

<img src="assets/pagewalk.svg" width="880" alt="Animated four-level page walk. A virtual address is split into its index fields, then PML4, PDPT, PD and PT light up in sequence with a pulse travelling between them, ending at a physical frame.">

Underneath the walk is a frame allocator handing out 4 KiB pages: the kernel
image and page tables first, then the heap, then user pages that come back when
a process exits.

<img src="assets/heap.svg" width="880" alt="Animated grid of physical page frames filling up in batches: kernel image, page tables, kernel heap, then user pages, with ten user frames returning to the free pool when the process exits.">

## Scheduling

A timer interrupt at 100 Hz, a round-robin queue, and a context switch that
swaps `rsp` and `cr3`. No process is asked to be polite.

<img src="assets/scheduler.svg" width="880" alt="Animated scheduler view. A CPU panel shows the current process, its stack pointer, page table root and privilege ring, while a run queue rotates the running slot between idle, init, shell and ticker. A timer interrupt fires between each quantum.">

## Interrupts

<img src="assets/interrupt.svg" width="880" alt="Animated interrupt path. A pulse travels from a keypress through the IDT vector, the keyboard ISR and the scheduler, and ends at the ring 3 shell, which echoes a character.">

The shell blocks on a read, a scancode arrives, the IDT hands the CPU to an
ISR, the scheduler unblocks pid 2, and a character appears. Five stops, and the
shell never learns it was ever asleep.

## The shell

A ring 3 process, pid 2, talking to the kernel through the syscall gate and to
you through the UART.

<img src="assets/shell.svg" width="880" alt="Animated terminal session. Commands help, ps, meminfo and exit are typed one character at a time, each printing its output, ending with the kernel halted.">

## Tests

`cargo test` boots each of these under QEMU and reports over serial.

| test | asserts |
|---|---|
| `basic_boot` | The kernel reaches its entry point and the console works. |
| `stack_overflow` | A guard-page hit on the boot stack becomes a double fault the kernel survives. |
| `task_stack_overflow` | The same, for a scheduled task's own guarded stack. |
| `null_page` | Dereferencing a null pointer faults instead of reading zeroes. |
| `register_dump` | A kernel-mode page fault's panic report carries the real register state, not zeroes. |
| `user_mode` | A process enters ring 3 and returns through the syscall gate. |
| `shell` | The shell answers commands over serial. |

## Reading the repo

| | |
|---|---|
| `SUMMARY.md` | What Flint is and how the pieces fit. |
| `PROGRESS.md` | The build record, in order. |
| `DECISIONS.md` | Why each fork in the road went the way it did. |
| `COMPLEXITY.md` | Cost, alternative, and tradeoff for every core operation. |

## About these animations

Every diagram above is one SVG file with CSS keyframes inside it. No
JavaScript, no GIFs, no external requests, nothing that GitHub strips out of a
README. They are generated, not drawn: edit `assets/gen.py` and run it.

```
python3 assets/gen.py
```

They also honour `prefers-reduced-motion`, in which case each one freezes on
its last frame instead of looping.
