//! QEMU `isa-debug-exit` device: lets the kernel exit QEMU with a status code
//! so a test run has a pass/fail exit code instead of a hang.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

/// Writes `code` to the isa-debug-exit port, which halts QEMU with exit
/// status `(code << 1) | 1`.
pub fn exit_qemu(exit_code: QemuExitCode) -> ! {
    use x86_64::instructions::port::Port;

    // SAFETY: port 0xf4 is the isa-debug-exit iobase configured in the QEMU
    // runner args (Cargo.toml [package.metadata.bootimage]); it exists only
    // in our test/dev QEMU invocation and writing to it is defined to exit.
    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }

    // exit_qemu should have halted the VM; if we get here QEMU didn't have
    // the device configured. Loop rather than fall off into undefined state.
    loop {
        x86_64::instructions::hlt();
    }
}
