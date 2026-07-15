//! The task model: a kernel thread with a saved register state, a mapped
//! kernel stack bounded by an unmapped guard page (Doc 3 section 3), and no
//! address-space root of its own -- every task shares the kernel's single
//! CR3. Per-process address spaces exist for ring-3 *processes* (see
//! `user/mod.rs`, `memory::paging::new_address_space`), but ring-3
//! processes are not scheduler `Task`s in Flint, so this remains accurate
//! for kernel threads specifically; see DECISIONS.md's M6 addendum.

pub mod context;
pub mod scheduler;

use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::structures::paging::{Page, PageTableFlags, Size4KiB};
use x86_64::VirtAddr;

pub type TaskId = u64;

const STACK_SIZE: u64 = 16 * 1024;
const GUARD_SIZE: u64 = 4096;

/// Deliberately far from every other reserved region (the heap at
/// `0x_4444_4444_0000`, the lazy demand-paging region at
/// `0x_5555_5555_0000`, the user code/stack regions at
/// `0x_2000_0000_0000`/`0x_3000_0000_0000`): a virtual-address range
/// reserved for kernel task stacks, one guard page + `STACK_SIZE` mapped
/// pages per task, bump-allocated and never reused -- Flint has no task
/// teardown (a spawned task runs forever; see `scheduler.rs`), so there is
/// nothing to free back into a real allocator yet.
const STACK_REGION_START: u64 = 0x_6666_6666_0000;

static NEXT_STACK_BASE: AtomicU64 = AtomicU64::new(STACK_REGION_START);

/// Maps a fresh `STACK_SIZE`-byte kernel stack with an unmapped guard page
/// immediately below it, so a stack overflow takes a page fault (Doc 3
/// section 3, the build-time checklist's "stacks have guard pages") instead
/// of silently corrupting whatever memory happened to sit below a plain
/// heap allocation. Returns the mapped region's top (the initial `rsp`
/// `context::init_stack` builds its frame under).
fn map_task_stack() -> VirtAddr {
    let base = NEXT_STACK_BASE.fetch_add(GUARD_SIZE + STACK_SIZE, Ordering::Relaxed);
    // `base..base + GUARD_SIZE` is deliberately left unmapped -- nothing
    // ever maps it, which *is* the guard page; there is no "unmap" call to
    // make here, only an absence to preserve.
    let stack_start = VirtAddr::new(base + GUARD_SIZE);
    let stack_top = stack_start + STACK_SIZE;

    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE;
    let start_page: Page<Size4KiB> = Page::containing_address(stack_start);
    let end_page: Page<Size4KiB> = Page::containing_address(stack_top - 1u64);
    for page in Page::range_inclusive(start_page, end_page) {
        crate::memory::paging::map_page(page, flags).expect("failed to map a task stack page");
    }

    stack_top
}

pub struct Task {
    pub id: TaskId,
    pub name: &'static str,
    /// The saved stack pointer to resume at, valid only while this task is
    /// *not* the one currently running (while running, its live state is
    /// in the CPU's actual registers, not here).
    rsp: u64,
    /// The top of this task's mapped stack region, kept only for
    /// debugging/introspection (e.g. a future `ps` extension) -- `rsp`
    /// (and whatever the task itself pushed while running) is the only
    /// view `context::switch` actually needs. The mapped pages themselves
    /// are intentionally never unmapped: Flint has no task teardown, so
    /// every spawned task's stack lives for the kernel's entire uptime,
    /// same as the task itself.
    _stack_top: VirtAddr,
}

impl Task {
    /// Builds a new task whose first `context::switch` into it starts
    /// executing `entry`.
    fn new(id: TaskId, name: &'static str, entry: extern "C" fn() -> !) -> Task {
        let stack_top = map_task_stack();

        // SAFETY: `map_task_stack` just mapped `[stack_top - STACK_SIZE,
        // stack_top)` fresh, exclusively for this task, and `stack_top` is
        // page-aligned (hence 16-byte aligned).
        let rsp = unsafe { context::init_stack(stack_top.as_u64(), entry) };

        Task {
            id,
            name,
            rsp,
            _stack_top: stack_top,
        }
    }

    /// A placeholder task representing whatever context is running at the
    /// moment the scheduler takes over (the kernel's own boot thread). Its
    /// `rsp` is a dummy until the first switch *out* of it fills in the
    /// real value, exactly the way every other task's `rsp` is updated, and
    /// it needs no mapped stack of its own -- it never becomes the target
    /// of a `context::switch` (there is nothing to resume into) until after
    /// that first switch-out has filled in a real `rsp` from whatever stack
    /// the boot thread was already running on.
    fn placeholder(id: TaskId, name: &'static str) -> Task {
        Task {
            id,
            name,
            rsp: 0,
            _stack_top: VirtAddr::zero(),
        }
    }
}
