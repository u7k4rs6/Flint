//! Flint kernel library. Shared by the `flint` binary and by the in-kernel
//! integration tests under `tests/`, so both boot through the same `init`
//! path and the same panic/test machinery.

#![no_std]
#![cfg_attr(test, no_main)]
#![feature(custom_test_frameworks)]
#![feature(abi_x86_interrupt)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

pub mod serial;
pub mod qemu;
pub mod gdt;
pub mod interrupts;
pub mod memory;
pub mod task;
pub mod syscall;
pub mod user;

use core::panic::PanicInfo;

/// Bring up the parts of the kernel every entry point needs: GDT/TSS so the
/// double-fault IST stack exists, then the IDT, then unmask the PIC and
/// enable interrupts. Physical memory and paging are brought up separately
/// via `init_memory`, which needs the boot info the bootloader handed the
/// caller.
pub fn init() {
    gdt::init();
    interrupts::init_idt();
    // SAFETY: init_idt() has just installed handlers for every vector the
    // PIC can raise (timer, keyboard), so it is safe to start delivering
    // IRQs; PICS.lock().initialize() must run before interrupts::enable()
    // or an IRQ could arrive before the PIC's own remap is complete.
    unsafe { interrupts::PICS.lock().initialize() };
    interrupts::init_pit(100); // 100 Hz: a 10ms scheduling quantum
    x86_64::instructions::interrupts::enable();
}

/// Brings up the physical frame allocator from the bootloader's memory map.
/// Separate from `init` because it needs the `BootInfo` the bootloader
/// handed the entry point, which `init` (called identically by every entry
/// point, test or not) does not take.
pub fn init_memory(boot_info: &'static bootloader::BootInfo) {
    // SAFETY: `boot_info` is the genuine value the bootloader passed to
    // this entry point via `entry_point!`, and this runs once during boot
    // before anything could have allocated a frame.
    unsafe { memory::init(&boot_info.memory_map, boot_info.physical_memory_offset) };
}

/// Starts the scheduler with an empty ready queue. Separate from `init`
/// because spawning even the placeholder boot task allocates (`Box`), so
/// this must run after `init_memory` has brought up the heap.
pub fn init_scheduler() {
    task::scheduler::init();
}

/// Spin forever with `hlt` between iterations instead of busy-looping, so an
/// idle kernel is not pegging the (virtual) CPU.
pub fn hlt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

// ===== Test framework =====
//
// A kernel is tested by booting it and observing behavior. Each #[test_case]
// function runs in-kernel, reports over serial, and the harness exits QEMU
// via isa-debug-exit with a pass/fail status. This is what makes `cargo
// test` meaningful for a no_std kernel: compiling is not the same as
// booting, and booting is not the same as passing.

pub trait Testable {
    fn run(&self) -> ();
}

impl<T> Testable for T
where
    T: Fn(),
{
    fn run(&self) {
        serial_print!("{}...\t", core::any::type_name::<T>());
        self();
        serial_println!("[ok]");
    }
}

pub fn test_runner(tests: &[&dyn Testable]) {
    serial_println!("Running {} tests", tests.len());
    for test in tests {
        test.run();
    }
    qemu::exit_qemu(qemu::QemuExitCode::Success);
}

pub fn test_panic_handler(info: &PanicInfo) -> ! {
    serial_println!("[failed]\n");
    serial_println!("Error: {}\n", info);
    qemu::exit_qemu(qemu::QemuExitCode::Failed);
}

/// Entry point for `cargo test --lib`, i.e. tests compiled into this crate
/// itself (as opposed to the integration tests under `tests/`).
#[cfg(test)]
use bootloader::{entry_point, BootInfo};

#[cfg(test)]
entry_point!(test_kernel_main);

#[cfg(test)]
fn test_kernel_main(boot_info: &'static BootInfo) -> ! {
    init();
    init_memory(boot_info);
    init_scheduler();
    test_main();
    hlt_loop();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    test_panic_handler(info)
}

#[cfg(test)]
#[test_case]
fn trivial_assertion() {
    assert_eq!(1, 1);
}
