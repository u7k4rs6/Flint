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
