//! Required isolation gates (PRD definition of done / Doc 3 section 7):
//! a user-mode program performs a syscall and returns, with the effect
//! visible over serial; and a hostile user pointer is rejected while the
//! kernel survives. Both are exercised by one demo program
//! (`user::program::user_entry`): it issues a valid `SYS_WRITE` (its
//! effect -- "hi ring3!" -- lands on serial), then a `SYS_WRITE` with a
//! pointer into kernel memory it was never granted (rejected, logged,
//! kernel keeps running), then `SYS_EXIT`.
//!
//! Entering ring 3 is a one-way trip for this thread of control until a
//! syscall brings it back, so this test hooks `SYS_EXIT` (see
//! `syscall::set_exit_hook`) to report success and exit QEMU instead of
//! its normal `hlt_loop`. Reaching that hook at all already proves the
//! hostile write from moments earlier didn't take the kernel down.
//! Everything in between is on the record in the QEMU serial transcript.

#![no_std]
#![no_main]

use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;
use flint::{qemu, serial_println};

entry_point!(kmain);

fn kmain(boot_info: &'static BootInfo) -> ! {
    serial_println!("user_mode: booting");

    flint::init();
    flint::init_memory(boot_info);
    flint::syscall::set_exit_hook(on_user_exit);

    let (entry, stack_top) = flint::user::setup();
    serial_println!(
        "user_mode: entering ring 3 at {:?}, stack {:?}",
        entry,
        stack_top
    );

    // SAFETY: `flint::init` has already loaded the GDT/TSS (including
    // rsp0), and `flint::user::setup` just mapped `entry`/`stack_top` as
    // the user-accessible code/stack pages this function's contract
    // requires.
    unsafe { flint::user::enter_user_mode(entry, stack_top) };
}

fn on_user_exit() -> ! {
    serial_println!("user_mode: SYS_EXIT reached, kernel survived -- [ok]");
    qemu::exit_qemu(qemu::QemuExitCode::Success);
}

/// A panic here can only mean a fault (page fault, GPF, double fault) tore
/// through the ring-3 demo -- treat it as the isolation gate failing.
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("[failed] kernel panicked during the user-mode demo: {}", info);
    qemu::exit_qemu(qemu::QemuExitCode::Failed);
}
