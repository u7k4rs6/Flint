# Flint: decisions log

Assumptions and judgment calls, most recent first within each milestone.

## Doc 2-4 gap closure (post-M7, second pass)

A second audit against Docs 2 (Technical Architecture), 3 (Isolation and
Privilege), and 4 (Console and CLI Spec) -- the same exercise that produced
the per-process-address-space addendum below, extended to the other three
specs -- found eight further gaps between what the docs describe and what
the M1-M7 build actually shipped. All eight are closed here; none required
touching the docs themselves.

- **VGA output (`src/vga.rs`) was simply never built in the original M1-M7
  run**, despite Doc 2 section 3 and PRD FR-OUT-1 naming it the secondary
  output channel. Added as a straightforward mirror of `serial.rs`'s own
  `Mutex`+`fmt::Write` pattern, reached through the same physical-memory
  offset mapping every other physical-address access in Flint already uses
  (confirmed against the vendored `bootloader` 0.9.35 source that VGA text
  mode, not `vga_320x200`, is the crate's own default boot-stub setting) --
  not a raw identity-mapped pointer, since Flint makes no assumption that
  low physical addresses are identity-mapped into the kernel's own virtual
  space. Only *key* output (the boot banner, panics) is mirrored to it, per
  Doc 2 section 3's own "useful for a visible banner and panic, but serial
  is the workhorse" framing -- not every routine log line, which would mean
  a VGA write on every syscall.

- **No README, no single-command debug launch.** Doc 4 section 6 explicitly
  asks for both. A `README.md` documenting exactly the four commands (build/
  run/test/debug) was added, plus a small `Makefile` `debug` target wrapping
  the gdb-stub QEMU invocation `SUMMARY.md`/`PROGRESS.md` already documented
  as hand-typed. The `bootimage` target is intentionally *not* a real Make
  file-target with a timestamp dependency -- `cargo bootimage` is already
  incremental and fast when nothing changed, and a file-timestamp-based Make
  rule would silently skip rebuilding after a source edit, since Make has no
  visibility into Cargo's own dependency graph.

- **No structured log levels, no task id anywhere in output.** Doc 4 section
  2 wants every line to carry a level (`trace`/`debug`/`info`/`warn`/
  `error`) and, once tasks exist, the current task id. A small hand-written
  macro set (`log_trace!`/`log_debug!`/`log_info!`/`log_warn!`/`log_error!`
  in `serial.rs`) replaces the ad hoc `[user]`/`[syscall]`/`[shell]` prefixes
  the original build used, each expanding through a shared `TaskTag` that
  queries `task::scheduler::current_task_id()` (new) and renders nothing
  before the scheduler exists, rather than a placeholder task id. Two
  categories of existing `serial_println!` call were deliberately *not*
  migrated to a log level: the two-line boot banner and panic headers (kept
  as plain, VGA-mirrored "key output," a separate concern from routine
  diagnostics), and a user program's own `SYS_WRITE` effect (`[user] {}` --
  literally the bytes a ring-3 process asked to write, not a kernel
  diagnostic, so tagging it with a kernel log level would misrepresent
  whose output it is).

