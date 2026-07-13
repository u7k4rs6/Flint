//! Physical memory management. Paging and the kernel heap (M4) are layered
//! on top of the frame allocator here.

pub mod frame;

use bootloader::bootinfo::MemoryMap;
use frame::BootFrameAllocator;
use spin::Mutex;
use x86_64::VirtAddr;

static FRAME_ALLOCATOR: Mutex<Option<BootFrameAllocator>> = Mutex::new(None);

/// Brings up the physical frame allocator from the bootloader's memory map.
///
/// # Safety
/// Must be called exactly once, early in boot, with the genuine `BootInfo`
/// the bootloader handed the kernel (see `BootFrameAllocator::init`'s
/// contract, which this forwards).
pub unsafe fn init(memory_map: &MemoryMap, physical_memory_offset: u64) {
    let phys_mem_offset = VirtAddr::new(physical_memory_offset);
    // SAFETY: forwarded to the caller's contract above.
    let allocator = unsafe { BootFrameAllocator::init(memory_map, phys_mem_offset) };
    *FRAME_ALLOCATOR.lock() = Some(allocator);
}

/// Runs `f` with exclusive access to the global frame allocator. Panics if
/// called before `init`.
pub fn with_frame_allocator<R>(f: impl FnOnce(&mut BootFrameAllocator) -> R) -> R {
    let mut guard = FRAME_ALLOCATOR.lock();
    let allocator = guard.as_mut().expect("frame allocator not initialized");
    f(allocator)
}

#[cfg(test)]
mod tests {
    use super::with_frame_allocator;
    use x86_64::structures::paging::{FrameAllocator, FrameDeallocator};

    #[test_case]
    fn allocated_frames_are_distinct_and_nonzero() {
        let (a, b, c) = with_frame_allocator(|fa| {
            (
                fa.allocate_frame().expect("frame a"),
                fa.allocate_frame().expect("frame b"),
                fa.allocate_frame().expect("frame c"),
            )
        });
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
        assert_ne!(a.start_address().as_u64(), 0);
        assert_ne!(b.start_address().as_u64(), 0);
        assert_ne!(c.start_address().as_u64(), 0);
    }

    #[test_case]
    fn freed_frame_is_reused() {
        let (free_before, first) = with_frame_allocator(|fa| (fa.frames_free(), fa.allocate_frame().unwrap()));

        // SAFETY: `first` was just allocated above and nothing else has
        // touched it since, so it is genuinely free to hand back.
        with_frame_allocator(|fa| unsafe { fa.deallocate_frame(first) });

        let (free_after, second) = with_frame_allocator(|fa| (fa.frames_free(), fa.allocate_frame().unwrap()));

        // The free list is LIFO, so the very next allocation after a single
        // free must return exactly the frame that was just freed.
        assert_eq!(first, second);
        assert_eq!(free_before, free_after);
    }
}
