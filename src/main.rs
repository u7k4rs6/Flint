//! Flint: a bootable x86-64 kernel. `_start` is provided by the `bootloader`
//! crate's `entry_point!` macro, which hands us the CPU already in 64-bit
//! long mode with a stack, so the earliest Rust code we write is `kmain`.

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(flint::test_runner)]
#![reexport_test_harness_main = "test_main"]

use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;
use flint::serial_println;

entry_point!(kmain);

fn kmain(_boot_info: &'static BootInfo) -> ! {
    flint::init();

    serial_println!("Flint kernel booting...");
    serial_println!("Flint v{} -- boot OK", env!("CARGO_PKG_VERSION"));

    #[cfg(test)]
    test_main();

    flint::hlt_loop();
}

/// The panic handler for normal (non-test) boots: print to serial and halt.
/// A kernel panic is not recoverable, so this never returns.
#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("KERNEL PANIC: {}", info);
    flint::hlt_loop();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    flint::test_panic_handler(info)
}
