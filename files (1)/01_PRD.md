# Flint: Product Requirements Document

> Working codename: **Flint**, a bootable x86-64 kernel that runs under QEMU.
> Rename freely. This name is only a consistent handle across the four docs.

**Doc 1 of 4** (PRD, Technical Architecture, Isolation & Privilege, Console & CLI)
**Status:** Draft v1
**Owner:** Utkarsh

---

## 1. Summary

Flint is a small operating-system kernel for x86-64 that boots under QEMU and runs user programs. It is the OS-internals flagship: a single artifact that folds a physical and virtual memory manager, a kernel heap allocator, a preemptive scheduler, a syscall boundary, and a user-space shell into one codebase. The point is not to reimplement Linux. The point is to show that the machine is understood from the boot vector up, and to do it in a memory-safe language so the kernel itself makes a reliability statement.

This is a portfolio and interview artifact first. A reader should be able to run one command, watch it boot over serial, type into a shell backed by real user-mode processes, and read a codebase where the memory manager, the scheduler, and the syscall path are clean and deliberate.

## 2. Motivation

Two motivations, one artifact.

1. **The gap:** the OS-internals blank names an allocator, a scheduler, a shell, a toy kernel, and virtual memory. A kernel is the one project that delivers all of them at once, in a single coherent system rather than five disconnected exercises.
2. **The signal:** "I understand computers from the metal up" is the loudest systems signal there is, and very few candidates have shipped a kernel that boots and preemptively multitasks user processes. Doing it in Rust adds a second signal about memory safety at the lowest level.

## 3. Goals and non-goals

### Goals
- Boot from firmware to 64-bit long mode and into Rust.
- Text output over the serial port (primary) and the VGA text buffer (secondary).
- Interrupt handling: CPU exceptions, a timer, and a keyboard.
- A physical frame allocator over the memory map.
- Paging with per-process address spaces and page-fault handling.
- A kernel heap allocator, enabling `alloc` (Box, Vec) in the kernel.
- A preemptive, timer-driven scheduler over kernel tasks.
- A syscall interface crossing from ring 3 to ring 0.
- User-mode processes, and a shell running as one of them, driven over serial.
- A boot-and-assert test harness (QEMU debug-exit plus serial capture).
- A documented complexity and tradeoff note for every core operation (frame alloc and free, page map and unmap, heap alloc and free, context switch, scheduler pick).

### Non-goals (v1)
- Not Linux, and no POSIX compatibility.
- No SMP (single core in v1; multicore is stretch).
- No networking, no GUI, no window system.
- A single architecture (x86-64 only).
- No security hardening beyond the isolation boundary in Doc 3 (no KPTI, no Spectre mitigations, no ASLR in v1).

## 4. Who and what interacts with it

Instead of product personas, a kernel has three parties.

- **The developer:** builds the kernel, boots it under QEMU, and debugs it over serial and through a gdb stub. This is the primary human, and Doc 4 is written for them.
- **User programs:** run in ring 3 and reach the kernel only through the syscall boundary. The shell is the first of these.
- **Hardware and QEMU:** deliver asynchronous events (timer ticks, keyboard input) as interrupts, and expose the serial port and a debug-exit device.

## 5. Scope

### Tier 1: boot, output, interrupts, memory (the foundation)
- Boot to long mode, a stack, and Rust entry, with the memory map handed off from the bootloader.
- Serial and VGA output, and a panic path that prints a fault.
- The GDT (kernel and user segments), a TSS, and the IDT.
- Exception handlers (the page-fault handler is the interesting one), a remapped PIC or the APIC, a timer, and a PS/2 keyboard.
- A physical frame allocator.
- Paging: build and switch page tables, map the kernel, and handle page faults.
- A kernel heap allocator, so `alloc` works.

### Tier 2: tasks and scheduling
- A task model (kernel threads first), each with saved registers, a stack, and an address-space root.
- Context switching (the assembly that saves and restores state and swaps address spaces).
- A preemptive round-robin scheduler driven by the timer.

### Tier 3: user mode, syscalls, and the shell
- Ring 3 setup and the transition into and out of user mode.
- A syscall mechanism (`syscall` and `sysret`, or `int 0x80`), with strict validation of user pointers.
- User processes, and a shell process reading keystrokes and dispatching commands over serial.

