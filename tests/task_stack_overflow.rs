//! Required isolation gate (Doc 3 section 3 / section 7 checklist): kernel
//! task stacks are bounded by an unmapped guard page, so an overflow faults
//! instead of silently corrupting neighboring memory. Before this existed,
//! `Task::new` backed each kernel stack with a plain heap `Box<[u8]>` --
//! overflowing it would have silently corrupted adjacent heap memory rather
//! than faulting at all.
//!
//! This spawns a task whose entry point recurses forever and lets the
//! *real* kernel handle whatever fault results (unlike `stack_overflow.rs`,
//! which installs its own minimal test-local IDT and drives the overflow
//! directly on the boot thread -- this test needs the full kernel running,
//! including the timer and scheduler, to actually switch into the task in
//! the first place, so it reuses the real `interrupts::double_fault_handler`
//! rather than replacing the IDT). Pushing the double-fault exception frame
//! itself happens on the now-overflowed stack, so it re-faults and
//! escalates to a double fault exactly the way a boot-stack overflow does
//! (Doc 3 section 5) -- the same general, stack-independent IST mechanism
//! `stack_overflow.rs` already proves, just reached by an overflowing task
//! stack instead of the boot stack. The only way this test's `kmain` can
//! reach its panic handler at all is via that double fault, since nothing
//! else in this narrow path panics -- reaching it (not a hang, not a
//! silent QEMU reset) is the success condition.

#![no_std]
#![no_main]

use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;
use flint::{qemu, serial_print, serial_println, task};

entry_point!(kmain);

fn kmain(boot_info: &'static BootInfo) -> ! {
    serial_print!("task_stack_overflow::task_stack_overflow...\t");

    flint::init();
    flint::init_memory(boot_info);
    flint::init_scheduler();

    task::scheduler::spawn("overflower", overflow_task);

    // Nothing else is spawned, so the very first timer tick switches
    // straight into `overflow_task` and never returns here. If the guard
    // page didn't work, this would hang (silent corruption, no fault)
    // instead of ever reaching the panic handler below -- caught by the
    // harness's own QEMU timeout as an honest failure, not a false pass.
    flint::hlt_loop();
}

#[allow(unconditional_recursion)]
extern "C" fn overflow_task() -> ! {
    recurse();
    flint::hlt_loop();
}

#[allow(unconditional_recursion)]
fn recurse() -> u64 {
    let r = recurse();
    // Stops the compiler from turning this into a tail call (which would
    // never grow the stack and never fault), same as stack_overflow.rs.
    volatile::Volatile::new(r).read()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    serial_println!("[ok]");
    qemu::exit_qemu(qemu::QemuExitCode::Success);
}
