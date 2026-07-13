//! Required isolation gate (Doc 3 section 5 / section 7 checklist): a kernel
//! stack overflow must be caught as a double fault on its own IST stack, not
//! a triple fault that silently resets the machine. This deliberately
//! overflows the kernel stack and asserts that the double-fault handler --
//! not a QEMU reset -- is what catches it.

#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;
use flint::{gdt, qemu, serial_print, serial_println};
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

entry_point!(kmain);

fn kmain(_boot_info: &'static BootInfo) -> ! {
    serial_print!("stack_overflow::stack_overflow...\t");

    gdt::init();
    init_test_idt();

    // Trigger the overflow. The volatile read after the recursive call
    // stops the compiler from turning this into a tail call (which would
    // never grow the stack and never fault).
    stack_overflow();

    panic!("execution continued after stack overflow, should be unreachable");
}

#[allow(unconditional_recursion)]
fn stack_overflow() {
    stack_overflow();
    volatile::Volatile::new(0u8).read();
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    flint::test_panic_handler(info)
}

// A test-local IDT: only the double-fault vector matters here, and it must
// point at a handler that reports success (not the kernel's normal
// panic-and-halt double-fault handler), since a caught double fault is
// exactly the outcome this test wants.
lazy_static::lazy_static! {
    static ref TEST_IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        unsafe {
            // SAFETY: gdt::init() above has already loaded the TSS with the
            // double-fault IST stack at this index, so this names a valid,
            // installed IST entry.
            idt.double_fault
                .set_handler_fn(test_double_fault_handler)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        }
        idt
    };
}

fn init_test_idt() {
    TEST_IDT.load();
}

extern "x86-interrupt" fn test_double_fault_handler(
    _stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    serial_println!("[ok]");
    qemu::exit_qemu(qemu::QemuExitCode::Success);
}
