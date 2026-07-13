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
