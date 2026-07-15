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
use flint::{serial_println, vga_println};

entry_point!(kmain);

fn kmain(boot_info: &'static BootInfo) -> ! {
    flint::init();
    flint::init_memory(boot_info);
    flint::init_scheduler();

    // The boot banner is "key output" (Doc 2 section 3) -- mirrored to VGA,
    // not just serial, so a developer with only the emulated screen visible
    // (no serial capture) still sees the kernel came up.
    serial_println!("Flint kernel booting...");
    vga_println!("Flint kernel booting...");
    serial_println!("Flint v{} -- boot OK", env!("CARGO_PKG_VERSION"));
    vga_println!("Flint v{} -- boot OK", env!("CARGO_PKG_VERSION"));

    #[cfg(test)]
    test_main();

    let (entry, stack_top) = flint::user::setup_shell();
    serial_println!("Flint: dropping to ring 3, starting the shell (type 'help')");
    vga_println!("Flint: dropping to ring 3, starting the shell (type 'help')");

    // SAFETY: flint::init() has already loaded the GDT/TSS (including
    // rsp0), and flint::user::setup_shell() just mapped entry/stack_top
    // (and the shell's line buffer) as the user-accessible pages this
    // function's contract requires.
    unsafe { flint::user::enter_user_mode(entry, stack_top) };
}

/// The panic handler for normal (non-test) boots: print to serial and halt.
/// A kernel panic is not recoverable, so this never returns.
///
/// Register-dump note (Doc 4 section 5): the naked capture trampolines in
/// `interrupts.rs` give an exact, at-fault GPR snapshot for the exception
/// handlers that go through them (a kernel-mode page fault, a general
/// protection fault, a double fault). A *plain* `panic!()` reaching this
/// handler has no hardware trap frame behind it at all -- there is nothing
/// analogous to intercept before a prologue runs, because there is no
/// prologue, just ordinary Rust control flow that already ran through
/// `PanicInfo` formatting before this function's first instruction. The
/// four registers read below are a best-effort approximation only: their
/// values *at the moment this handler started running*, not at the
/// original `panic!()` call site, since intervening code (formatting the
/// message, reaching this function) may already have reused them.
#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let rax: u64;
    let rbx: u64;
    let rcx: u64;
    let rdx: u64;
    // SAFETY: four independent, single-operand reads of live register
    // values into locals -- no memory access, no stack, no flags touched.
    unsafe {
        core::arch::asm!("mov {0}, rax", out(reg) rax);
        core::arch::asm!("mov {0}, rbx", out(reg) rbx);
        core::arch::asm!("mov {0}, rcx", out(reg) rcx);
        core::arch::asm!("mov {0}, rdx", out(reg) rdx);
    }
    serial_println!(
        "KERNEL PANIC: {}\n(best-effort, panic-handler-entry registers, not the original panic!() site: rax={:#018x} rbx={:#018x} rcx={:#018x} rdx={:#018x})",
        info, rax, rbx, rcx, rdx
    );
    vga_println!("KERNEL PANIC: {}", info);
    flint::hlt_loop();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    flint::test_panic_handler(info)
}