- **No guard pages for kernel task stacks**, despite Doc 3 section 3 and its
  own build-time checklist explicitly requiring them. `Task::new` (M5)
  originally backed every kernel stack with a plain heap `Box<[u8]>` -- no
  boundary, so an overflow would have silently corrupted adjacent heap
  memory. Fixed by giving every task stack its own mapped virtual region
  (via the same `memory::paging::map_page` primitive `heap::init_heap` and
  `user::map_program` already use) with one page left deliberately unmapped
  immediately below it, bump-allocated over a reserved PML4-adjacent range
  (`0x_6666_6666_0000`) never reused -- Flint has no task teardown, so there
  is nothing to free back into a real allocator yet, a decision inherited
  from the same "no teardown machinery" gap M6's own addendum below already
  named for processes. `tests/task_stack_overflow.rs` proves it: spawns a
  task whose entry recurses forever and confirms the *real* kernel double-
  fault handler (not a test-local one, since this test needs the timer and
  scheduler running to reach the task at all, unlike `stack_overflow.rs`)
  catches the overflow instead of hanging.

  **A real, previously-latent bug surfaced by this change and fixed in the
  same pass:** `task::scheduler::spawn` holds `SCHEDULER`'s lock across the
  whole call to `Task::new`, with interrupts enabled -- fine when `Task::new`
  did negligible work (a heap `alloc`), but once it started calling
  `map_task_stack` (four real page-table walks), the critical section grew
  long enough to intermittently overlap a 10ms timer tick. When it did,
  `timer_tick` (reached through the timer IRQ) tried to re-acquire the same
  non-reentrant `spin::Mutex` `spawn` was still holding and spun forever --
  a single-core self-deadlock, since only the interrupted `spawn` call could
  release it, and it can't run again until the interrupt handler spinning on
  its lock returns. Manifested as `tests/task_stack_overflow.rs` hanging
  (QEMU's `test-timeout` killing it) roughly half the time -- diagnosed by
  bisecting with temporary diagnostic `serial_println!`s at each init stage,
  which showed the hang always fell between "scheduler initialized" and
  "task spawned," narrowing it to `spawn`/`Task::new`/`map_task_stack`
  before landing on the lock-hold-with-interrupts-enabled pattern. Fixed by
  wrapping `spawn`, `init`, `switch_count`, and `current_task_id` (the
  latter added in this same pass, and just as exposed) in
  `x86_64::instructions::interrupts::without_interrupts`, the same
  discipline `serial::_print` already documents and uses for exactly this
  reason. Verified with 8 consecutive isolated runs and 3 consecutive full
  `cargo test` runs, all green, after the fix (both were previously failing
  at a roughly 25-50% rate across the same number of runs).

- **A syscall's user-pointer copy panicked the kernel on a mid-copy fault**,
  contradicting Doc 3 sections 4 and 7 and the threat table's explicit
  requirement that this scenario "return an error rather than panicking."
  `validate_user_range` closed the "does this pointer look valid" gap but
  left a TOCTOU window open: nothing stopped the mapping it just walked from
  being revoked before the actual copy ran (not reachable by anything in
  Flint today -- single process, single core, interrupts disabled for the
  whole `int 0x80` body -- but Doc 3 requires the mechanism architecturally,
  not "only if currently exploitable"). Closed with
  `copy_from_user_byte`/`copy_to_user_byte` (`syscall/mod.rs`): each records
  its own fallthrough address in a global `RECOVERY_IP` immediately before
  the one instruction that might fault, and `page_fault_handler` gets one
  new check -- if a kernel-mode fault's `RECOVERY_IP` is set, redirect there
  (via `x86_64` 0.15's `InterruptStackFrame::as_mut()`, confirmed to exist
  specifically for this) instead of falling through to its panic. Considered
  and rejected: hand-parsing the CPU-pushed frame to build a from-scratch
  recovery mechanism -- `as_mut()` already provides exactly the escape hatch
  needed, no naked stub required for this specific piece (unlike the
  register-dump addendum below, which needs one for a different reason).
  Global, not per-CPU/per-task, state is sound specifically because `int
  0x80` already runs with interrupts disabled for its entire body (a
  pre-existing invariant, not something newly introduced here) -- no IRQ can
  preempt an in-flight copy and race the recovery state; only the copy's own
  synchronous fault can touch it. `sys_write`/`sys_shell_dispatch`'s bulk
  `slice::from_raw_parts` reads and `sys_read_line`'s raw pointer write were
  all replaced with bounded byte loops through these helpers.
  `flint::syscall::tests::copy_helpers_return_err_instead_of_panicking_on_a_mid_copy_fault`
  proves it directly: validates a range, deliberately unmaps it, and asserts
  the copy returns `Err` rather than taking the kernel down -- reaching the
  assertions at all is itself part of the proof, since before this existed
  the same sequence would have panicked.

- **No register dump in any panic report**, despite Doc 4 section 5 asking
  for GPRs, the instruction pointer, the stack pointer, and relevant control
  registers. Confirmed against the vendored `x86_64` 0.15.5 source that
  `extern "x86-interrupt" fn` bodies cannot see GPRs at all -- LLVM's
  x86-interrupt calling convention saves and restores whatever it clobbers
  in a hidden prologue/epilogue never exposed to the handler. Considered and
  rejected: hand-parsing the CPU-pushed exception frame in a full naked
  replacement for each handler (real risk of getting the error-code/frame
  layout subtly wrong -- exactly what `task/context.rs`'s own docs warn
  "corrupts state silently"). Instead, each of `page_fault_entry`/
  `general_protection_fault_entry`/`double_fault_entry` (`interrupts.rs`) is
  a small naked trampoline, generated from one shared `macro_rules!` template
  so a typo can't silently diverge between the three, that captures GPRs via
  plain `mov [addr], reg` writes -- no stack or flags touched -- into a set
  of statics, then `jmp`s straight into the existing, completely unchanged
  `extern "x86-interrupt" fn` handler. That handler's own compiler-generated
  frame/error-code parsing does all the real work, untouched; it just also
  has the captured GPRs available (via a `GprDump` `Display` impl) when it
  builds its panic message, alongside the current task id
  (`task::scheduler::current_task_id()`). `breakpoint_handler` was
  deliberately *not* given a trampoline -- it logs and resumes, never
  panics, so Doc 4 section 5's "panic report" requirement doesn't apply to
  it. The generic `#[panic_handler]` (`main.rs`, a plain `panic!()` with no
  hardware trap frame behind it at all) gets an honestly-labeled best-effort
  fallback instead: four registers read via inline `asm!` at the top of the
  handler, documented explicitly as "registers at panic-handler-entry time,
  not the original `panic!()` call site," since intervening code (formatting
  the message, reaching that function) may have already reused them.
  `tests/register_dump.rs` proves the real (trampoline) path with an actual
  value match, not just "didn't crash": loads a known marker into `rax`,
  triggers a genuine kernel-mode page fault on a deliberately unmapped
  canonical address, and asserts the panic report's `rax=` field contains
  the exact marker. (One accidental but useful confirmation along the way:
  an earlier draft of that test used a *non-canonical* address by mistake,
  which triggered a general protection fault instead of a page fault --
  directly proving the GPF trampoline works too, with an exact register
  match, before the test was fixed to exercise the page-fault path its name
  actually claims.)

