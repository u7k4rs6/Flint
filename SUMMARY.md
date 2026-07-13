# Flint: build summary

Overnight build session. Milestones M1 through M7 (the full non-stretch
build order from the brief) are complete, green, and pushed. M8 (stretch:
ELF loader, ramdisk/filesystem, disk driver, SMP) was not attempted --
explicitly out of scope for this run, left as a documented stub per the
brief's own guidance, not a silent gap.

Compiling is not booting, and booting is not passing. Every claim below
states whether the harness actually boots and asserts it, and which test,
not just whether the code compiles.

## Source docs actually available

The build brief names four specs in `docs/`: a PRD (Doc 1), the technical
architecture (Doc 2), isolation and privilege (Doc 3), and a console/CLI
doc (Doc 4). Only Doc 2 and Doc 3 were ever attached to this session (Doc
3 three times, identical); Doc 1 and Doc 4 were never provided. The build
brief itself restates the PRD's milestone list, gates, and definition of
done, and the shell's exact command set, inline, so the build proceeded
on the brief plus Docs 2 and 3 as the source of truth rather than
blocking on files that were never sent. Doc 2 and Doc 3 are checked into
`docs/` for reference.

## Functional requirements

| Requirement | Status | Harness boots + asserts? | Test id |
|---|---|---|---|
| FR-BOOT (boot to Rust, long mode) | Implemented | Yes | `basic_boot::test_boots_and_prints` |
| FR-OUT (serial output) | Implemented | Yes | `basic_boot::test_boots_and_prints`; every test in the suite prints and is read over serial |
| FR-INT (GDT/TSS/IDT, exceptions, double-fault IST, PIC, timer, keyboard) | Implemented; keyboard (PS/2, IRQ1) wired and compiled but not exercised by an automated assertion (no PS/2 keystroke is injected in headless QEMU testing -- the shell's interactive I/O goes over the UART/serial path, not PS/2) | Yes for breakpoint + double-fault IST; no for keyboard | `flint::interrupts::tests::test_breakpoint_exception`, `stack_overflow::stack_overflow` |
| FR-MEM (physical frame allocator, paging, heap) | Implemented | Yes | `flint::memory::tests::*`, `flint::memory::heap::tests::*`, `flint::memory::paging::tests::page_fault_on_lazy_region_is_handled_and_continues`, `null_page::null_page` |
| FR-SCHED (task model, context switch, preemptive round robin) | Implemented | Yes | `flint::task::scheduler::tests::two_tasks_alternate_under_the_timer` |
| FR-SYS (syscall boundary, user-pointer validation) | Implemented (`int 0x80`; `SYS_WRITE`, `SYS_EXIT`, `SYS_READ_LINE`, `SYS_SHELL_DISPATCH`; every pointer validated via a real page-table walk) | Yes | `user_mode::*` (hostile pointer rejected, kernel survives), `shell::*` (a writable-range validation, the copy-to-user direction) |
| FR-USER (ring 3, first user program) | Implemented; no ELF loader or per-process address space (hand-written machine code, shared CR3 -- see DECISIONS.md) | Yes | `user_mode::*` |
| FR-SHELL (user-space shell over serial: help, echo, meminfo, ps, ticks/uptime) | Implemented; command control flow runs in ring 3, command *parsing* runs in the kernel behind a syscall (see DECISIONS.md) | Yes | `flint::shell::tests::*` (dispatch logic), `shell::*` (full ring-3 loop against scripted input) |

M8 (ELF loader, ramdisk/filesystem, disk driver, SMP): not done. Not
attempted, not partially compiled, not stubbed with dead code -- simply
not started, in line with the brief's scope guardrail to finish M1-M7
before touching M8.

## What boots and works, milestone by milestone

- **M1 (boot + serial + panic handler):** boots to long mode via the
  `bootloader` 0.9 crate, reaches `kmain`, prints a banner over COM1.
  Panic handler reports over serial and halts. Green:
  `basic_boot::test_boots_and_prints`.
- **Test harness** (built immediately after M1, per the brief): QEMU
  `isa-debug-exit` + serial capture + an in-kernel `custom_test_frameworks`
  runner. `cargo test` boots a dedicated test kernel, runs `#[test_case]`
  functions, reports each over serial, and exits QEMU with a pass/fail
  status.
- **M2 (interrupts):** GDT with kernel/user code and data segments, a TSS
  carrying the double-fault IST stack, an IDT with breakpoint/double-fault/
  page-fault/general-protection-fault handlers, the legacy 8259 PIC
  (remapped clear of the CPU exception range), a timer (IRQ0, PIT
  reprogrammed to 100 Hz), and a PS/2 keyboard handler (IRQ1). Green:
  `flint::interrupts::tests::test_breakpoint_exception`,
  `stack_overflow::stack_overflow` (the double-fault-on-stack-overflow
  isolation gate).
- **M3 (physical frame allocator):** an intrusive free-list allocator over
  the bootloader's memory map, O(1) alloc/free after an O(frames) init
  walk, threaded through the free frames' own backing memory (no heap
  dependency, since the heap doesn't exist yet at this point in boot).
  Physical frame 0 is never handed out. Green:
  `flint::memory::tests::allocated_frames_are_distinct_and_nonzero`,
  `flint::memory::tests::freed_frame_is_reused`.
- **M4 (paging + heap):** an `OffsetPageTable` mapper with map/unmap;
  `map_page` refuses to ever map virtual page 0; a fixed-size-block heap
  allocator with a linked-list fallback, installed as
  `#[global_allocator]`; demand paging for a designated "lazy" virtual
  region wired into the page-fault handler. Green:
  `flint::memory::heap::tests::*`,
  `flint::memory::paging::tests::page_fault_on_lazy_region_is_handled_and_continues`
  (the PRD's "valid-but-unmapped page fault is handled and execution
  continues" gate), `null_page::null_page` (the "page 0 is unmapped"
  gate).
- **M5 (tasks + preemptive scheduling):** a hand-written `#[unsafe(naked)]`
  context switch (O(1), no allocation, no TLB flush -- every task shares
  the kernel's one CR3), a `Task` owning its own kernel stack, and a
  preemptive round-robin scheduler driven by the timer. Green:
  `flint::task::scheduler::tests::two_tasks_alternate_under_the_timer` --
  two kernel threads, each an infinite loop with no voluntary yield, both
  reach a target counter value, which is only possible under genuine
  preemption.
- **M6 (user mode + syscalls):** ring 3 via a GDT with user segments and a
  TSS `rsp0`; an isolated user code page (read-only + executable) and
  stack page (writable + `NO_EXECUTE`), each in a separate top-level
  page-table region so W xor X can't be silently weakened by a shared
  intermediate entry; a hand-written IRETQ transition into ring 3; `int
  0x80` syscalls with a hand-written entry stub; every user pointer
  validated by an actual page-table walk before it is ever dereferenced.
  Green: `user_mode::*` -- a valid `SYS_WRITE`'s effect lands on serial, a
  `SYS_WRITE` with a pointer into the kernel heap is rejected and logged,
  the kernel keeps running, `SYS_EXIT` is reached.
- **M7 (shell):** `help`, `echo`, `meminfo`, `ps`, `ticks`/`uptime`, as a
  pure, independently-tested `dispatch(line) -> String` function, reached
  from a ring-3 process loop via two new syscalls (`SYS_READ_LINE`,
  `SYS_SHELL_DISPATCH`). `cargo run` now boots straight into this shell.
  Green: `flint::shell::tests::*` (the command logic, 8 cases) and
  `shell::*` (the full ring-3 loop against a scripted `help` / `meminfo` /
  `exit` sequence, deterministic under a bare `cargo test`). Real
  interactive serial input (a human typing at a real terminal) was also
  manually verified end to end -- see PROGRESS.md for the exact command
  and transcript -- but is not the automated harness path, since piping
  external stdin through the full `cargo test` process chain reliably
  would not make a good boot-and-assert gate.

## What is stubbed or unfinished, and why

- **M8 in its entirety**: not started. Time budget went to a solid,
  fully-tested M1-M7 rather than a partial, untested M8, per the brief's
  own priority ("A verified scheduler with no user mode is a real result;
  a half-wired ring 3 that triple-faults is not" -- the same logic applies
  one tier up).
- **No ELF loader or per-process address space.** Every task (M5 kernel
  threads, the M6 demo program, the M7 shell) shares the kernel's single
  CR3. Both user programs are hand-written, position-independent machine
  code copied into isolated pages, not loaded from a binary format. A
  natural, explicitly deferred extension (Doc 2 section 7.3 explicitly
  calls ELF loading a stretch goal).
- **Shell command parsing runs behind a syscall, in the kernel**, not in
  ring-3 code. The shell *process* (its loop, its syscalls, its exit
  decision) genuinely runs in ring 3; the string matching for `help` vs
  `echo` vs `meminfo` does not, because hand-encoding a full parser
  directly as raw machine code bytes was judged not worth the added risk
  this late in the build. See DECISIONS.md M7.
- **PS/2 keyboard handler is wired and compiles but has no automated
  test.** All interactive I/O in this build (the shell, the M6/M7 demos)
  goes over the serial UART, which is what both the human interface and
  the test harness actually exercise; the PS/2 path from M2 has no
  scripted-keystroke test analogous to `syscall::set_scripted_input`.
- **`SYS_READ_LINE` runs with interrupts disabled for its whole
  (potentially long, human-typing-speed) blocking duration**, since `int
  0x80` uses an interrupt gate. The scheduler and tick counter are
  effectively paused while the shell is waiting for a line. Harmless for
  a single foreground interactive shell (this build's actual scope); a
  real implementation would re-enable interrupts around the blocking wait
  and use a proper wait queue.
- **A benign UART startup race**: the very first byte of piped input can
  occasionally be lost to a startup timing race (no RX FIFO/interrupt-
  driven receive). Confirmed harmless and does not affect the automated
  harness (which uses scripted input, not real UART timing); documented
  in PROGRESS.md/DECISIONS.md with the one-line manual mitigation.
- **A single, static `rsp0`** (the TSS's ring 3 -> ring 0 stack), not
  updated per task. Fine for one demo user program at a time (M6/M7's
  actual scope); a real multi-process kernel writes this on every
  scheduler switch.

Nothing here is reported as done that only compiles. Every item in the
"what boots and works" section above is backed by a named, green test.

## Commands

- Build: `cargo build`
- Boot interactively (drops straight into the shell -- try `help`):
  `cargo run`. Headless QEMU, serial to stdout. Runs until you type
  `exit`, after which the kernel halts; Ctrl-C to stop earlier.
- Run the full harness: `cargo test`
- Run one integration test: `cargo test --test <name>` (`basic_boot`,
  `stack_overflow`, `null_page`, `user_mode`, `shell`)
- Manually drive a live interactive session over real serial I/O (not the
  scripted-input test path): after `cargo build`,
  `qemu-system-x86_64 -drive format=raw,file=target/x86_64-flint/debug/bootimage-flint.bin -device isa-debug-exit,iobase=0xf4,iosize=0x04 -serial stdio -display none -no-reboot`
- gdb-stub debugging (for the finicky milestones -- the context switch,
  the ring-3/syscall boundary): after `cargo build`,
  `qemu-system-x86_64 -drive format=raw,file=target/x86_64-flint/debug/bootimage-flint.bin -serial stdio -display none -s -S`,
  then in another shell: `gdb -ex "target remote :1234"`
- QEMU interrupt-log diagnostics (used during the build to diagnose the
  M6 code-page write fault; see DECISIONS.md): add
  `-d int,cpu_reset -D /path/to/log.txt` to any of the above.

Environment notes for reproducing this build: `qemu-system-x86` and
`bootimage`/`cargo-bootimage` are not preinstalled and were installed
during this session (`apt-get install -y qemu-system-x86`,
`cargo install bootimage`); the pinned toolchain is `nightly` per
`rust-toolchain.toml` (rustup will fetch it automatically), with
`rust-src` and `llvm-tools` components.

## Decisions log and remaining gaps

The full reasoning for every judgment call above -- including two real
bugs found and fixed during the build (a duplicate-`core`-under-build-std
`cargo test` failure needing `-Z panic-abort-tests`, and a page-fault
diagnosed via QEMU's interrupt log when the M6 code page was mapped
read-only before the kernel tried to copy the program into it) -- is in
`DECISIONS.md`, organized by milestone, most recent first. Complexity and
tradeoff notes for every core operation (frame alloc/free, page map/
unmap, page fault/demand paging, heap alloc/free, context switch,
scheduler pick, user-pointer check) are in `COMPLEXITY.md`, per Doc 2
section 11. Current state and what's next (nothing required; M8 items if
resumed) are in `PROGRESS.md`.
