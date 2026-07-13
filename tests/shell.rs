//! The M7 gate: "the shell echoes input and runs help and one status
//! command" (PRD definition of done). Boots straight into the ring-3
//! shell loop (`user::shell_program`), with `SYS_READ_LINE` fed a scripted
//! byte sequence (`help`, `meminfo`, `exit`) via
//! `syscall::set_scripted_input` instead of the real UART, so this test
//! is self-contained and deterministic under a bare `cargo test` -- no
//! external stdin has to reach this exact QEMU process for the harness to
//! pass. `exit` is what ends the run: the shell loop issues `SYS_EXIT`
//! itself once `SYS_SHELL_DISPATCH` reports the line was `exit`/`quit`,
//! which this test hooks to report success.
//!
//! The scripted path exercises the identical ring-3/syscall/validate/echo
//! pipeline real typed input uses -- only the byte *source* differs. Real
//! interactive serial input was also manually verified end to end (piping
//! bytes into `cargo test --test shell`'s own stdin does reach the UART,
//! since `-serial stdio` inherits it through `bootimage runner`'s and
//! `cargo test`'s child processes); see PROGRESS.md for that transcript
//! and the exact command, kept as a manual demonstration rather than a
//! harness dependency for the reason above.

#![no_std]
#![no_main]

use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;
use flint::{qemu, serial_println};

entry_point!(kmain);

fn kmain(boot_info: &'static BootInfo) -> ! {
    serial_println!("shell: booting");

    flint::init();
    flint::init_memory(boot_info);
    flint::syscall::set_exit_hook(on_shell_exit);
    flint::syscall::set_scripted_input(b"help\nmeminfo\nexit\n");

    let (entry, stack_top) = flint::user::setup_shell();
    serial_println!("shell: ready, dispatching scripted input");

    // SAFETY: `flint::init` has already loaded the GDT/TSS (including
    // rsp0), and `flint::user::setup_shell` just mapped `entry`/
    // `stack_top` (and the line buffer) as the user-accessible pages this
    // function's contract requires.
    unsafe { flint::user::enter_user_mode(entry, stack_top) };
}

fn on_shell_exit() -> ! {
    serial_println!("shell: exited cleanly -- [ok]");
    qemu::exit_qemu(qemu::QemuExitCode::Success);
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("[failed] kernel panicked during the shell demo: {}", info);
    qemu::exit_qemu(qemu::QemuExitCode::Failed);
}
