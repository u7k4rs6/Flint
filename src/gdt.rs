//! The GDT and TSS. In long mode segmentation is mostly flat, but the GDT
//! still carries the kernel/user code and data segment selectors that the
//! ring transition and syscall path rely on, and the TSS carries `rsp0`
//! (the stack the CPU switches to on a privilege-level change) and the IST
//! stacks used for fault handling that must not run on a possibly-corrupt
//! kernel stack.

use lazy_static::lazy_static;
use x86_64::instructions::segmentation::{Segment, CS, DS, ES, SS};
use x86_64::instructions::tables::load_tss;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;

/// Index into the TSS's interrupt stack table that the double-fault handler
/// runs on. A dedicated stack means a kernel stack overflow -- which would
/// otherwise fault again while the CPU tries to push the double-fault
/// exception frame onto the same already-overflowed stack, causing an
/// unrecoverable triple fault -- instead lands cleanly on fresh memory.
pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

const STACK_SIZE: usize = 4096 * 5;

/// `[u8; N]` alone has no alignment guarantee beyond 1 byte. The syscall
/// entry stub (`syscall::syscall_entry`) and the double-fault handler both
/// eventually `call` into normal Rust code from these stacks, and the
/// SysV ABI requires 16-byte stack alignment at a call site -- so the
/// *top* of each of these stacks (what becomes the initial `rsp`) must
/// itself be 16-byte aligned for that arithmetic to hold.
#[repr(align(16))]
#[allow(dead_code)] // only ever addressed via &raw const, never read as a value
struct AlignedStack([u8; STACK_SIZE]);

lazy_static! {
    static ref TSS: TaskStateSegment = {
        let mut tss = TaskStateSegment::new();
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = {
            // SAFETY: this `static mut` is written here once, before the TSS
            // (and therefore this address) is ever loaded into the CPU, and
            // is never accessed as a Rust value again afterward -- it exists
            // purely to reserve backing memory for the stack the CPU will
            // switch `rsp` to on IST entry.
            static mut DOUBLE_FAULT_STACK: AlignedStack = AlignedStack([0; STACK_SIZE]);
            let stack_start = VirtAddr::from_ptr(&raw const DOUBLE_FAULT_STACK);
            stack_start + STACK_SIZE as u64
        };
        // rsp0: the stack the CPU switches to on any ring 3 -> ring 0
        // transition (a syscall via `int 0x80`, or any exception/IRQ taken
        // while running user code). Without this, a privilege-level change
        // would try to keep using whatever rsp ring 3 had -- a user stack
        // the kernel cannot trust -- for kernel-mode exception handling.
        // One static stack is enough for M6 (a single demo user program,
        // no concurrent user processes yet); a real per-task rsp0 would be
        // written here by the scheduler on every switch.
        tss.privilege_stack_table[0] = {
            // SAFETY: same reasoning as DOUBLE_FAULT_STACK above.
            static mut PRIVILEGE_STACK: AlignedStack = AlignedStack([0; STACK_SIZE]);
            let stack_start = VirtAddr::from_ptr(&raw const PRIVILEGE_STACK);
            stack_start + STACK_SIZE as u64
        };
        tss
    };
}

struct Selectors {
    kernel_code_selector: SegmentSelector,
    kernel_data_selector: SegmentSelector,
    user_code_selector: SegmentSelector,
    user_data_selector: SegmentSelector,
    tss_selector: SegmentSelector,
}

lazy_static! {
    static ref GDT: (GlobalDescriptorTable, Selectors) = {
        let mut gdt = GlobalDescriptorTable::new();
        let kernel_code_selector = gdt.append(Descriptor::kernel_code_segment());
        let kernel_data_selector = gdt.append(Descriptor::kernel_data_segment());
        let user_code_selector = gdt.append(Descriptor::user_code_segment());
        let user_data_selector = gdt.append(Descriptor::user_data_segment());
        let tss_selector = gdt.append(Descriptor::tss_segment(&TSS));
        (
            gdt,
            Selectors {
                kernel_code_selector,
                kernel_data_selector,
                user_code_selector,
                user_data_selector,
                tss_selector,
            },
        )
    };
}

/// Selectors published for the ring-3 transition (M6) to load into the
/// segment registers of the user-mode IRETQ frame.
pub fn kernel_selectors() -> (SegmentSelector, SegmentSelector) {
    (GDT.1.kernel_code_selector, GDT.1.kernel_data_selector)
}

pub fn user_selectors() -> (SegmentSelector, SegmentSelector) {
    (GDT.1.user_code_selector, GDT.1.user_data_selector)
}

pub fn init() {
    GDT.0.load();
    // SAFETY: the selectors above were just added to the GDT that `load()`
    // installed, in the same order, so they name valid, loaded descriptors.
    // CS/DS/ES/SS must be reloaded after an LGDT because the CPU caches
    // segment descriptor contents in hidden registers that a bare LGDT does
    // not refresh; load_tss additionally requires the TSS descriptor's
    // "busy" bit be clear, which it is on first load.
    unsafe {
        CS::set_reg(GDT.1.kernel_code_selector);
        DS::set_reg(GDT.1.kernel_data_selector);
        ES::set_reg(GDT.1.kernel_data_selector);
        SS::set_reg(GDT.1.kernel_data_selector);
        load_tss(GDT.1.tss_selector);
    }
}
