# Flint: Technical Architecture

**Doc 2 of 4**
**Status:** Draft v1

This doc carries the systems weight. It specifies the boot chain, the CPU setup, interrupt handling, the memory manager, the scheduler, and the syscall path, and it states the complexity and tradeoff of every core operation inline (section 11), because for a kernel the interesting question is always why a mechanism was chosen, not only what it does.

Language is Rust in `no_std`, with inline assembly for the three places it is unavoidable: the boot stub, the context switch, and the syscall entry.

---

## 1. Guiding principles

- **One address space you built.** Everything runs in the kernel's world until user mode exists. Discipline is enforced by module boundaries and by types, not by a process boundary.
- **Serial first.** The very first milestone is output, because without it every later bug is invisible. Nothing else is debuggable until this works.
- **Safety at the lowest level.** Use Rust's ownership and its type system to make whole classes of kernel bugs unrepresentable. Confine `unsafe` to small, audited blocks (port I/O, page-table writes, the context switch) with a comment stating the invariant each one relies on.
- **Complexity is documented.** Every operation in section 11 lists its cost, the alternative, and the tradeoff.

## 2. Boot chain

Firmware to Rust, in stages:

```
firmware (BIOS/UEFI) -> bootloader -> long mode + stack -> Rust _start -> kmain
```

- **Bootloader choice.** Three reasonable paths: the `bootloader` crate (least friction, sets up long mode and paging and hands you a memory map), a Multiboot2 kernel loaded by GRUB (classic, more manual), or Limine (modern, capable). Recommendation: start with the `bootloader` crate to reach Rust and a memory map fast, and treat the boot stub as a component you can swap later.
- **Memory-map handoff.** The bootloader passes the physical memory map (which regions are usable RAM). The kernel records the usable frames from this; the frame allocator (section 5) is built on it.
- **Assembly here is unavoidable:** the earliest instructions before Rust can run.

## 3. Output subsystem

- **Serial (COM1), the primary channel.** Port I/O to the UART. This is the boot log, panics, and the shell's I/O. QEMU forwards it to the terminal and to the test harness. Build this first.
- **VGA text buffer / framebuffer, secondary.** Write characters to the emulated screen. Useful for a visible banner and panic, but serial is the workhorse.
- **A `println`-style macro** over both, with log levels, is the first quality-of-life tool you build.

## 4. CPU setup

- **GDT.** Kernel code and data segments (ring 0) and user code and data segments (ring 3). In long mode segmentation is mostly flat, but the segments and their privilege levels are what the ring transition and syscall path rely on.
- **TSS.** Holds `rsp0`, the kernel stack the CPU switches to on a privilege transition, and the IST stacks. Wiring `rsp0` correctly is what makes ring 3 to ring 0 transitions land on a valid kernel stack.
- **IDT.** Maps each interrupt and exception vector to a handler. Set up right after output so faults become visible instead of silent reboots.

## 5. Memory management (the heart)

Three layers, each with its complexity stated in section 11.

### 5.1 Physical frame allocator
Manages physical page frames drawn from the bootloader's memory map.
- **Options:** a bitmap (one bit per frame; simple, O(frames) to scan for a free one unless you track a hint), or a free-list of frames (O(1) alloc and free, at the cost of storing the list). A common start is a bump or free-list allocator over the usable regions.
- **Tradeoff:** bitmap is compact and easy to reason about but has scan cost; a free-list is O(1) but needs storage and careful init.

