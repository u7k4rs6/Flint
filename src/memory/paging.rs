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
    PhysFrame, Size4KiB,
};
use x86_64::VirtAddr;

static MAPPER: Mutex<Option<OffsetPageTable<'static>>> = Mutex::new(None);

/// The virtual address at which the bootloader identity-mapped all of
/// physical memory, set once by `init`. Needed by `new_address_space` (to
/// clone an arbitrary PML4 frame's contents) and `activate` (to rebuild the
/// cached `MAPPER` over a newly activated one), in addition to `init` itself.
static PHYS_MEM_OFFSET: Mutex<Option<VirtAddr>> = Mutex::new(None);

fn phys_mem_offset() -> VirtAddr {
    PHYS_MEM_OFFSET.lock().expect("paging not initialized")
}

/// The virtual address at which the bootloader's `map_physical_memory`
/// feature identity-mapped all of physical memory, exposed so other kernel
/// subsystems that need to reach a specific physical address directly (e.g.
/// `vga`, whose 0xb8000 text buffer is ordinary physical memory, not
/// something Flint maps itself) can do so through the same, single, already
/// -established mapping rather than creating their own.
pub fn physical_memory_offset() -> VirtAddr {
    phys_mem_offset()
}

/// # Safety
/// `frame` must be a valid, fully-formed level 4 page table (either the
/// frame CR3 currently names, or a frame `new_address_space` has just
/// finished cloning into), and `offset` must be the physical-memory offset
/// mapping's virtual base. May be called more than once over the kernel's
/// lifetime (today: once from `init`, and once per address-space switch
/// from `activate`), but never while an earlier `&mut PageTable` reference
/// this function returned could still be alive -- both call sites
/// immediately fold the result into the single cached `MAPPER` and never
/// hold onto an older reference.
unsafe fn level_4_table_at(frame: PhysFrame<Size4KiB>, offset: VirtAddr) -> &'static mut PageTable {
    let virt = offset + frame.start_address().as_u64();
    let page_table_ptr: *mut PageTable = virt.as_mut_ptr();

    // SAFETY: forwarded to this function's contract above.
    unsafe { &mut *page_table_ptr }
}

/// # Safety
/// `physical_memory_offset` must be the virtual address at which the
/// bootloader identity-mapped all of physical memory, and this must be
/// called exactly once, during boot, before any code depends on the mapper
/// existing.
pub unsafe fn init(physical_memory_offset: VirtAddr) {
    *PHYS_MEM_OFFSET.lock() = Some(physical_memory_offset);

    use x86_64::registers::control::Cr3;
    let (boot_frame, _) = Cr3::read();

    // SAFETY: `boot_frame` is whatever CR3 already names at boot (a valid,
    // currently-active level 4 table handed off by the bootloader), and
    // this is the first-ever call to `level_4_table_at`.
    let level_4_table = unsafe { level_4_table_at(boot_frame, physical_memory_offset) };
    let mapper = unsafe { OffsetPageTable::new(level_4_table, physical_memory_offset) };
    *MAPPER.lock() = Some(mapper);
}

/// Builds a brand new, private top-level (PML4) page table by cloning every
/// entry out of the currently-active one, so a fresh process gets its own
/// address space without losing the kernel: entries already populated at
/// clone time (the kernel image, the physical-memory-offset mapping, the
/// heap) end up pointing at the *same* physical PDPT/PD/PT sub-tables as
/// the source, so they stay identically mapped and reachable from the new
/// table too. Entries not yet populated (e.g. the PML4 slots a user
/// process's own code/stack pages will use) start absent in the clone and
/// can be populated afterward without being visible from any other table --
/// this is what gives each process real isolation (Doc 3 section 3,
/// PRD FR-MEM-2).
///
/// Relies on process creation always happening from the kernel's own,
/// still-active address space (Flint has no fork/exec; every real call site
/// -- `user::setup`/`setup_shell`, once each per boot -- runs before any
/// process CR3 has ever been activated, so "currently active" here always
/// means the boot table, never a previously created process's table). See
/// DECISIONS.md.
pub fn new_address_space() -> PhysFrame<Size4KiB> {
    use x86_64::registers::control::Cr3;

    let (current_frame, _) = Cr3::read();
    let offset = phys_mem_offset();
    let new_frame = with_frame_allocator(|fa| {
        fa.allocate_frame()
            .expect("out of physical frames while building a new address space")
    });

    let src: *const u64 = (offset + current_frame.start_address().as_u64()).as_ptr();
    let dst: *mut u64 = (offset + new_frame.start_address().as_u64()).as_mut_ptr();
    // SAFETY: `current_frame` is the live CR3 target, a real, fully-formed
    // 4096-byte level 4 table reached through the complete physical-memory
    // offset mapping; `new_frame` was just allocated fresh from the frame
    // allocator (not yet referenced by any mapping or table), giving 4096
    // bytes of exclusively-owned room for the copy -- the two ranges cannot
    // overlap.
    unsafe { core::ptr::copy_nonoverlapping(src, dst, 512) };

    // Sanity check the copy landed: cheap confirmation the mechanism ran,
    // not a claim about correctness beyond that.
    debug_assert_eq!(
        {
            let src_table = unsafe { &*(src as *const PageTable) };
            src_table.iter().filter(|e| !e.is_unused()).count()
        },
        {
            let dst_table = unsafe { &*(dst as *const PageTable) };
            dst_table.iter().filter(|e| !e.is_unused()).count()
        },
        "new_address_space: cloned PML4 entry count doesn't match the source"
    );

    new_frame
}

/// Switches to `frame` as the active address space and rebuilds the cached
/// `MAPPER` to match, so `with_mapper`/`map_page`/`unmap_page`/`demand_page`
/// -- and, unchanged, `syscall::validate_user_range` and the page-fault
/// handler's demand-paging call -- all transparently target whichever
/// address space is active from here on, with no call-site changes outside
/// this module.
///
/// # Safety
/// `frame` must already carry the full kernel-half mapping (kernel
/// `.text`/`.data`, the IDT, the current kernel stack, the phys-mem-offset
/// mapping the frame allocator dereferences raw pointers through, and the
/// code executing this very function) identically to the table being
/// switched away from, because interrupts stay enabled across the write and
/// an IRQ can land on the very next instruction. In practice this means
/// `frame` must be a table `new_address_space` produced, activated before
/// any further mapping call. CR3 must only ever be written through this
/// function, never directly, or `MAPPER` goes stale and silently mutates
/// the wrong table.
pub fn activate(frame: PhysFrame<Size4KiB>) {
    use x86_64::registers::control::Cr3;

    // Preserve the current CR3 flags (PWT/PCD) rather than assuming they're
    // empty -- Flint doesn't set them itself, but whatever the bootloader
    // left them as should carry over rather than being silently reset.
    let (_, flags) = Cr3::read();
    // SAFETY: forwarded to this function's contract above.
    unsafe { Cr3::write(frame, flags) };

    let offset = phys_mem_offset();
    // SAFETY: `frame` was just activated above, and this is the only other
    // call site of `level_4_table_at` besides `init`; the mapper this
    // replaces is dropped here, so no stale `&mut PageTable` from it can
    // still be alive.
    let level_4_table = unsafe { level_4_table_at(frame, offset) };
    let mapper = unsafe { OffsetPageTable::new(level_4_table, offset) };
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

