# Flint: complexity and tradeoff notes

One entry per core operation, added as each lands (Doc 2 section 11). Every
entry states the cost, the alternative considered, and the tradeoff -- not
just what the code does.

## Interrupt dispatch (M2)

- **Cost:** O(1). The IDT is a flat 256-entry table indexed directly by
  vector number; delivery is a hardware table lookup, not a search.
- **Alternative:** none really at the IDT level (the table shape is
  dictated by the architecture); the design choice is PIC vs APIC for
  hardware IRQ routing.
- **Tradeoff:** the legacy 8259 PIC (chosen for Flint) is simple, well
  documented, and enough for single-core, non-SMP v1 (in scope), at the
  cost of no per-CPU routing and coarser priority control than the APIC/
  IOAPIC path. APIC is the natural extension once SMP is in scope (out of
  scope for v1 per Doc 3 section 6.4).

## User-pointer check (M6)

- **Cost:** O(1) range/overflow check plus O(levels) = O(1) per page in
  the range (a page-table walk via `Translate`), so O(pages-in-range)
  overall -- one walk per 4 KiB page the syscall argument spans.
- **Alternative:** cache the validation result for a whole copy span
  (validate once, trust it for every subsequent access within that
  syscall), which Doc 2 section 11 lists as the option that trades some
  safety margin (a validated range could theoretically be remapped
  between the check and a later use, a classic TOCTOU) for fewer walks on
  a large multi-page copy.
- **Tradeoff:** Flint validates the whole range up front and then treats
  it as trusted only for the remainder of that single syscall invocation
  (no caching across syscalls, no long-lived "trusted" pointers) -- the
  safety cost is one walk per page, paid on every syscall that touches a
  pointer, which is deliberately conservative given this is Doc 3's
  single most important safety control.

## Context switch (M5)

See the module doc comment in `src/task/context.rs` for the full writeup;
summary: O(1) -- a fixed sequence of register saves and restores, no loop,
no allocation, no TLB flush. Kernel threads (the only tasks the scheduler
ever switches between) still share the kernel's one CR3, so `context::switch`
itself never touches it. Per-process address spaces (M6 addendum, see below)
arrived through a separate mechanism -- `paging::activate`, called once when
a ring-3 process is created, not on every scheduler pick -- because ring-3
processes are not scheduler `Task`s in Flint (see `src/user/mod.rs`'s module
doc comment). The alternative Doc 2 section 6.2 gives is "load its CR3 to
change the address space" as part of the switch itself; that remains a real,
not-yet-taken extension for a future change that schedules processes as
`Task`s, not something this addendum adds to the hot switch path.

## Address space creation / activation (M6 addendum)

- **Cost:** O(1) -- `new_address_space` copies a fixed 512 8-byte entries
  (one 4 KiB page, the whole top-level PML4) regardless of how much memory
  is mapped underneath; no recursion into lower levels, since kernel-region
  sub-tables (PDPT/PD/PT) are shared by reference (the same physical frames)
  rather than deep-copied. `activate` is one `mov cr3, reg`, which
  architecturally flushes the whole (non-PCID) TLB -- a real but bounded
  cost, same as Doc 2 section 11 and the M5 entry above anticipated, and
  incurred once per process created, not once per scheduler tick.
- **Alternative:** copy each populated `PageTableEntry` through the typed
  `PageTable` API instead of a raw `u64` `copy_nonoverlapping`.
- **Tradeoff:** the raw copy is consistent with the frame allocator's own
  style (`src/memory/frame.rs` already writes free-list links as raw `u64`s
  through the same physical-memory offset mapping) and needs no
  `PageTableEntry: Clone` support; a `debug_assert!` right after the copy
  (matching entry-populated counts between source and destination) is the
  cheap correctness check in place of leaning on a typed API. The real
  limitation isn't the copy mechanism, it's timing: only entries already
  populated in the source table at clone time are shared going forward -- a
  kernel-region page mapped for the *first* time while a different table is
  active would only exist in that one table, not retroactively in any
  earlier clone. Not hit by anything in Flint today (see DECISIONS.md), but
  the real fix -- pre-populating every kernel-region PDPT entry once at
  boot, before any process is ever cloned from it -- is future work.

## Scheduler pick (M5)

