# Flint: progress

## Current milestone
M1 through M5 complete and green. Starting M6 (user mode + syscalls).

## What boots
- `cargo run` boots to long mode via the `bootloader` 0.9 crate, reaches `kmain` in `src/main.rs`, and prints a banner over COM1 serial (`Flint kernel booting...` / `Flint vX.Y.Z -- boot OK`).
- `flint::init()` loads the GDT/TSS (with the double-fault IST stack), loads the IDT, remaps and unmasks the PIC, and enables interrupts. The timer (IRQ0) increments a tick counter; the keyboard (IRQ1) decodes scancodes and echoes to serial.
- Panic handler prints `KERNEL PANIC: <info>` over serial and halts (`hlt_loop`), for the non-test build.
- `flint::init_memory(boot_info)` walks the bootloader's memory map and builds the physical frame allocator (`src/memory/frame.rs`), an intrusive free list threaded through the free frames' own backing memory. Physical frame 0 is never handed out.
- The same call now also brings up the page table mapper (`src/memory/paging.rs`, an `OffsetPageTable` over the bootloader's physical-memory mapping) and the kernel heap (`src/memory/heap.rs`, a fixed-size-block allocator with a linked-list fallback, installed as `#[global_allocator]`), so `alloc` (`Box`, `Vec`, `BTreeMap`) works from here on. Virtual page 0 can never be mapped -- `map_page` asserts on it -- so a null-pointer dereference always faults.
- The kernel-mode page-fault handler now demand-pages a not-present fault inside a designated "lazy" virtual region (`memory::paging::LAZY_REGION_*`), mapping a fresh frame and letting the faulting instruction re-run; anything else (a protection violation, or a not-present fault outside that region) still panics with a register dump, per Doc 3 section 5.
- The PIT (IRQ0's source) is reprogrammed from its ~18.2 Hz BIOS default to 100 Hz (`interrupts::init_pit`), giving the scheduler a known 10ms quantum.
- `flint::init_scheduler()` starts a preemptive round-robin scheduler (`src/task/scheduler.rs`) with a placeholder task standing in for the boot thread. `task::scheduler::spawn` adds kernel threads to the ready queue; the timer handler calls `scheduler::timer_tick()` (after sending the PIC its EOI, since a switch may not return for a long time) to pop the next ready task and hand-written asm (`src/task/context.rs`, `#[unsafe(naked)]`) to actually switch stacks. All tasks currently share the kernel's one address space (no per-task CR3 until M6).

## Harness status
- `cargo test --lib`: `flint::trivial_assertion`, `flint::interrupts::tests::test_breakpoint_exception`, `flint::memory::tests::allocated_frames_are_distinct_and_nonzero`, `flint::memory::tests::freed_frame_is_reused`, `flint::memory::heap::tests::{boxed_value_round_trips, large_vec_uses_every_slot, many_boxes_dont_exhaust_the_heap, large_allocation_uses_fallback_path}`, `flint::memory::paging::tests::page_fault_on_lazy_region_is_handled_and_continues` (the PRD's "valid-but-unmapped page fault is handled and execution continues" gate: writes through an intentionally never-pre-mapped pointer, then reads the value back to prove a real frame landed there, not just that the fault was swallowed).
- `cargo test --test basic_boot`: `basic_boot::test_boots_and_prints` -- boots and prints.
- `cargo test --test stack_overflow` (harness = false): double-fault-on-stack-overflow gate (Doc 3 section 5/7).
- `cargo test --test null_page` (harness = false, new): dereferences virtual address 0 in kernel mode and asserts the fault is a genuine not-present fault on page 0 caught by a test-local handler, not a crash and not a lucky read of real memory -- the Doc 3 section 3/7 "page 0 is unmapped" gate.
- `flint::task::scheduler::tests::two_tasks_alternate_under_the_timer` (new): the PRD's "two tasks alternate under the timer a known number of times" gate. Spawns two kernel threads, each an infinite loop with no voluntary yield incrementing its own atomic counter; waits until both reach 20. Since neither task can make progress without genuine preemption, and 40+ actual context switches are asserted to have happened, this is direct proof the scheduler and the hand-written context switch both work, not just that the scheduler ran once.
- Bare `cargo test` runs all of the above plus doctests (0, since `no_std`) in one invocation, all green.
- QEMU is run headless (`-display none -serial stdio`) so serial is what the harness (and a human) actually observes.

## Next
- M6: ring 3 setup, the syscall boundary, strict user-pointer validation, a first user program.

## Commands
- Build: `cargo build`
- Boot interactively: `cargo run` (headless QEMU, serial to stdout; runs forever, Ctrl-C to stop -- there is no shutdown syscall yet)
- Run the harness: `cargo test`
- gdb-stub debugging: `qemu-system-x86_64 -drive format=raw,file=target/x86_64-flint/debug/bootimage-flint.bin -serial stdio -display none -s -S` then `gdb -ex "target remote :1234"` in another shell
