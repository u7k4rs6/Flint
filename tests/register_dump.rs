//! Doc 4 section 5's register-dump gate: a panic report includes "the
//! general-purpose registers ... the instruction pointer, the stack
//! pointer, and the relevant control registers." `interrupts.rs`'s
//! GPR-capture trampolines (`page_fault_entry` and friends) exist
//! specifically so a kernel-mode fault's panic message reflects genuine
//! register state at the moment of the fault, not zeros or whatever the
//! panic machinery's own bookkeeping happened to leave behind.
//!
//! This loads a known marker value into `rax` immediately before
//! dereferencing a deliberately unmapped, non-lazy-region address -- a
//! genuine, unrecoverable kernel-mode page fault, caught by the real
//! `page_fault_handler` (through the real trampoline, not a test-local
//! IDT) -- and asserts the resulting panic message's `rax=` field actually
//! contains that marker, proving the dump is real, not a stub.

#![no_std]
#![no_main]

extern crate alloc;

use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;
use flint::{qemu, serial_print, serial_println};

entry_point!(kmain);

const MARKER: u64 = 0xdead_beef_1234_5678;

fn kmain(boot_info: &'static BootInfo) -> ! {
    serial_print!("register_dump::register_dump...\t");

    flint::init();
    flint::init_memory(boot_info);

    // SAFETY: `0x_1234_5678_0000` is a canonical (bit 47 clear, bits 48-63
    // clear to match) but deliberately unmapped address, outside the
    // kernel heap, the lazy demand-paging region, and every other range
    // this kernel ever maps -- a genuine not-present kernel-mode page
    // fault, not (as a non-canonical address would trigger) a general
    // protection fault. This write is the deliberate trigger; `rax`
    // carries the marker the panic handler's `GprDump` must reflect.
    unsafe {
        core::arch::asm!(
            "mov rax, {marker}",
            "mov qword ptr [{addr}], rax",
            marker = const MARKER,
            addr = in(reg) 0x_1234_5678_0000u64,
            out("rax") _,
        );
    }

    unreachable!("the deliberate fault above should have panicked before this point");
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let message = alloc::format!("{}", info);
    serial_println!("{}", message);

    // `{:#018x}` of MARKER formats as "0xdeadbeef12345678"; matching the
    // hex digits without the `0x` prefix is robust to exact width/padding.
    // Also confirms this is genuinely the page-fault panic path (Doc 4
    // section 5's "the faulting address and the error code, for a page
    // fault"), not some other exception.
    if message.contains("deadbeef12345678") && message.contains("KERNEL PAGE FAULT") {
        serial_println!("[ok]");
        qemu::exit_qemu(qemu::QemuExitCode::Success);
    }

    serial_println!("[failed] panic report did not contain the expected register marker");
    qemu::exit_qemu(qemu::QemuExitCode::Failed);
}
