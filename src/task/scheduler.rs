//! Preemptive round-robin scheduling, driven by the timer interrupt.
//!
//! Complexity and tradeoff (Doc 2 section 11): picking the next task is
//! O(1) -- pop the front of a ring (here, a `VecDeque` used strictly as a
//! FIFO ring, not searched). The alternative is per-priority ready queues,
//! which improve responsiveness for latency-sensitive tasks at the cost of
//! needing a ready structure per priority level and raising starvation
//! questions Flint does not need to answer yet (every task is equally
//! important, v1). Round robin was chosen because it is fair and trivial,
//! matching Doc 2 section 6.3's recommendation to start there.

use super::{context, Task, TaskId};
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use spin::Mutex;

struct Scheduler {
    current: Option<Box<Task>>,
    ready_queue: VecDeque<Box<Task>>,
    next_id: TaskId,
    switches: u64,
}

static SCHEDULER: Mutex<Option<Scheduler>> = Mutex::new(None);

/// Sets up an empty scheduler with one placeholder task (id 0) standing in
/// for whatever context calls this -- typically the kernel's own boot
/// thread. Safe to call with no tasks ever spawned: `timer_tick` is then
/// simply a no-op on every tick.
///
/// Runs with interrupts disabled for its whole body, like every other
/// function below that locks `SCHEDULER` from outside an interrupt handler
/// (`timer_tick` is the one exception -- it's only ever reached *through*
/// one, where IF is already 0). Without this, a timer tick landing while
/// this same core holds `SCHEDULER`'s lock would have `timer_tick` spin
/// forever trying to re-acquire a lock only this now-interrupted code can
/// release -- a real bug this project hit and fixed, not a hypothetical:
/// `spawn` below used to be safe only because `Task::new` did negligible
/// work (a heap alloc); once it started mapping real pages for a guarded
/// stack (Doc 3 section 3), the critical section grew long enough to
/// intermittently overlap a 10ms tick, deadlocking `tests/task_stack_overflow.rs`
/// roughly half the time. See DECISIONS.md.
pub fn init() {
    x86_64::instructions::interrupts::without_interrupts(|| {
        *SCHEDULER.lock() = Some(Scheduler {
            current: Some(Box::new(Task::placeholder(0, "boot"))),
            ready_queue: VecDeque::new(),
            next_id: 1,
            switches: 0,
        });
    });
}

/// Adds a new task to the ready queue. Returns its id.
pub fn spawn(name: &'static str, entry: extern "C" fn() -> !) -> TaskId {
    x86_64::instructions::interrupts::without_interrupts(|| {
        let mut guard = SCHEDULER.lock();
        let sched = guard.as_mut().expect("scheduler not initialized");
        let id = sched.next_id;
        sched.next_id += 1;
        sched
            .ready_queue
            .push_back(Box::new(Task::new(id, name, entry)));
        id
    })
}

pub fn switch_count() -> u64 {
    x86_64::instructions::interrupts::without_interrupts(|| {
        let guard = SCHEDULER.lock();
        guard.as_ref().map_or(0, |s| s.switches)
    })
}

/// The id of whatever task is currently running, for attributing log lines
/// and panic reports (Doc 4 section 2/5: "the current task id, so
/// interleaved output is attributable"). `None` before `scheduler::init`
/// has run (there is no task yet, current or otherwise) -- callers that log
/// during that early-boot window just omit the `[task N]` tag.
pub fn current_task_id() -> Option<TaskId> {
    x86_64::instructions::interrupts::without_interrupts(|| {
        let guard = SCHEDULER.lock();
        guard.as_ref().and_then(|s| s.current.as_ref().map(|t| t.id))
    })
}

/// Called from the timer interrupt handler, after it has already
/// acknowledged the interrupt to the PIC (a context switch may not return
/// here for a long time, and the PIC must not be left thinking IRQ0 is
/// still in service in the meantime). Picks the next ready task and
/// switches to it, moving the current task to the back of the ready queue.
/// A no-op if the scheduler has not been started or nothing else is ready.
pub fn timer_tick() {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(sched) => sched,
        None => return,
    };

    if sched.ready_queue.is_empty() {
        return;
    }

    let next = sched.ready_queue.pop_front().expect("checked non-empty above");
    let mut outgoing = sched.current.take().expect("current task always set");

    // Heap-allocated (`Box<Task>`), so this address stays valid no matter
    // how `ready_queue`/`current` are mutated afterward -- only the `Box`
    // handle moves, never the `Task` it points to.
    let old_rsp: *mut u64 = &mut outgoing.rsp;
    let new_rsp = next.rsp;

    sched.ready_queue.push_back(outgoing);
    sched.current = Some(next);
    sched.switches += 1;

    drop(guard);

    // SAFETY: `old_rsp` points into the outgoing task's heap-allocated TCB,
    // which is now owned by `sched.ready_queue` but not otherwise touched
    // until it is popped and switched to again. `new_rsp` is either a
    // value a prior call to this function saved here, or (the task's first
    // run) the result of `context::init_stack`. Interrupts are disabled
    // for this entire function (we are inside an interrupt-gate handler
    // with IF=0 until some task's saved rflags eventually restores it), so
    // nothing else can touch `SCHEDULER` between dropping `guard` and this
    // call.
    unsafe { context::switch(old_rsp, new_rsp) };
}

#[cfg(test)]
mod tests {
    use super::{spawn, switch_count};
    use core::sync::atomic::{AtomicU64, Ordering};

    static COUNTER_A: AtomicU64 = AtomicU64::new(0);
    static COUNTER_B: AtomicU64 = AtomicU64::new(0);

    extern "C" fn task_a() -> ! {
        loop {
            COUNTER_A.fetch_add(1, Ordering::Relaxed);
            x86_64::instructions::hlt();
        }
    }

    extern "C" fn task_b() -> ! {
        loop {
            COUNTER_B.fetch_add(1, Ordering::Relaxed);
            x86_64::instructions::hlt();
        }
    }

    /// PRD gate: two tasks alternate under the timer a known number of
    /// times. Both `task_a` and `task_b` loop forever with no voluntary
    /// yield, so the only way *either* counter advances past its first
    /// increment is genuine preemption -- if the scheduler only ran the
    /// first-spawned task, the other's counter would stay at 0 forever.
    /// Observing both reach the target is direct proof of preemptive
    /// alternation, not just "the scheduler ran once."
    #[test_case]
    fn two_tasks_alternate_under_the_timer() {
        let switches_before = switch_count();
        spawn("task_a", task_a);
        spawn("task_b", task_b);

        const TARGET: u64 = 20;
        while COUNTER_A.load(Ordering::Relaxed) < TARGET || COUNTER_B.load(Ordering::Relaxed) < TARGET
        {
            x86_64::instructions::hlt();
        }

        assert!(COUNTER_A.load(Ordering::Relaxed) >= TARGET);
        assert!(COUNTER_B.load(Ordering::Relaxed) >= TARGET);
        // At least 2*TARGET actual context switches happened to get both
        // counters this far (plus whatever switched us in and out of this
        // very test task along the way).
        assert!(switch_count() - switches_before >= 2 * TARGET);
    }
}