### Stretch (documented, optional)
- An ELF loader, so real compiled user programs run.
- A filesystem: a ramdisk or initrd first, then a block-backed filesystem behind a small VFS.
- A disk driver (ATA PIO or virtio-blk).
- SMP (bringing up additional cores).

## 6. Functional requirements

- FR-BOOT-1: The system reaches 64-bit long mode and executes Rust code with a valid stack.
- FR-BOOT-2: The system receives the physical memory map from the bootloader and records usable regions.
- FR-OUT-1: The kernel prints formatted text over serial, with log levels, and mirrors key output to the VGA buffer.
- FR-INT-1: The kernel installs an IDT and handles CPU exceptions, distinguishing a fault from user mode from one in kernel mode.
- FR-INT-2: A timer interrupt fires periodically, and a keyboard interrupt delivers scancodes.
- FR-MEM-1: A physical frame allocator hands out and reclaims page frames.
- FR-MEM-2: The kernel builds page tables, maps and unmaps pages, and maintains a separate address space per process.
- FR-MEM-3: A page fault on an unmapped-but-valid address is handled (for example by mapping a fresh frame), and an illegal access is reported.
- FR-MEM-4: A kernel heap allocator supports `alloc`, so Box and Vec work in the kernel.
- FR-SCHED-1: The scheduler preemptively switches between at least two tasks on timer ticks, saving and restoring full state.
- FR-SYS-1: A user-mode process invokes a syscall, the kernel validates its arguments, services it, and returns to user mode.
- FR-USER-1: The kernel loads and runs a program in ring 3 in its own address space.
- FR-SHELL-1: A shell process reads input over serial, echoes it, and dispatches a small command set.

## 7. Representative scenarios (the user-story analog)

- The developer runs one command; the kernel boots and prints a boot log over serial.
- A page fault on a lazily mapped stack page is resolved by mapping a frame, and execution continues.
- Two kernel tasks run in turn under the timer, each printing its own counter, proving preemption.
- The kernel drops to ring 3, the user program issues a write syscall, and text appears over serial from user mode.
- The developer types `help` into the shell and sees the command list; a bad pointer passed to a syscall is rejected rather than crashing the kernel.

## 8. Acceptance criteria (the gate, verified under QEMU)

Each is asserted by the test harness (Doc 4) via serial output and the debug-exit code, not by eyeballing.

- The kernel boots to Rust and prints an expected banner over serial.
- A deliberate access to an unmapped valid page is handled and execution continues past it.
- A double fault is caught on its own stack rather than triple-faulting (a stack-overflow test proves the IST path).
- Two tasks alternate under the timer a known number of times.
- A user-mode program performs a syscall and returns; the effect appears over serial.
- A syscall given an out-of-range or unmapped user pointer returns an error and the kernel stays alive.
- The shell echoes typed input and runs at least `help` and one status command.
- Every core operation carries a complexity and tradeoff note.

## 9. Milestones (build order)

1. **M1 Boot and serial.** Long mode, Rust entry, serial output, a panic handler. This is where you stop flying blind.
2. **M2 Interrupts.** GDT, TSS, IDT, exception handlers, the double-fault IST stack, PIC or APIC, timer, keyboard.
3. **M3 Physical memory.** Frame allocator over the memory map.
4. **M4 Paging and heap.** Page tables, map and unmap, page-fault handling, the kernel heap allocator.
5. **M5 Tasks and scheduling.** Task model, context switch, preemptive round-robin.
6. **M6 User mode and syscalls.** Ring 3 setup, the syscall boundary, user-pointer validation, a first user program.
7. **M7 Shell.** A user-space shell over serial with a small command set.
8. **M8 Stretch.** ELF loading, a ramdisk and filesystem, a disk driver, SMP, as time allows.

## 10. Out of scope for v1

SMP, networking, a GUI, POSIX compatibility, hardware beyond what QEMU exposes, and security hardening beyond the isolation boundary (KPTI, Spectre and Meltdown mitigations, ASLR).

## 11. Success metrics

**Product-shaped (proves it works):**
- One command boots it under QEMU.
- It preemptively multitasks and drops to user mode and back.
- The boot-and-assert harness is green.

**Portfolio-shaped (proves the point of building it):**
- One repository contains the allocator, scheduler, virtual memory, and shell the OS gap named.
- The architecture doc names the mechanisms (4-level paging, the chosen heap allocator, the context switch, the syscall path) and states the complexity and tradeoff of each core operation.
- The kernel is written in a memory-safe language, and the isolation boundary in Doc 3 is real and tested.
