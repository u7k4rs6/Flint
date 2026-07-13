//! The physical frame allocator.
//!
//! Complexity and tradeoff (Doc 2 section 11): `init` is O(frames) -- every
//! usable frame must be touched once to link it into the free list, which is
//! unavoidable no matter which allocator shape is chosen. After that,
//! `allocate_frame` and `deallocate_frame` are both O(1): each usable frame
//! becomes a node of an intrusive singly linked free list, threaded through
//! the *frame's own backing memory* (the first 8 bytes of each free frame
//! store the physical address of the next free frame, reached through the
//! bootloader's physical-memory offset mapping). The alternative from Doc 2
//! is a bitmap (one bit per frame, O(frames) scan per allocation unless a
//! hint is tracked). The intrusive list was chosen over a bitmap because it
//! gives O(1) alloc and free with zero extra storage -- the tradeoff Doc 2
//! calls out ("storage for the list vs scan time") disappears because the
//! list lives inside memory that is, by definition, otherwise unused while
//! free. The cost instead is a raw pointer write into physical memory on
//! every free, which is why `deallocate_frame` is unsafe: the caller must
//! guarantee the frame is truly unused from this point on.

use bootloader::bootinfo::{MemoryMap, MemoryRegionType};
use x86_64::structures::paging::{FrameAllocator, FrameDeallocator, PhysFrame, Size4KiB};
use x86_64::{PhysAddr, VirtAddr};

pub struct BootFrameAllocator {
    phys_mem_offset: VirtAddr,
    free_list_head: Option<PhysFrame>,
    frames_free: usize,
    frames_total: usize,
}

impl BootFrameAllocator {
    /// Builds the allocator by walking every `Usable` region in the
    /// bootloader's memory map and linking each frame into the free list.
    ///
    /// # Safety
    /// The caller must guarantee that `memory_map` is the genuine map
    /// handed to the kernel by the bootloader (so every frame it marks
    /// `Usable` really is unused RAM), and that `phys_mem_offset` is the
    /// virtual address at which the bootloader identity-mapped all physical
    /// memory (the `map_physical_memory` feature), since this allocator
    /// writes free-list links through that mapping.
    pub unsafe fn init(memory_map: &MemoryMap, phys_mem_offset: VirtAddr) -> Self {
        let mut allocator = BootFrameAllocator {
            phys_mem_offset,
            free_list_head: None,
            frames_free: 0,
            frames_total: 0,
        };

        for region in memory_map.iter() {
            if region.region_type != MemoryRegionType::Usable {
                continue;
            }
            let start = region.range.start_addr();
            let end = region.range.end_addr();
            let mut addr = start;
            while addr < end {
                // Never hand out physical frame 0: the classic BIOS data
                // area / real-mode IVT, and keeping it permanently
                // unavailable is one less way a stray physical-address-0
                // write could look "successful."
                if addr != 0 {
                    let frame = PhysFrame::containing_address(PhysAddr::new(addr));
                    // SAFETY: `frame` is inside a region the memory map
                    // marked `Usable` and has not been handed out yet (we
                    // are still in the one-time init walk), so writing a
                    // free-list link into it cannot corrupt live data.
                    unsafe { allocator.push_free(frame) };
                }
                addr += 4096;
            }
        }

        allocator
    }

    fn frame_ptr(&self, frame: PhysFrame) -> *mut u64 {
        (self.phys_mem_offset + frame.start_address().as_u64()).as_mut_ptr()
    }

    /// # Safety
    /// `frame` must not currently be in use by anyone else: pushing it onto
    /// the free list immediately makes it eligible to be handed back out by
    /// `allocate_frame`, and this write clobbers its first 8 bytes.
    unsafe fn push_free(&mut self, frame: PhysFrame) {
        let ptr = self.frame_ptr(frame);
        let next = self.free_list_head.map_or(u64::MAX, |f| f.start_address().as_u64());
        // SAFETY: `ptr` is a physical frame reached through the identity
        // offset mapping, which the bootloader guarantees is mapped
        // read-write for all of physical memory; the caller's contract
        // above guarantees the frame is free to overwrite.
        unsafe { ptr.write(next) };
        self.free_list_head = Some(frame);
        self.frames_free += 1;
        self.frames_total += 1;
    }
}

unsafe impl FrameAllocator<Size4KiB> for BootFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        let frame = self.free_list_head?;
        let ptr = self.frame_ptr(frame);
        // SAFETY: `frame` was linked in by `push_free`/`init`, both of which
        // only ever write a valid encoded next-pointer (a real physical
        // frame address or the u64::MAX end sentinel) to this location.
        let next = unsafe { ptr.read() };
        self.free_list_head = if next == u64::MAX {
            None
        } else {
            Some(PhysFrame::containing_address(PhysAddr::new(next)))
        };
        self.frames_free -= 1;
        Some(frame)
    }
}

impl FrameDeallocator<Size4KiB> for BootFrameAllocator {
    /// # Safety
    /// `frame` must have been allocated by this allocator and must no
    /// longer be referenced by any live mapping or data structure: freeing
    /// it makes it eligible for immediate reuse.
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame) {
        // SAFETY: forwarded to `push_free`'s contract, which the caller of
        // this function is required to uphold.
        unsafe { self.push_free(frame) };
    }
}

impl BootFrameAllocator {
    pub fn frames_free(&self) -> usize {
        self.frames_free
    }

    pub fn frames_total(&self) -> usize {
        self.frames_total
    }
}