See the module doc comment in `src/task/scheduler.rs`; summary: O(1),
popping the front of a plain ring used as a FIFO. The alternative is
per-priority ready queues (O(1) pick within a level, but a structure per
level and starvation questions to answer), left for a later extension
since v1 treats every task as equally important.

## Page map / unmap (M4)

See the module doc comment in `src/memory/paging.rs` for the full writeup;
summary: O(1) (4 fixed levels) per operation regardless of how much memory
is mapped elsewhere, using 4 KiB pages throughout rather than huge pages,
trading fewer page-table entries for large ranges against the coarser
granularity huge pages would force on every mapping (including small,
short-lived ones).

## Page fault / demand paging (M4)

- **Cost:** O(1) handler dispatch plus one frame allocation (itself O(1),
  see below) for a demand-paged fault; O(1) and non-recoverable (panic)
  for anything else.
- **Alternative:** eagerly map a region's pages up front ("prefaulting").
- **Tradeoff:** demand paging (chosen, per Doc 2 section 5.2) only spends a
  physical frame -- and the O(1) map cost -- when a page is actually
  touched, at the price of a fault (and its associated trap overhead) on
  first access. Prefaulting removes that per-page fault cost but spends
  memory and map time up front even for pages that might never be
  touched. Flint additionally scopes *which* not-present faults are
  treated as demand-pageable to an explicit lazy region rather than
  treating every kernel-mode not-present fault as legitimate -- a wild
  kernel pointer outside that region still panics loudly instead of
  silently getting a frame, which is a deliberate correctness/debuggability
  tradeoff against a more general (and more dangerous) "map anything
  not-present" policy.

## Heap alloc / free (M4)

See the module doc comment in `src/memory/heap.rs` for the full writeup;
summary: O(1) for both alloc and free on the common block-size classes (an
intrusive free list per class, same technique as the frame allocator),
falling back to `linked_list_allocator`'s O(n)-in-holes first-fit search
for anything bigger than the largest class (2048 bytes). Chosen over a
plain linked-list allocator (simpler, but O(n) on every alloc and free) and
a buddy allocator (O(log n) splits/merges, lower external fragmentation,
meaningfully more implementation complexity) as the middle ground Doc 2
section 5.3 recommends.

## Frame alloc / frame free (M3)

