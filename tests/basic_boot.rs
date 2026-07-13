//! Boot-and-assert integration test: the kernel boots to `kmain`-equivalent
//! code and can print over serial. This is the milestone gate for M1 -- if
//! this test does not pass, the kernel does not boot, no matter what else
//! compiles.

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
    test_main();
    flint::hlt_loop();
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    flint::test_panic_handler(info)
}

#[test_case]
fn test_boots_and_prints() {
    serial_println!("basic_boot: kernel reached test_main over serial");
}
