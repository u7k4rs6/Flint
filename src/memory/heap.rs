//! The kernel heap: a fixed-size-block allocator with a linked-list
//! fallback for anything larger than the biggest block class, installed as
//! the `#[global_allocator]` so `alloc` (`Box`, `Vec`, `BTreeMap`, ...)
//! works in the kernel from here on.
//!
//! Complexity and tradeoff (Doc 2 section 11): allocation for a size that
//! fits a block class is O(1) -- pop the head of that class's free list.
//! Deallocation is the same O(1) push. The alternative Doc 2 lists is a
//! plain linked-list (free-list) allocator, which is simpler but walks the
//! whole list on both alloc (first-fit search) and free (to merge adjacent
//! holes) -- O(n) in the number of outstanding holes. A buddy allocator
//! gets O(log n) splits/merges with low external fragmentation but is
//! meaningfully more code to get right. The fixed-size-block design was
//! chosen (per Doc 2's recommendation) as the middle ground: O(1) for the
//! common small/medium allocation sizes a kernel actually makes (task
//! control blocks, small `Vec`s, `Box`ed syscall arguments), at the cost of
//! internal fragmentation up to the next block size, and it still needs a
//! fallback path (here, `linked_list_allocator`'s free-list algorithm) for
//! anything bigger than the largest block class.

use crate::memory::paging::map_page;
use core::alloc::{GlobalAlloc, Layout};
use core::mem;
use core::ptr::NonNull;
use linked_list_allocator::Heap;
use spin::Mutex;
use x86_64::structures::paging::{Page, PageTableFlags};
use x86_64::VirtAddr;

pub const HEAP_START: usize = 0x_4444_4444_0000;
pub const HEAP_SIZE: usize = 1024 * 1024; // 1 MiB

/// Sizes chosen as a geometric progression covering common small
/// allocations; the smallest, 8 bytes, is also the minimum needed to hold
/// the free list's own intrusive next-pointer.
const BLOCK_SIZES: &[usize] = &[8, 16, 32, 64, 128, 256, 512, 1024, 2048];

struct ListNode {
    next: Option<&'static mut ListNode>,
}

fn list_index(layout: &Layout) -> Option<usize> {
    let required = layout.size().max(layout.align());
    BLOCK_SIZES.iter().position(|&s| s >= required)
}

pub struct FixedSizeBlockAllocator {
    list_heads: [Option<&'static mut ListNode>; BLOCK_SIZES.len()],
    fallback: Heap,
}

impl FixedSizeBlockAllocator {
    pub const fn new() -> Self {
        const EMPTY: Option<&'static mut ListNode> = None;
        FixedSizeBlockAllocator {
            list_heads: [EMPTY; BLOCK_SIZES.len()],
            fallback: Heap::empty(),
        }
    }

    /// # Safety
    /// `[heap_start, heap_start + heap_size)` must already be mapped,
    /// writable, and not aliased by anything else -- this claims the whole
    /// range as the fallback allocator's backing memory.
    unsafe fn init(&mut self, heap_start: usize, heap_size: usize) {
        // SAFETY: forwarded to this function's own contract above.
        unsafe { self.fallback.init(heap_start, heap_size) };
    }

    fn fallback_alloc(&mut self, layout: Layout) -> *mut u8 {
        match self.fallback.allocate_first_fit(layout) {
            Ok(ptr) => ptr.as_ptr(),
            Err(()) => core::ptr::null_mut(),
        }
    }
}

pub struct Locked<A> {
    inner: Mutex<A>,
}

impl<A> Locked<A> {
    pub const fn new(inner: A) -> Self {
        Locked {
            inner: Mutex::new(inner),
        }
    }

    fn lock(&self) -> spin::MutexGuard<'_, A> {
        self.inner.lock()
    }
}

#[global_allocator]
static ALLOCATOR: Locked<FixedSizeBlockAllocator> = Locked::new(FixedSizeBlockAllocator::new());

