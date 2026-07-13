//! The IDT: exception handlers, the double-fault IST stack, and the two
//! hardware interrupts wired up so far (the PIT timer and the PS/2
//! keyboard). Set up right after output so faults become visible instead of
//! silent reboots.

use crate::gdt;
use crate::{serial_print, serial_println};
use core::sync::atomic::{AtomicU64, Ordering};
use lazy_static::lazy_static;
use pic8259::ChainedPics;
use spin::Mutex;
use x86_64::instructions::port::Port;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

/// The legacy PIC remaps IRQ0-15 onto interrupt vectors 32-47, clear of the
/// CPU exception vectors 0-31, so a hardware interrupt can never collide
/// with (and be misread as) a CPU exception.
pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard,
}

impl InterruptIndex {
    fn as_u8(self) -> u8 {
        self as u8
    }
}

pub static PICS: Mutex<ChainedPics> =
    // SAFETY: 32 and 40 are clear of the CPU exception range (0-31) and do
    // not overlap each other (8 vectors per PIC), which is ChainedPics::new's
    // documented safety requirement.
    unsafe { Mutex::new(ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET)) };

/// Timer ticks since boot, driven by IRQ0. `Relaxed` is enough: this is a
/// monotonically increasing counter with no other memory it needs to
/// synchronize with, only used for `uptime`/scheduling cadence.
pub static TICKS: AtomicU64 = AtomicU64::new(0);

pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        idt.page_fault.set_handler_fn(page_fault_handler);
        idt.general_protection_fault
            .set_handler_fn(general_protection_fault_handler);
        unsafe {
            // SAFETY: DOUBLE_FAULT_IST_INDEX names a stack that gdt::init
            // will have loaded into the TSS before interrupts are enabled
            // (flint::init calls gdt::init before interrupts::init_idt).
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        }
        idt[InterruptIndex::Timer.as_u8()].set_handler_fn(timer_interrupt_handler);
        idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(keyboard_interrupt_handler);
        idt
    };
}

pub fn init_idt() {
    IDT.load();
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    serial_println!("EXCEPTION: BREAKPOINT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    // A double fault means the first fault's handler itself faulted (most
    // often a kernel stack overflow re-faulting while the CPU tries to push
    // the exception frame). There is no safe way to continue, so report and
    // halt rather than attempt recovery on possibly-corrupt state.
    panic!("EXCEPTION: DOUBLE FAULT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let accessed_address = x86_64::registers::control::Cr2::read();

    if error_code.contains(PageFaultErrorCode::USER_MODE) {
        // A page fault from ring 3 is a user-process error, not a kernel
        // bug: report it and let the caller decide (kill/signal). No user
        // processes exist yet (M6), so this arm is exercised once syscalls
        // and user mode land.
        serial_println!(
            "USER PAGE FAULT at {:?}, error code: {:?}\n{:#?}",
            accessed_address,
            error_code,
            stack_frame
        );
        return;
    }

    panic!(
        "EXCEPTION: KERNEL PAGE FAULT\nAccessed Address: {:?}\nError Code: {:?}\n{:#?}",
        accessed_address, error_code, stack_frame
    );
}

extern "x86-interrupt" fn general_protection_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    panic!(
        "EXCEPTION: GENERAL PROTECTION FAULT (error code {:#x})\n{:#?}",
        error_code, stack_frame
    );
}

extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    TICKS.fetch_add(1, Ordering::Relaxed);

    // SAFETY: this vector is only ever reached via the PIC's timer IRQ, so
    // acknowledging exactly that vector is correct; the PIC requires an EOI
    // after every IRQ it delivers or it will stop delivering further ones.
    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
    }
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    use pc_keyboard::{layouts, DecodedKey, HandleControl, Keyboard, ScancodeSet1};

    lazy_static! {
        static ref KEYBOARD: Mutex<Keyboard<layouts::Us104Key, ScancodeSet1>> = Mutex::new(
            Keyboard::new(
                ScancodeSet1::new(),
                layouts::Us104Key,
                HandleControl::Ignore
            )
        );
    }

    let mut keyboard = KEYBOARD.lock();
    // SAFETY: 0x60 is the standard PS/2 data port; reading it here (only
    // from within this IRQ1 handler) is what acknowledges the keyboard
    // controller's byte to the device.
    let mut port = Port::new(0x60);
    let scancode: u8 = unsafe { port.read() };

    if let Ok(Some(key_event)) = keyboard.add_byte(scancode) {
        if let Some(key) = keyboard.process_keyevent(key_event) {
            match key {
                DecodedKey::Unicode(character) => serial_print!("{}", character),
                DecodedKey::RawKey(key) => serial_print!("{:?}", key),
            }
        }
    }

    // SAFETY: same reasoning as the timer handler -- this vector is only
    // reached via IRQ1, so acknowledging it is correct and required.
    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
    }
}

#[cfg(test)]
mod tests {
    #[test_case]
    fn test_breakpoint_exception() {
        x86_64::instructions::interrupts::int3();
    }
}