### 5.2 Paging and virtual memory
x86-64 uses **4-level paging** (PML4, PDPT, PD, PT), 512 entries per level, 4 KiB pages (2 MiB and 1 GiB huge pages available).
- **Accessing page tables.** The tables are themselves physical frames, so the kernel needs a way to read and write them from virtual space: either a recursive mapping, or (simpler with the `bootloader` crate) a complete physical-memory offset mapping. Pick one and state it.
- **Per-process address spaces.** Each process has its own PML4 (its own CR3 value). The kernel is mapped into every address space (the high half) so it is reachable during syscalls and interrupts.
- **Page-fault handling.** The page-fault handler (exception 14) reads the faulting address and the error code, then decides: map a fresh frame for a valid-but-unmapped address (demand paging and lazily grown stacks), or report an illegal access. A fault from user mode is handled differently from one in kernel mode.
- **Tradeoff:** demand paging saves memory and enables lazy stacks, at the cost of a fault on first touch.

### 5.3 Kernel heap allocator
Once paging works, map a heap region and install a global allocator so `alloc` (Box, Vec, BTreeMap) works in the kernel.
- **Options and tradeoffs:**
  - **Linked-list (free-list) allocator:** simple, general, but alloc and free walk the list, and it fragments.
  - **Fixed-size-block / slab allocator:** O(1) alloc and free for common sizes by bucketing into size classes, with less fragmentation for those sizes, at the cost of some internal waste and a fallback path for large or odd sizes.
  - **Buddy allocator:** power-of-two blocks with O(log n) split and merge and low external fragmentation, more complex to implement.
- **Recommendation:** a fixed-size-block allocator backed by a linked-list fallback is a strong, teachable middle ground, and it is where your existing allocator work transfers directly.

## 6. Tasks and scheduling

### 6.1 Task model
A task (kernel thread first, process later) has a control block holding its saved register state, its kernel stack, and its address-space root (CR3). Tasks live in a run queue.

### 6.2 Context switch
The switch is hand-written assembly: save the outgoing task's callee-saved registers and stack pointer, load the incoming task's, and (for a process switch) load its CR3 to change the address space. This is the single most error-prone routine in the kernel; a wrong offset corrupts state silently.
- **Cost:** O(1), a fixed sequence of register moves plus, for an address-space switch, a CR3 write that flushes the TLB (a real but bounded cost).

### 6.3 Scheduler
A preemptive round-robin scheduler driven by the timer interrupt: on each tick, save the current task, pick the next runnable one, and switch. Round-robin first, priorities as an extension.
- **Cost:** O(1) to pick the next task in a ring; O(number of tasks) only if you scan for priority without a ready structure.
- **Tradeoff:** round-robin is fair and trivial; priorities improve responsiveness but need a ready queue per level and raise starvation questions.

## 7. User mode and syscalls

### 7.1 Ring 3
Entering user mode means loading user segments and returning (via `iretq` or `sysret`) into ring 3 code in a user address space, with `rsp0` in the TSS pointing at the kernel stack to use on the next entry.

### 7.2 Syscall mechanism
- **Options:** the `syscall` and `sysret` instructions (fast, configured via the STAR, LSTAR, and SFMASK MSRs), or `int 0x80` (a software interrupt through the IDT; simpler to stand up first). Recommendation: start with the interrupt path if it is faster to get working, move to `syscall` and `sysret` for the real version.
- **The trust boundary (see Doc 3).** The kernel must never trust a pointer or a length from user mode. Every user pointer is validated (in range, mapped, user-accessible) before it is dereferenced. This is the single most important safety control in the kernel and the most common place kernels are exploited.

### 7.3 Process model and ELF loading (loading is stretch)
A process is a task with a user address space. Loading a program means creating that address space, mapping its segments, placing a user stack, and entering ring 3 at its entry point. A flat binary works first; an ELF loader is the stretch that runs real compiled programs.

## 8. The shell

The shell is a user-space process, not a kernel feature. It reads input over serial through a syscall, echoes it, parses a line, and dispatches a small command set (help, echo, a stats or meminfo command, a task list). It is the proof that user mode, syscalls, and scheduling all work together, and it folds the shell gap from the OS list into the kernel.

## 9. Stretch: filesystem and disk

