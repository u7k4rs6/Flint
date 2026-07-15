//! COM1 serial output. The primary channel: boot log, panics, and (later) the shell.

use spin::Mutex;
use uart_16550::SerialPort;
use lazy_static::lazy_static;

lazy_static! {
    pub static ref SERIAL1: Mutex<SerialPort> = {
        // SAFETY: 0x3F8 is the standard COM1 I/O base on the PC platform QEMU emulates.
        // No other code touches this port range, so exclusive ownership holds.
        let mut serial_port = unsafe { SerialPort::new(0x3F8) };
        serial_port.init();
        Mutex::new(serial_port)
    };
}

#[doc(hidden)]
pub fn _print(args: core::fmt::Arguments) {
    use core::fmt::Write;
    use x86_64::instructions::interrupts;

    // Disable interrupts while holding the serial lock: a timer tick that
    // reenters and tries to print (e.g. from a panic in a handler) must not
    // deadlock on a lock this same core already holds.
    interrupts::without_interrupts(|| {
        SERIAL1
            .lock()
            .write_fmt(args)
            .expect("printing to serial failed");
    });
}

/// Prints to the host through the serial interface.
#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {
        $crate::serial::_print(format_args!($($arg)*))
    };
}

/// Prints to the host through the serial interface, appending a newline.
#[macro_export]
macro_rules! serial_println {
    () => ($crate::serial_print!("\n"));
    ($fmt:expr) => ($crate::serial_print!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => ($crate::serial_print!(
        concat!($fmt, "\n"), $($arg)*));
}

/// `[task N]`, or nothing before the scheduler exists -- shared by every
/// `log_*!` macro below so the format stays identical across levels
/// (Doc 4 section 2: "the current task id, so interleaved output is
/// attributable"). Not for use from code already holding the scheduler's
/// own lock (`task::scheduler::SCHEDULER`) -- like every other lock in
/// Flint, it is not reentrant.
#[doc(hidden)]
pub struct TaskTag;

impl core::fmt::Display for TaskTag {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match crate::task::scheduler::current_task_id() {
            Some(id) => write!(f, " [task {}]", id),
            None => Ok(()),
        }
    }
}

/// One structured log line per Doc 4 section 2: a level (`trace`/`debug`/
/// `info`/`warn`/`error`), the current task id where one exists, then the
/// message -- e.g. `[warn] [task 3] SYS_WRITE rejected: ...`. Kept as a
/// small hand-written macro set (matching this codebase's style elsewhere:
/// no `log` crate dependency) rather than the ad hoc `[user]`/`[syscall]`-
/// style prefixes call sites used before this existed.
#[macro_export]
macro_rules! log_line {
    ($level:expr, $($arg:tt)*) => {
        $crate::serial_println!(
            "[{}]{} {}",
            $level,
            $crate::serial::TaskTag,
            format_args!($($arg)*)
        )
    };
}

#[macro_export]
macro_rules! log_trace {
    ($($arg:tt)*) => { $crate::log_line!("trace", $($arg)*) };
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => { $crate::log_line!("debug", $($arg)*) };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => { $crate::log_line!("info", $($arg)*) };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => { $crate::log_line!("warn", $($arg)*) };
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => { $crate::log_line!("error", $($arg)*) };
}
