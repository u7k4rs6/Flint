//! The task model: a kernel thread with a saved register state, its own
//! kernel stack, and (for now) no address-space root of its own -- every
//! task shares the kernel's single CR3, per Doc 2 section 1 ("everything
//! runs in the kernel's world until user mode exists"). A per-task CR3
//! arrives with user processes in M6.

pub mod context;
pub mod scheduler;

use alloc::boxed::Box;

pub type TaskId = u64;

const STACK_SIZE: usize = 16 * 1024;

pub struct Task {
    pub id: TaskId,
    pub name: &'static str,
    /// The saved stack pointer to resume at, valid only while this task is
    /// *not* the one currently running (while running, its live state is
    /// in the CPU's actual registers, not here).
    rsp: u64,
    /// Owns the task's kernel stack for its whole lifetime. Never read
    /// directly after construction -- `rsp` (and whatever the task itself
    /// pushed while running) is the only view into it -- but it must stay
    /// alive exactly as long as the task can still be switched to.
    _stack: Box<[u8]>,
}

impl Task {
    /// Builds a new task whose first `context::switch` into it starts
    /// executing `entry`.
    fn new(id: TaskId, name: &'static str, entry: extern "C" fn() -> !) -> Task {
        let stack = alloc::vec![0u8; STACK_SIZE].into_boxed_slice();
        let stack_top = (stack.as_ptr() as u64 + STACK_SIZE as u64) & !0xf;

        // SAFETY: `stack` was just allocated fresh (STACK_SIZE bytes,
        // exclusively owned by this Task from here on) and `stack_top` is
        // 16-byte aligned by the mask above.
        let rsp = unsafe { context::init_stack(stack_top, entry) };

        Task {
            id,
            name,
            rsp,
            _stack: stack,
        }
    }

    /// A placeholder task representing whatever context is running at the
    /// moment the scheduler takes over (the kernel's own boot thread). Its
    /// `rsp` is a dummy until the first switch *out* of it fills in the
    /// real value, exactly the way every other task's `rsp` is updated.
    fn placeholder(id: TaskId, name: &'static str) -> Task {
        Task {
            id,
            name,
            rsp: 0,
            _stack: alloc::vec![].into_boxed_slice(),
        }
    }
}
