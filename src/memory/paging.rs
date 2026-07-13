//! Page tables: mapping, unmapping, and the mapper used both to build the
//! kernel heap and (in M4's page-fault handler) to demand-page valid
//! addresses.
//!
//! Complexity and tradeoff (Doc 2 section 11): map/unmap are O(levels) = O(1)
//! -- x86-64 uses fixed 4-level paging (PML4/PDPT/PD/PT), so walking to a
//! leaf entry is always exactly 4 steps regardless of how much memory is
//! mapped, allocating an intermediate table only when a level doesn't exist
//! yet. The alternative is huge pages (2 MiB/1 GiB), which reduce the
//! number of *entries* needed for a large range at the cost of coarser
//! granularity (can't map/protect/unmap less than the huge page's size).
//! Flint maps everything as 4 KiB pages for now; huge pages are a natural
//! extension once a range is known to be large and uniformly permissioned
//! (e.g. the whole kernel heap).

use crate::memory::with_frame_allocator;
use spin::Mutex;
use x86_64::structures::paging::{
    mapper::{MapToError, UnmapError},
    FrameAllocator, FrameDeallocator, Mapper, OffsetPageTable, Page, PageTable, PageTableFlags,
    Size4KiB,
};
use x86_64::VirtAddr;

static MAPPER: Mutex<Option<OffsetPageTable<'static>>> = Mutex::new(None);

/// # Safety
/// `physical_memory_offset` must be the virtual address at which the
/// bootloader identity-mapped all of physical memory, and this must be
/// called exactly once, since it takes a live `&mut` reference to the
/// currently-active (CR3) level 4 page table.
unsafe fn active_level_4_table(physical_memory_offset: VirtAddr) -> &'static mut PageTable {
    use x86_64::registers::control::Cr3;

    let (level_4_table_frame, _) = Cr3::read();
    let phys = level_4_table_frame.start_address();
    let virt = physical_memory_offset + phys.as_u64();
    let page_table_ptr: *mut PageTable = virt.as_mut_ptr();

    // SAFETY: `virt` is the CR3 frame reached through the physical-memory
    // offset mapping the caller guarantees is valid and complete, and this
    // function is only ever called once (from `init`), so no other `&mut`
    // to the same table can be alive concurrently.
    unsafe { &mut *page_table_ptr }
}

/// # Safety
/// Same contract as `active_level_4_table`; forwarded by `crate::init_memory`,
/// which calls this exactly once during boot before any code depends on the
/// mapper existing.
pub unsafe fn init(physical_memory_offset: VirtAddr) {
    // SAFETY: forwarded to the caller's contract above.
    let level_4_table = unsafe { active_level_4_table(physical_memory_offset) };
    let mapper = unsafe { OffsetPageTable::new(level_4_table, physical_memory_offset) };
    *MAPPER.lock() = Some(mapper);
}

/// Runs `f` with exclusive access to the global page table mapper. Panics if
/// called before `init`.
pub fn with_mapper<R>(f: impl FnOnce(&mut OffsetPageTable<'static>) -> R) -> R {
    let mut guard = MAPPER.lock();
    let mapper = guard.as_mut().expect("page table mapper not initialized");
    f(mapper)
}

/// Maps `page` to a freshly allocated frame with the given flags, using the
/// global physical frame allocator. Used both by the heap (a contiguous
/// range, mapped eagerly at heap init) and by demand paging (a single page,
/// mapped lazily on first fault).
pub fn map_page(page: Page<Size4KiB>, flags: PageTableFlags) -> Result<(), MapToError<Size4KiB>> {
    assert_ne!(
        page.start_address().as_u64(),
        0,
        "refusing to map virtual page 0 (Doc 3 section 3: null-pointer dereferences must fault, not read real memory)"
    );
    with_frame_allocator(|frame_allocator| {
        with_mapper(|mapper| {
            let frame = frame_allocator
                .allocate_frame()
                .ok_or(MapToError::FrameAllocationFailed)?;
            // SAFETY: `frame` was just allocated fresh from the frame
            // allocator (not currently backing any other mapping), and
            // `frame_allocator` is threaded through to satisfy Mapper's
            // requirement that intermediate page-table frames can also be
            // allocated on demand.
            unsafe { mapper.map_to(page, frame, flags, frame_allocator) }
                .map(|flush| flush.flush())
        })
    })
}

/// Unmaps `page`, returning its backing frame to the frame allocator.
pub fn unmap_page(page: Page<Size4KiB>) -> Result<(), UnmapError> {
    with_mapper(|mapper| {
        let (frame, flush) = mapper.unmap(page)?;
        flush.flush();
        with_frame_allocator(|frame_allocator| {
            // SAFETY: `unmap` just removed the only page table entry that
            // referenced `frame`, and Flint has no shared/copy-on-write
            // mappings, so nothing else can still be pointing at it.
            unsafe { frame_allocator.deallocate_frame(frame) };
        });
        Ok(())
    })
}

/// A virtual range the kernel treats as "valid but not yet backed": reading
/// or writing here is legitimate, but no frame is mapped until the first
/// access faults. Nothing eagerly maps this range; it exists to give the
/// page-fault handler (and its test) a concrete, deliberately lazy region
/// to demand-page into, standing in for the lazily grown kernel stacks a
/// later milestone would carve from the same mechanism.
pub const LAZY_REGION_START: u64 = 0x_5555_5555_0000;
pub const LAZY_REGION_SIZE: u64 = 16 * 4096;

pub fn is_lazy_region(addr: VirtAddr) -> bool {
    let start = LAZY_REGION_START;
    let end = LAZY_REGION_START + LAZY_REGION_SIZE;
    (start..end).contains(&addr.as_u64())
}

/// Maps a single freshly allocated frame at `addr` for demand paging: the
/// page-fault handler calls this for a fault on an address inside a region
/// the kernel considers valid-but-not-yet-backed (e.g. a lazily grown
/// stack), per Doc 2 section 5.2.
pub fn demand_page(addr: VirtAddr) -> Result<(), MapToError<Size4KiB>> {
    let page = Page::containing_address(addr);
    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
    map_page(page, flags)
}

#[cfg(test)]
mod tests {
    use super::{LAZY_REGION_START, LAZY_REGION_SIZE};
    use x86_64::VirtAddr;

    /// PRD definition-of-done gate: a page fault on a valid-but-unmapped
    /// page is handled and execution continues. This deliberately touches
    /// an address that is *not* mapped ahead of time -- the write below is
    /// exactly what triggers the fault -- and then verifies the value
    /// really landed, proving the handler mapped a real, usable frame
    /// rather than merely swallowing the fault.
    #[test_case]
    fn page_fault_on_lazy_region_is_handled_and_continues() {
        let addr = VirtAddr::new(LAZY_REGION_START + 4096 * 3 + 8);
        assert!(super::is_lazy_region(addr));
        assert_ne!(LAZY_REGION_SIZE, 0);

        let ptr = addr.as_mut_ptr::<u64>();
        // SAFETY: `addr` is inside the reserved lazy region, which nothing
        // else ever maps or writes to, so this pointer is exclusively ours;
        // the write below is the deliberate trigger for the not-present
        // page fault this test exists to exercise.
        unsafe {
            ptr.write_volatile(0xdead_beef);
            assert_eq!(ptr.read_volatile(), 0xdead_beef);
        }
    }
}