/// Maps the heap's virtual range one page at a time (via the shared
/// `map_page` used by demand paging too) and hands the range to the
/// allocator. Must run after `memory::paging::init`.
pub fn init_heap() -> Result<(), x86_64::structures::paging::mapper::MapToError<x86_64::structures::paging::Size4KiB>> {
    let page_range = {
        let heap_start = VirtAddr::new(HEAP_START as u64);
        let heap_end = heap_start + HEAP_SIZE as u64 - 1u64;
        let start_page = Page::containing_address(heap_start);
        let end_page = Page::containing_address(heap_end);
        Page::range_inclusive(start_page, end_page)
    };

    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
    for page in page_range {
        map_page(page, flags)?;
    }

    // SAFETY: every page in `page_range` was just mapped fresh above, so
    // the whole [HEAP_START, HEAP_START + HEAP_SIZE) range is valid,
    // writable, and not yet claimed by anything else.
    unsafe { ALLOCATOR.lock().init(HEAP_START, HEAP_SIZE) };

    Ok(())
}

unsafe impl GlobalAlloc for Locked<FixedSizeBlockAllocator> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut allocator = self.lock();
        match list_index(&layout) {
            Some(index) => match allocator.list_heads[index].take() {
                Some(node) => {
                    allocator.list_heads[index] = node.next.take();
                    node as *mut ListNode as *mut u8
                }
                None => {
                    let block_size = BLOCK_SIZES[index];
                    let block_layout = Layout::from_size_align(block_size, block_size).unwrap();
                    allocator.fallback_alloc(block_layout)
                }
            },
            None => allocator.fallback_alloc(layout),
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let mut allocator = self.lock();
        match list_index(&layout) {
            Some(index) => {
                debug_assert!(mem::size_of::<ListNode>() <= BLOCK_SIZES[index]);
                debug_assert!(mem::align_of::<ListNode>() <= BLOCK_SIZES[index]);
                let new_node = ListNode {
                    next: allocator.list_heads[index].take(),
                };
                let new_node_ptr = ptr as *mut ListNode;
                // SAFETY: `ptr` was handed out by `alloc` for this same
                // size class, so it points to a live, correctly aligned
                // block of at least `BLOCK_SIZES[index]` bytes that the
                // caller is done with; writing the free-list node into it
                // and re-borrowing it as `&'static mut` is the same
                // intrusive-list technique the frame allocator uses.
                unsafe {
                    new_node_ptr.write(new_node);
                    allocator.list_heads[index] = Some(&mut *new_node_ptr);
                }
            }
            None => {
                let ptr = NonNull::new(ptr).expect("dealloc of null pointer");
                // SAFETY: `ptr`/`layout` came from a prior `fallback_alloc`
                // call with this same layout (the `None` arm of `alloc`),
                // matching `Heap::deallocate`'s contract.
                unsafe { allocator.fallback.deallocate(ptr, layout) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::boxed::Box;
    use alloc::vec::Vec;

    #[test_case]
    fn boxed_value_round_trips() {
        let heap_value = Box::new(41);
        assert_eq!(*heap_value, 41);
    }

    #[test_case]
    fn large_vec_uses_every_slot() {
        let n = 2000;
        let mut vec = Vec::new();
        for i in 0..n {
            vec.push(i);
        }
        assert_eq!(vec.iter().sum::<u64>(), (n - 1) * n / 2);
    }

    #[test_case]
    fn many_boxes_dont_exhaust_the_heap() {
        // Exercises reuse: without freeing between iterations, this would
        // run the small heap out of memory long before 10_000 iterations.
        for i in 0..10_000 {
            let x = Box::new(i);
            assert_eq!(*x, i);
        }
    }

    #[test_case]
    fn large_allocation_uses_fallback_path() {
        // Bigger than the largest fixed-size block class (2048 bytes),
        // so this exercises the linked_list_allocator fallback rather
        // than a block-class free list.
        let big: Vec<u8> = alloc::vec![0u8; 8192];
        assert_eq!(big.len(), 8192);
    }
}
