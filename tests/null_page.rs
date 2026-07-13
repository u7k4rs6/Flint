//! Required isolation gate (Doc 3 section 3 / section 7 checklist): page 0
//! is left unmapped, so a null-pointer dereference is a clean page fault,
//! not a read of real memory. This deliberately dereferences virtual
//! address 0 and asserts the fault is a genuine not-present fault on page
//! 0 -- not a crash, not a silent success, and not routed through the lazy
//! demand-paging region (which starts well above address 0).

#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;
use flint::{gdt, qemu, serial_print, serial_println};
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

entry_point!(kmain);

fn kmain(_boot_info: &'static BootInfo) -> ! {
    serial_print!("null_page::null_page...\t");

    gdt::init();
    init_test_idt();

    // SAFETY: this is the deliberate trigger for the fault this test
    // exists to observe; the handler below never returns to here, it exits
    // QEMU directly, so the actual (would-be UB) read never completes.
    unsafe {
        let ptr = 0 as *const u8;
        core::ptr::read_volatile(ptr);
    }

    panic!("execution continued after a null-pointer dereference, should be unreachable");
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    flint::test_panic_handler(info)
}

lazy_static::lazy_static! {
    static ref TEST_IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.page_fault.set_handler_fn(test_page_fault_handler);
        idt
    };
}

fn init_test_idt() {
    TEST_IDT.load();
}

extern "x86-interrupt" fn test_page_fault_handler(
    _stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let accessed = x86_64::registers::control::Cr2::read().expect("valid CR2");

    let is_not_present = !error_code.contains(PageFaultErrorCode::PROTECTION_VIOLATION);
    let is_page_zero = accessed.as_u64() == 0;

    if is_not_present && is_page_zero {
        serial_println!("[ok]");
        qemu::exit_qemu(qemu::QemuExitCode::Success);
    }

    serial_println!(
        "[failed] unexpected fault: address {:?}, error code {:?}",
        accessed,
        error_code
    );
    qemu::exit_qemu(qemu::QemuExitCode::Failed);
}