- **Ramdisk first.** An in-memory filesystem (an initrd blob) behind a small VFS interface, so the shell can list and read files without a disk driver.
- **Block-backed filesystem.** A simple on-disk format (a superblock, inodes, a block allocator, directories) behind the same VFS.
- **Disk driver.** ATA PIO (simplest) or virtio-blk (cleaner under QEMU).

## 10. Module structure

```
flint/
  src/
    boot/         # early entry, long-mode handoff (asm + Rust)
    serial, vga/  # output
    gdt, idt/     # CPU tables, TSS
    interrupts/   # exception + IRQ handlers (timer, keyboard)
    mm/
      frame/      # physical frame allocator
      paging/     # page tables, map/unmap, fault handling
      heap/       # kernel heap allocator (global allocator)
    task/         # task model + context switch (asm)
    sched/        # the scheduler
    syscall/      # syscall entry, dispatch, user-pointer validation
    user/         # process model, (stretch) ELF loader
    fs/           # (stretch) VFS + ramdisk + block FS
  tests/          # boot-and-assert integration tests
```

Rust `no_std`, with `alloc` enabled only after the heap allocator (mm/heap) is installed. Confine `unsafe` to boot, port I/O, page-table writes, the context switch, and the syscall entry.

## 11. Complexity and tradeoffs (core operations)

The section that answers why, not just what.

| Operation | Baseline | Better option | The tradeoff |
|---|---|---|---|
| Frame alloc | O(frames) scan (bitmap) | O(1) with a free-list or a scan hint | storage for the list vs scan time |
| Frame free | O(1) | same | bitmap wastes little; free-list needs a node |
| Page map / unmap | O(levels) = O(1), 4-level walk, allocating tables as needed | huge pages reduce entries for large ranges | fewer entries vs coarser granularity |
| Page fault | O(1) handler + a frame alloc | prefaulting avoids the fault | fault-on-first-touch vs eager mapping cost |
| Heap alloc / free | O(n) list walk (linked-list) | O(1) for size classes (fixed-block), O(log n) split/merge (buddy) | speed and fragmentation vs implementation complexity |
| Context switch | O(1) register save/restore; +TLB flush on CR3 change | tagged TLB (PCID) avoids full flush | complexity vs switch cost |
| Scheduler pick | O(1) round-robin ring | O(1) with per-priority ready queues | fairness and simplicity vs responsiveness |
| User-pointer check | O(1) range + O(levels) mapped check | cache the validation for a copy span | safety cost per syscall vs a small check |

## 12. Testing architecture

A kernel is tested by booting it and observing behavior, so the harness is part of the design.

- **QEMU `isa-debug-exit`.** The kernel writes a status code to a debug port to exit QEMU, so a test run has a pass or fail exit code.
- **Serial capture.** The harness runs QEMU with serial redirected to stdio and asserts on the printed output.
- **In-kernel tests.** Unit tests run inside the kernel (a `no_std` test runner), report each result over serial, and exit via the debug port. This is what makes `cargo test` meaningful for a kernel.
- **gdb stub.** For the hard bugs (a triple fault, a bad context switch), run QEMU with its gdb stub and step through, since printf may not survive the failure.

Every acceptance criterion in the PRD maps to one of these: a boot assertion, a fault-handling assertion, a preemption count, a user-mode round trip, or a rejected bad pointer.

## 13. Recommended stack

- **Language:** Rust, `no_std`, with inline assembly for the boot stub, the context switch, and the syscall entry.
- **Helper crates (optional, to avoid re-deriving boilerplate):** the `bootloader` crate for boot and the memory map, and the `x86_64` crate for CPU structures (page tables, the IDT, ports). Going fully from scratch is a valid choice if you want more of the surface for its own sake.
- **Target:** QEMU (x86-64), with `isa-debug-exit` and serial for the harness, and the gdb stub for debugging.
- **Build:** a custom target spec, `cargo` with a runner that launches QEMU, so `cargo run` boots the kernel and `cargo test` runs the harness.