- **Cost:** `init` is O(frames) -- every usable frame is visited once to
  link it into the free list, which cannot be avoided regardless of
  allocator shape (something has to enumerate what's free). After that,
  `allocate_frame` and `deallocate_frame` are both O(1): pop/push the head
  of an intrusive singly linked free list.
- **Alternative:** a bitmap, one bit per frame -- compact, easy to reason
  about, but a naive scan for the next free bit is O(frames) per
  allocation unless a "last freed" hint is tracked.
- **Tradeoff:** Doc 2 section 11 frames this as "storage for the list vs
  scan time." The intrusive design sidesteps that tradeoff by storing each
  free list node *inside the free frame itself* (its first 8 bytes hold
  the physical address of the next free frame, reached through the
  bootloader's physical-memory offset mapping) rather than in a
  separately allocated structure -- there is no free-list storage cost
  because the "storage" is memory that is otherwise idle while unused.
  The real cost moves elsewhere: `deallocate_frame` is `unsafe` and does a
  raw pointer write into physical memory, so an incorrect caller (freeing
  a frame that is still referenced) corrupts that frame's contents
  immediately rather than merely mis-tracking a bitmap bit.

## Double-fault IST stack (M2)

- **Cost:** O(1) -- a fixed, pre-allocated 20 KiB stack, switched to by
  hardware (not software) on IST entry via the TSS.
- **Alternative:** run the double-fault handler on the current kernel
  stack, like other exceptions.
- **Tradeoff:** the whole reason a double fault exists as its own
  exception is to catch faults that occur while handling another fault;
  the most common real trigger is a kernel stack overflow. Handling it on
  the *same* stack that just overflowed re-faults immediately, which the
  CPU escalates to an unrecoverable triple fault (silent reset). A
  dedicated IST stack costs a fixed slab of memory reserved up front, in
  exchange for turning that reset into a catchable, debuggable exception --
  verified directly by `tests/stack_overflow.rs`.

## Kernel task stack allocation (M5 addendum, post-M7)

- **Cost:** O(1) per task -- `task::map_task_stack` bump-allocates the next
  virtual-address slot (a counter increment, no search) and maps a fixed
  `STACK_SIZE / 4 KiB` = 4 pages, each an O(1) page-table walk (see "Page
  map / unmap" above). One guard page is *not* mapped, which costs nothing
  -- an absence, not an operation.
- **Alternative:** the original M5 design, a plain heap `Box<[u8]>`
  (`alloc::vec![0u8; STACK_SIZE].into_boxed_slice()`) -- O(1) via the
  existing fixed-size-block heap allocator, no page-table walk at all.
- **Tradeoff:** the heap-backed version is cheaper (no map calls, no
  virtual-address bookkeeping) but has no boundary: an overflow silently
  overwrites whatever heap memory happens to sit below it (Doc 3 section 3's
  gap). The mapped-plus-guard-page version costs 4 page-table walks and a
  bump-allocated virtual slot per task, in exchange for turning a stack
  overflow into a catchable double fault (the same IST mechanism Doc 3
  section 5 already covers) instead of silent corruption -- verified
  directly by `tests/task_stack_overflow.rs`. Never reused or freed (Flint
  has no task teardown), so the bump allocator never needs a free list; a
  real multi-process kernel would need one once tasks can exit.

## Fault-safe user copy (M6 addendum, post-M7)

- **Cost:** O(1) per byte -- `copy_from_user_byte`/`copy_to_user_byte`
  (`syscall/mod.rs`) are a fixed handful of instructions each (a `lea`, a
  store to `RECOVERY_IP`, the one risky access), called once per byte in a
  bounded loop (already bounded by `MAX_WRITE_LEN`/`MAX_LINE_LEN`), so the
  cost is the same O(pages-in-range) shape "User-pointer check" above
  already pays, at the granularity of bytes instead of pages.
- **Alternative:** trust `validate_user_range`'s earlier check unconditionally
  and do a raw, unchecked copy (`slice::from_raw_parts` / a direct pointer
  write) -- what Flint did before this addendum.
- **Tradeoff:** the raw-copy version is faster (no per-byte recovery-point
  bookkeeping) but has a real TOCTOU gap: a mapping valid at
  `validate_user_range` time can be revoked before the copy runs, and the
  resulting kernel-mode page fault would panic the whole kernel (Doc 3
  section 4's specific requirement this addendum closes). The fault-safe
  version costs recording and clearing a global recovery pointer around
  every single-byte access, in exchange for turning that fault into a plain
  `Err` the syscall returns to its caller -- verified directly by
  `flint::syscall::tests::copy_helpers_return_err_instead_of_panicking_on_a_mid_copy_fault`,
  which deliberately unmaps a page mid-sequence and confirms the kernel
  survives. Sound as global (not per-CPU) state specifically because `int
  0x80` runs with interrupts disabled for its whole body (see
  `sys_read_line`'s doc comment) -- a real multi-process, interrupts-enabled-
  during-syscalls kernel would need this state to be per-task instead.

## Register dump on panic (M2/M4 addendum, post-M7)

- **Cost:** O(1) -- each GPR-capture trampoline (`interrupts.rs`) is a fixed
  15 `mov [addr], reg` instructions plus one `jmp`, run once per fault,
  before the real (unchanged) exception handler's own O(1) dispatch.
- **Alternative:** hand-parse the CPU-pushed exception frame directly in a
  naked replacement for each handler, reconstructing `InterruptStackFrame`/
  the error code from raw stack offsets instead of leaving that parsing to
  the existing, compiler-generated `extern "x86-interrupt" fn` bodies.
- **Tradeoff:** hand-parsing the frame would avoid the extra `jmp` (a
  handful of cycles), but re-derives a stack layout the `x86_64` crate and
  LLVM's `x86-interrupt` calling convention already get right today --
  exactly the kind of place this codebase's own docs warn a wrong offset
  "corrupts state silently" (see `task/context.rs`). The capture-then-jmp
  trampoline touches no stack or flags state at all, so the real handler
  sees identical CPU state to what plain `set_handler_fn` registration would
  have produced, at the cost of one extra jump. Verified directly by
  `tests/register_dump.rs`, which loads a known marker into `rax`
  immediately before a deliberate, genuine kernel-mode page fault and
  asserts the panic report's `rax=` field contains it exactly.
