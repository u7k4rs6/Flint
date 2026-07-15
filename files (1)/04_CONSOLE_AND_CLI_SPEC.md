# Flint: Console and CLI Spec

**Doc 4 of 4** (the frontend slot, reframed for a kernel)
**Status:** Draft v1

A kernel has no web UI. Its interface is the serial console: what the kernel prints, the shell you type into, the panic screen when it dies, and the developer-facing commands to build, run, and debug it. This doc specifies all four, in the same spirit as a frontend spec (screens, states, interactions), mapped onto a kernel.

---

## 1. Principles

- **The serial console is the interface.** Boot log, shell, panics, and debug output all flow over serial. It is the terminal, so its legibility is the whole user experience.
- **Every output has a level and a shape.** Consistent, parseable formatting matters because the test harness reads this output and asserts on it. The console is both a human UI and a machine interface.
- **The panic screen is the error state.** When the kernel faults, what it prints is the difference between a five-minute fix and an hour of blind guessing.
- **Boot has states, like a UI has loading and error states**, and the scariest one is the pre-serial window where nothing is visible yet.

## 2. The serial console

- **Primary output channel** over COM1, forwarded by QEMU to the terminal and to the harness.
- **Log format:** each line carries a level (`trace`, `debug`, `info`, `warn`, `error`, `panic`) and, once tasks exist, the current task id, so interleaved output is attributable. Keep the format stable and greppable.
- **The VGA text buffer is secondary:** a visible banner on the emulated screen and a mirrored panic, but serial is where the real work happens.

## 3. Boot output states

Like a frontend's loading, empty, and error states, the boot sequence has distinct phases the developer learns to read.

- **Pre-serial (the blind window).** From firmware to the moment serial is initialized, nothing prints. A crash here shows only a QEMU reset. The mitigation is to initialize serial as the first thing possible and, if needed, fall back to the gdb stub and QEMU's interrupt log.
- **Serial up (log flowing).** Once serial works, the boot log streams: memory map recognized, GDT and IDT installed, paging on, heap ready, scheduler started. Each milestone prints a line, so a hang is localized to the last line printed.
- **Interactive (shell prompt).** After user mode and the shell start, a prompt appears and the console is interactive.
- **Panic (fault dump).** On an unrecoverable fault, the panic screen (section 5) takes over.

## 4. The shell (interactive console)

The shell is a user-space process (Doc 2, section 8) reading over serial through syscalls.

- **Prompt and input.** A prompt (for example `flint> `), line editing at least for backspace and enter, and an echo of typed characters.
- **Command set (v1):**
  - `help` lists commands.
  - `echo <text>` prints its argument (proves argument passing across the syscall boundary).
  - `meminfo` prints memory stats (frames used and free, heap usage), proving the memory manager is queryable.
  - `ps` lists the current tasks and their states, proving the scheduler is introspectable.
  - `uptime` or `ticks` prints the timer count.
  - (stretch, with a filesystem) `ls` and `cat`.
- **Errors:** an unknown command prints a short, consistent error and re-prompts, never hangs.
- **Behavior under load:** the shell is one task among several, so a background task printing its counter should interleave with the prompt without breaking input.

## 5. The panic and fault screen (the error state)

When the kernel cannot continue, it prints a fault report and halts (or exits QEMU via the debug port under test). The report should contain:

- The reason (the panic message or the exception name and vector).
- The faulting address and the error code, for a page fault.
- A register dump (the general-purpose registers, the instruction pointer, the stack pointer, and the relevant control registers).
- The current task id, once tasks exist.
- A backtrace if it can be produced safely.

A kernel fault in ring 0 panics loudly like this. A fault caused by a user process (Doc 3) does not panic the kernel; it reports and terminates or signals that process, and the shell survives.

## 6. The developer CLI (build, run, debug)

This is the command surface a developer uses to interact with the project, the closest analog to a frontend's interaction flows.

- **Build and run:** `cargo run` builds the kernel for the custom target and launches QEMU with serial on stdio and the `isa-debug-exit` device attached, so the kernel boots into your terminal.
- **Test:** `cargo test` boots the kernel test runner under QEMU; each in-kernel test reports over serial and the run exits with a pass or fail code from the debug port.
- **Debug:** a make or cargo alias launches QEMU with its gdb stub and halted at start (`-s -S`), plus interrupt logging (`-d int`), so the developer attaches gdb and steps through the hard failures (a triple fault, a bad context switch, a wrong page mapping).
- **Conventions:** QEMU flags live in the runner config, not scattered, so build, run, test, and debug are single commands. The README documents these four commands and nothing else is required to interact with the project.

## 7. Output and interaction conventions

- **Loading analog:** every boot milestone prints one line, so progress and hangs are both visible.
- **Empty analog:** the shell at rest shows a prompt, not a blank screen.
- **Error analog:** unknown shell commands and rejected syscalls print a short, consistent message and continue; kernel faults print the full panic report.
- **Denied analog:** a user process that oversteps (a bad pointer, a privileged instruction) is reported and contained, and the console stays usable.

## 8. Accessibility and ergonomics (developer-facing)

- Output is plain ASCII over serial, so it works in any terminal and in CI logs.
- The log format is stable and greppable, because the test harness depends on it.
- One command each to build, run, test, and debug, documented in the README, so the project is approachable to a reader who has never built a kernel.