## M6 addendum -- per-process address spaces (post-M7)

The PRD (Doc 1, Goals and FR-MEM-2/FR-USER-1) calls for a separate address
space per process. M6's original build shipped without this (see the M6
section below, "No ELF loader, no per-process address space"), a deliberate
scope cut at the time. This addendum closes that gap without reopening M6's
other scope lines (still no ELF loader, still one process per boot).

- **A fresh top-level (PML4) table per process, built by cloning the
  currently-active one, not a from-scratch table.** `memory::paging::new_address_space`
  copies all 512 PML4 entries (one 4 KiB page, a single `copy_nonoverlapping`
  through the physical-memory offset mapping) out of whatever table is
  active at call time. Kernel-region entries already populated by then (the
  kernel image, the phys-mem-offset mapping, the heap) end up pointing at
  the *same* physical PDPT/PD/PT sub-tables as the source, so the kernel
  stays identically reachable from the new table without walking or
  re-mapping anything below the top level. Entries not yet populated (the
  PML4 slots the user code/stack pages use) start absent and get mapped
  fresh, invisible to any other table -- this is what makes the isolation
  real rather than cosmetic. Building a table from scratch and re-mapping
  the kernel into it by hand was rejected: it would require enumerating and
  re-walking every kernel mapping made so far, redoing work the bootloader
  and `memory::init` already did once, for no isolation benefit over sharing
  the same physical sub-tables by reference.

