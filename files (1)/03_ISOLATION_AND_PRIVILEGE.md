# Flint: Isolation and Privilege

**Doc 3 of 4** (the security-and-access slot, reframed for a kernel)
**Status:** Draft v1

A kernel has no logins or roles. Its security model is the boundary between privileged and unprivileged code and the isolation between address spaces. This doc specifies that boundary, and it reads the kernel the way an attacker would: what can a hostile user process attempt, and what stops it. That framing is the same adversarial lens as the Cairn threat model, pointed at the metal.

---

## 1. Overview

Flint has three isolation jobs:

1. **Privilege separation:** kernel code runs in ring 0 and can do anything; user code runs in ring 3 and can do almost nothing without asking.
2. **Memory isolation:** each process sees only its own address space, and no user code can read or write kernel memory.
3. **The syscall trust boundary:** the one controlled doorway from ring 3 into ring 0, where every argument from user mode is treated as hostile until validated.

## 2. The privilege model (ring 0 versus ring 3)

- **Ring 0 (kernel):** full access to privileged instructions, all of memory, port I/O, control registers, and the page tables.
- **Ring 3 (user):** cannot execute privileged instructions, cannot touch I/O ports or control registers, and can reach memory only through mappings the kernel granted. Any attempt faults into the kernel.
- **Enforcement:** the current privilege level is carried in the segment registers (set from the GDT), and page-table permission bits gate memory. The CPU raises a general-protection or page fault when ring 3 oversteps, and the kernel decides what to do.

## 3. Memory isolation

- **Separate address spaces.** Each process has its own page tables (its own CR3). One process cannot name another's memory because it has no mapping to it.
- **The user/supervisor bit.** Every page-table entry marks whether ring 3 may access that page. Kernel pages are supervisor-only, so a user pointer into kernel space faults rather than reads.
- **W xor X.** No page is both writable and executable. Code pages are executable and read-only; data pages are writable and non-executable (the NX bit). This blocks the classic "write shellcode into a data page and jump to it" move.
- **Guard pages.** Kernel and user stacks are bounded by an unmapped guard page, so an overflow faults instead of silently corrupting neighboring memory.
- **Do not map page 0.** Leaving the lowest page unmapped turns a null-pointer dereference into a clean fault rather than a read of real data.

## 4. The syscall trust boundary (the critical control)

This is the kernel's equivalent of Cairn's ref-update authorization: the one place where untrusted input crosses into the trusted core, and therefore the place to get exactly right.

- **Never trust a user pointer or length.** On every syscall that takes a pointer, validate before dereferencing: the address range is within user space, every page in the range is mapped, and every page is user-accessible. Only then copy, using a checked copy-in and copy-out path (the copy_from_user and copy_to_user pattern).
- **Validate lengths and arithmetic.** A length from user mode can be enormous or can overflow when added to a base. Bound it and check for overflow before it is used to size a copy or an allocation.
- **Fault handling during a copy.** A user pointer can pass a shallow check and still fault mid-copy (for example a partially mapped range). The copy path must handle a fault gracefully and return an error, not panic the kernel.
- **Return errors, never crash.** A bad argument yields an error code to the caller; it must never take the kernel down. The kernel staying alive under hostile arguments is a tested acceptance criterion.

## 5. Fault and exception safety

- **The double-fault stack.** A double fault (exception 8) runs on its own IST stack, separate from the kernel stack, so that a kernel stack overflow escalates to a catchable double fault instead of a triple fault that resets the machine with no information. Standing this up early is what keeps stack bugs debuggable.
- **User faults versus kernel faults.** A page fault from ring 3 is a user error (kill or signal the process, or map on demand); the same fault in ring 0 is a kernel bug and should panic loudly with a register dump. The handler distinguishes them by the saved privilege level.
- **Interrupts during kernel work.** Be deliberate about when interrupts are enabled. Critical sections that touch shared kernel state disable interrupts (or use a proper lock) so a timer tick cannot reenter and corrupt them.

## 6. Threat model

Read as a hostile user process.

### 6.1 Assets
The integrity of kernel memory and control flow; the isolation between processes; the availability of the system (no single process should be able to wedge it).

### 6.2 Trust boundaries
Ring 3 to ring 0 (the syscall and fault entry points); one process's address space to another's; the CPU's asynchronous events (timer, keyboard) to the handlers that run on them.

### 6.3 Threats and mitigations

| Threat | Vector | Mitigation |
|---|---|---|
| Kernel memory read or write from user mode | A user pointer aimed at kernel space | Supervisor-only kernel pages; validate every user pointer before use |
| Code injection | Writing instructions into a data page and jumping to them | W xor X plus the NX bit, so writable pages are never executable |
| Null-deref information leak or crash | Dereferencing address 0 | Page 0 is left unmapped, turning it into a clean fault |
| Kernel stack overflow to triple fault | Deep or runaway recursion in the kernel | Guard pages and the double-fault IST stack, so it becomes a catchable fault |
| Integer overflow in a syscall length | A crafted length that overflows a size computation | Bound and overflow-check all lengths before sizing copies or allocations |
| Fault mid-copy across the boundary | A partially mapped user buffer | A copy path that catches the fault and returns an error rather than panicking |
| CPU monopolization (denial of service) | A user process that never yields | Preemptive scheduling on the timer removes the CPU regardless of the process's cooperation |
| Kernel-memory exhaustion | A process that drives the kernel to allocate without bound | Bound per-process kernel allocations and reject beyond a limit |
| Reentrancy corruption | An interrupt firing inside a kernel critical section | Disable interrupts or lock around shared-state critical sections |

### 6.4 Explicitly out of scope for v1
SMP concurrency safety (single core in v1), Meltdown and Spectre mitigations (KPTI, speculation barriers), ASLR, a full capability system, and secure boot. Named so their absence is a decision, not an oversight. Each becomes a natural extension and a talking point.

## 7. Build-time safety checklist

- [ ] Kernel pages are supervisor-only; a user pointer into kernel space faults.
- [ ] W xor X holds: no page is both writable and executable; data pages are NX.
- [ ] Page 0 is unmapped; stacks have guard pages.
- [ ] The double-fault handler runs on its own IST stack, verified by a stack-overflow test.
- [ ] Every syscall validates user pointers (range, mapped, user-accessible) and bounds and overflow-checks lengths before use.
- [ ] The copy-in and copy-out path handles a mid-copy fault and returns an error.
- [ ] A syscall with a hostile argument returns an error and leaves the kernel running (a tested case).
- [ ] Shared kernel state is protected against interrupt reentrancy.