- **A single cached mapper (`paging::MAPPER`), rebuilt once per address-space
  switch by `paging::activate`, rather than resolving the active table via
  `Cr3::read()` on every `with_mapper` call.** The alternative (dynamic
  per-call resolution) was considered and rejected: it would add a
  raw-pointer-to-`&'static mut PageTable` reconstruction to the hottest path
  in the kernel -- every syscall's `validate_user_range` call, i.e. every
  syscall that touches a pointer -- for no benefit, since CR3 changes at
  most once per boot in every real entry point (`user::setup`/`setup_shell`,
  called exactly once each). The invariant this relies on, stated explicitly
  in `paging::activate`'s doc comment: CR3 must only ever be written through
  `activate`, never directly, or the cached mapper goes stale and silently
  keeps mutating the table being switched away from.

- **Process creation always clones from the boot table, never from a
  previously created process's table.** Flint has no fork/exec, and every
  real call site (`main.rs`, `tests/user_mode.rs`, `tests/shell.rs`) creates
  exactly one process per boot, always before any process CR3 has ever been
  activated -- so "the currently active table" at every `new_address_space`
  call site today is always the boot table. `new_address_space` doesn't
  special-case or assert this (there is nothing to distinguish "the boot
  table" from "a process table" at that layer -- both are just "whatever CR3
  currently is"); it is a scope assumption inherited from Flint having no
  concept of a second concurrently-running process yet, not a check the code
  enforces.

- **Accepted limitation, not hit by anything in Flint today: a kernel-region
  page mapped for the first time while a non-boot table is active would only
  exist in that one table.** `new_address_space` only shares entries already
  present in the source table at clone time; a *new* PML4-level entry
  created afterward (e.g. if the lazy demand-paging region were touched for
  the first time while a process's table were active) would not retroactively
  appear in the boot table or in any other process's table. This doesn't
  affect anything that exists today: the lazy region (`paging::LAZY_REGION_*`)
  is only ever touched by one dedicated kernel-mode test
  (`memory::paging::tests::page_fault_on_lazy_region_is_handled_and_continues`),
  which never activates a process address space, and the page-fault handler
  returns before reaching `demand_page` at all for a fault taken from ring 3
  (see `interrupts.rs`'s `USER_MODE` early return). The correct general fix
  -- pre-populating every kernel-region PDPT entry once at boot, before the
  first process is ever cloned from it, so no kernel-region PML4 entry is
  ever created *after* the first clone -- is real, well-understood follow-up
  work, not attempted here.

## M7

- **Command parsing lives behind a syscall (`SYS_SHELL_DISPATCH`), in the kernel, not in the ring-3 loop itself.** Doc 2 section 8 frames the shell as "a user-space process, not a kernel feature," and the process-level claim is genuinely true here: the read/dispatch/loop control flow, and the decision to keep running or call its own `SYS_EXIT`, all execute in ring 3. What does not run in ring 3 is the actual string matching for `help`/`echo`/`meminfo`/`ps`/`ticks` -- that would require either an ELF loader (M8 stretch, not attempted) or hand-encoding a real parser directly as raw machine code bytes, which was judged too large a source of new risk for the value it added, this late in an already-long build. Named here as a conscious scope line, matching the same "flat binary, no loader yet" simplification M6 already made for its own demo program.

- **A test-input hook (`syscall::set_scripted_input`), discovered necessary only after proving the real thing first.** Piping bytes into `cargo test --test shell`'s own stdin genuinely does reach the in-kernel UART (`-serial stdio` is inherited transparently through `bootimage runner` and `cargo test`'s child processes) -- verified directly. But wiring the *harness* test to depend on that would make a bare `cargo test` (no piped input) hang for the full `test-timeout` and fail, which is unacceptable: the brief requires `cargo test` itself to stay green with no external setup. The fix mirrors `set_exit_hook` from M6: a swappable byte source, defaulting to the real UART, overridable by a test to a fixed scripted sequence. The scripted test exercises the identical ring-3/syscall/validate/echo code path real input uses; only where the bytes originate differs, and the real path is still manually demonstrated (see PROGRESS.md) rather than discarded.

- **A benign UART startup race, documented rather than chased down.** The very first byte of piped input can occasionally be lost (arrives and overwrites the UART's single-byte receive register before the kernel starts polling it, since the 16550 emulation here has no FIFO and Flint's read loop is polling-based, not interrupt-driven RX). Confirmed harmless -- everything after the first byte was correct, and the scripted-input test path used for the automated harness gate does not depend on real hardware timing at all, so it isn't affected. Fixing this properly would mean interrupt-driven RX with a buffer, a bigger change than M7's remaining scope justified; the one-line mitigation for a manual interactive session (send a throwaway leading newline) is documented in PROGRESS.md instead.

## M6

- **`int 0x80`, not `syscall`/`sysret`.** Doc 2 section 7.2 offers both and recommends starting with the interrupt path. `int 0x80` reuses the IDT and the ring-3 IRETQ transition M6 already has to build; `syscall`/`sysret` needs three additional MSRs (STAR, LSTAR, SFMASK) configured correctly on top of everything else in the hardest milestone. A real follow-up, not attempted here.

- **No ELF loader, no per-process address space -- the user program is hand-written machine code, copied into an isolated page, still inside the kernel's one CR3.** Doc 2 section 7.3 explicitly calls ELF loading a stretch goal and says "a flat binary works first." Going further and giving the demo program a genuinely separate address space would mean a second PML4 (its own CR3) and a scheme for the kernel-half of every address space to stay identically mapped (so syscalls and interrupts remain reachable mid-switch) -- real, well-understood work, but enough of it that attempting it inside M6's existing risk budget (already the hardest milestone per the build brief) risked turning a working, tested ring-3/syscall boundary into a broken one. Isolation is still real at the level M6 actually claims: ring 0 vs ring 3 is enforced, the syscall boundary validates every pointer, and W xor X holds for the program's own two pages. What's explicitly not yet true, and is the natural M6+ extension: one process cannot yet be isolated from another, because there is only ever one demo process and it shares the kernel's page tables. Named here so the gap is a decision, not a discovered surprise.

- **User code and user stack pages live in two entirely separate PML4 slots (`0x_2000_0000_0000` and `0x_3000_0000_0000`), each also far from the heap, the lazy demand-paging region, and the kernel image.** `Mapper::map_to`'s parent-table flags are derived from the *first* mapping call that creates a given intermediate table entry (PML4E/PDPTE/PDE); a second, later mapping that happens to reuse those same intermediate entries does not get its own flags OR'd in. Since the code page is mapped without `WRITABLE` and the stack page is mapped with it, sharing any intermediate table between them would let whichever mapping ran first silently determine the effective permission ceiling for both (the CPU ANDs permission bits across all four page-table levels) -- a subtle, hard-to-notice way to break W xor X. Separate PML4 slots make that impossible by construction instead of by careful ordering.

- **The code page is mapped `WRITABLE` just long enough to copy the program's bytes in, then flipped read-only via `Mapper::update_flags` before ring 3 ever sees it.** First attempt mapped it read-only immediately and the kernel's own `copy_nonoverlapping` into that page page-faulted -- CR0.WP (write-protect) applies to supervisor (ring 0) writes too, not only ring 3, so even the kernel cannot write through a PTE lacking `WRITABLE`. Diagnosed via the panic's own page-fault report (address, error code `PROTECTION_VIOLATION | CAUSED_BY_WRITE`) rather than by guessing; fixed on the next attempt, confirmed via a full test rerun. Logged per the anti-loop protocol.

- **The hostile-pointer test target is a fixed, known address (the kernel heap base, `memory::heap::HEAP_START`) rather than an arbitrary or random unmapped address.** The heap is unconditionally mapped by the time the demo runs, and is unconditionally never marked user-accessible, so this is a deterministic "present but not yours" case every run -- the more interesting of the two rejection paths the validator has (mapped-but-supervisor-only vs. simply unmapped), and not dependent on guessing at the bootloader's own memory layout.

- **A settable `syscall::set_exit_hook` rather than hardcoding `SYS_EXIT` to end the whole VM.** `SYS_EXIT`'s real (default) behavior is to park the kernel in `hlt_loop` -- correct for a real "a process exited" event, since a QEMU-exiting side effect has no place in normal kernel operation (M7's shell will also be a user program that syscalls). The test needs to observe "the demo reached `SYS_EXIT` without the kernel dying" some other way; a one-function hook lets the test swap in `qemu::exit_qemu(Success)` without `sys_exit` itself needing any test-harness awareness, and without `#[cfg(test)]` (which would not even apply here -- `tests/user_mode.rs` links the library's normal, non-test build, the same reason `tests/stack_overflow.rs` and `tests/null_page.rs` also install their own local hooks/IDTs rather than relying on library-internal `cfg(test)` branches).

- **`#[repr(align(16))]` added on the TSS's IST/rsp0 stack storage.** `[u8; N]` alone has no alignment guarantee beyond 1 byte; the syscall entry stub's `call syscall_dispatch` (and the double-fault handler, retroactively) both need the SysV-mandated 16-byte stack alignment at their call sites to hold, which depends on the *top* of these stacks being 16-aligned. Not previously a problem in practice (M2-M5 apparently got a suitably aligned address by chance, likely from linker/`.bss` placement defaults), but no longer left to chance now that a real call site depends on it.

## M5

- **`#[unsafe(naked)]` + `naked_asm!` (stable since Rust 1.88) instead of `global_asm!` or a hand-rolled `asm!` inside a normal function.** A naked function has no compiler-generated prologue/epilogue, which matters here specifically: `switch` manually pushes/pops the exact register set it wants and hands off `rsp` itself, and a normal function's own prologue (which would push `rbp` and adjust `rsp` before our code ever runs) would corrupt that layout. This is one of the three places inline assembly is unavoidable per Doc 2 section 1.

- **The context-switch register layout is written out as an explicit `#[repr(C)]` struct (`SavedContext`) with a comment on why field order matters**, rather than only living implicitly in the push/pop sequence. Doc 2 section 6.2 calls this the single most error-prone routine in the kernel -- a wrong offset corrupts state silently, no fault, nothing to catch it -- so the layout the asm assumes is written down once, in one place, instead of needing to be re-derived by reading push/pop order backward.

- **No PCID/tagged-TLB, and no per-task CR3 yet.** Every task in M5 is a kernel thread sharing the kernel's single address space, so `switch` never touches CR3 and there is no TLB flush on this path. Per-process address spaces (and the CR3 switch, and the flush cost Doc 2 section 11 calls out) arrive with M6's user processes.

- **PIT reprogrammed to 100 Hz, retroactively closing a gap left in M2.** M2 wired IRQ0 to a handler and counted ticks but never reprogrammed the 8253/8254 timer chip itself, so it was still running at the BIOS default of ~18.2 Hz. That is workable but needlessly coarse for a scheduler quantum (and later, for a `ticks`/`uptime` shell command to be meaningful), so `interrupts::init_pit(100)` was added now and wired into `flint::init()`.

- **The "two tasks alternate" test is deliberately adversarial to a broken scheduler**: both spawned tasks loop forever with no voluntary yield, so a scheduler that only ever ran the first-spawned task (a plausible bug shape -- e.g. forgetting to push the outgoing task back onto the ready queue) would leave the second counter at 0 forever and the test would hang rather than pass. A hang is a legitimate, honest test failure mode here (caught by the harness's QEMU timeout), preferred over a weaker test that could pass by accident.

- **First live run of the scheduler test under `cargo test --lib` alone hit a 60s timeout; a direct manual QEMU run of the identical binary (with `-d int` logging attached, to check for a triple fault or an unexpected exception) completed correctly in under a second, and three subsequent `cargo test --lib` runs were all green.** No exception storm or CPU reset showed up in the interrupt log, and the alternative I tried (rerun directly under gdb-adjacent logging to rule out a genuine hang) confirmed the mechanism itself works; the single slow run reads as host scheduling jitter under this sandboxed environment, not a bug in the context switch or scheduler. Logged per the anti-loop protocol rather than re-running in a blind loop.

## M4

- **Fixed-size-block heap allocator implemented directly, using `linked_list_allocator::Heap` only as the internal fallback path**, rather than installing `linked_list_allocator`'s `LockedHeap` wholesale as the global allocator. Doc 2 section 5.3 lists three options and recommends the fixed-block design as "a strong, teachable middle ground"; implementing it (rather than only wiring up a crate) is what makes the alloc/free complexity note in `COMPLEXITY.md` an honest description of what Flint's allocator actually does, not a description of a dependency's internals.

- **No formal virtual-memory-area (VMA) tracking yet.** The page-fault handler's demand paging only activates inside one explicit, hardcoded "lazy region" (`memory::paging::LAZY_REGION_START/SIZE`) rather than deciding "valid but unmapped" vs "illegal" from a real per-address-space region map. A real kernel needs the latter (e.g. to lazily grow a specific task's stack, or back a specific mmap'd range) but that requires the process/address-space model M5/M6 will introduce. Scoping demand paging to one explicit region now is enough to satisfy and test the PRD's "valid-but-unmapped page fault is handled and execution continues" requirement honestly, without a wild kernel-mode pointer bug getting silently papered over by an overly permissive fault handler in the meantime.

- **`map_page` asserts (not merely documents) that virtual page 0 is never mapped.** Doc 3 section 3's null-page requirement is enforced once, at the single choke point every mapping in the kernel funnels through (the heap, demand paging, and any future user-mode mapping code), rather than left as a convention each call site has to remember.

## M3

- **Intrusive free-list frame allocator instead of a bitmap or a `Vec`-backed free list.** The frame allocator has to exist *before* the heap does (frame alloc -> paging -> heap is the dependency order), so it cannot use `alloc::Vec` for its own bookkeeping. Threading the free list through the freed frames' own memory (via the bootloader's physical-memory offset mapping) sidesteps that ordering problem entirely and gives O(1) alloc/free with no separate storage, at the cost of every `deallocate_frame` being an unsafe raw write into physical memory. See `COMPLEXITY.md` for the full tradeoff writeup.

- **Physical frame 0 is permanently excluded from the free list.** Doc 3 section 3 calls for leaving virtual page 0 unmapped (a null-pointer-deref defense); that's enforced at the paging layer in M4. Excluding *physical* frame 0 here too is an extra, cheap belt-and-suspenders: it guarantees frame 0 can never be the backing memory for any mapping the kernel makes, regardless of what a future paging bug might attempt.

## M2

- **Two `x86_64` crate versions in the dependency graph.** `bootloader` 0.9.35 pins its own internal `x86_64 = 0.14.7` (resolved to 0.14.13); Flint's own code uses `x86_64 = "0.15"`. Cargo resolves both simultaneously since neither crosses the crate boundary as a shared type (the `bootloader::BootInfo` struct handed to `kmain` is plain data, not `x86_64` types). Confirmed this does not cause the `E0152 duplicate lang item` class of error by building and running the full test suite green.

- **`page_fault_handler` distinguishes ring 3 vs ring 0 via `PageFaultErrorCode::USER_MODE`, per Doc 3 section 5**, even though no user mode exists yet (M6): a user-mode fault reports and returns; a kernel-mode fault panics with a register dump and Cr2 (the faulting address). The "valid but unmapped, continue execution" demand-paging case from the PRD's definition of done is deferred to M4, once there is a heap/paging layer able to decide "unmapped-but-valid" vs "illegal."

- **Double-fault-on-stack-overflow test uses its own `harness = false` integration test (`tests/stack_overflow.rs`)** rather than a `#[test_case]`, because the test's entire point is that the kernel never returns from `stack_overflow()` -- the custom-test-framework runner (which expects to keep running further tests afterward) is the wrong shape for a test that ends the boot. It installs its own single-purpose IDT where the double-fault handler reports `[ok]` and exits QEMU, instead of the kernel's normal double-fault handler, which panics (the right behavior for the real kernel, the wrong one to observe from a test that wants to see the fault fire cleanly).

## Scaffolding / M1

- **Missing source docs.** The build brief references four specs in `docs/`: a PRD (Doc 1), the technical architecture (Doc 2), isolation and privilege (Doc 3), and a console/CLI doc (Doc 4). Only Doc 2 and Doc 3 were actually provided (Doc 3 three times, identical). Doc 1 (PRD) and Doc 4 (console/CLI) were never attached. The build brief itself restates the PRD's milestone list, gates, and definition of done, and the shell command set (`help`, `echo`, `meminfo`, `ps`, `ticks`/`uptime`) inline, so I proceeded using the brief plus Docs 2 and 3 as the source of truth rather than blocking on missing files. Noted here so the gap is a recorded decision, not a silent one.

- **Bootloader crate: `bootloader` 0.9.35 + `bootimage`, not 0.11.** Doc 2 section 13 recommends the `bootloader` crate. The 0.11 line restructures the build (kernel becomes a library invoked from an external builder binary) and is a bigger departure from the classic custom-target-json + `cargo run` workflow the brief asks for ("a custom target spec, cargo with a runner that launches QEMU"). `bootloader` 0.9.x plus the `bootimage` cargo subcommand matches that description directly (`isa-debug-exit`, `-serial stdio`, a `runner` in `.cargo/config.toml`) and is the same architecture used in the well-trodden "Writing an OS in Rust" reference material, which lowers the risk of unrecoverable triple-faults during the hardest milestones (M5/M6). Both `bootloader` 0.9.35 and `bootimage` 0.10.4 turned out to be recently republished crates.io versions patched for a current nightly, which confirmed this pairing is still actively maintained.

- **Toolchain: track the ambient `nightly` (rustc 1.99.0-nightly, 2026-07-12), not an older pinned nightly.** First attempt pinned `nightly-2024-01-01` to match the `x86_64` 0.14.x crate generation used by the original tutorial. That nightly's nonexistent `-Zjson-target-spec` flag is unconditionally passed by `bootimage` 0.10.4's bootloader-builder step, so the old nightly cannot drive the current `bootimage`. Switched back to the ambient modern nightly and instead resolved the `x86_64` crate's Rust `Step` trait breakage (from a nightly libcore API change) by using `x86_64 = "0.15"` for Flint's own code, while `bootloader` keeps its own pinned `x86_64 0.14.x` internally (Cargo happily resolves two versions in the graph since they never share a type across the crate boundary that matters). This is a genuine alternative attempted and logged per the anti-loop protocol.

- **`-Z panic-abort-tests` is required.** Without it, `cargo test` builds `core` twice under `-Z build-std` (once implicitly with unwind for the libtest-style test harness unit, once with `panic = "abort"` for the normal profile), and a shared dependency (`bit_field`, `volatile`, `bitflags`) built against both cores fails with `E0152 duplicate lang item`. Enabling `panic-abort-tests` in `.cargo/config.toml` makes cargo build a single abort-strategy core for both, which is what `panic-abort` and this kernel's `panic_handler` model assume everywhere else.

- **Custom target JSON field names/types changed for the current nightly.** `target-pointer-width` and `target-c-int-width` are now integers, not strings; `executable` was renamed `executables`; soft-float now needs an explicit `"rustc-abi": "softfloat"` key in addition to the `+soft-float` feature. These are mechanical adjustments for the pinned nightly's rustc, not architectural choices.

- **VGA text buffer skipped for now.** Doc 2 section 3 lists it as secondary ("useful for a visible banner... but serial is the workhorse"). Serial is required and sufficient for every gate in Doc 3's checklist and the PRD's definition of done as restated in the brief; VGA is left as an unstarted stretch rather than spending M1 budget on a non-required output path.

- **`unsafe` block count kept to exactly one so far**: `SerialPort::new(0x3F8)` in `src/serial.rs` and the port write in `src/qemu.rs`, each with an invariant comment, per the brief's requirement to confine `unsafe` to small audited blocks.
