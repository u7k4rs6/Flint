//! The VGA text buffer: the secondary output channel (Doc 2 section 3,
//! Doc 4 section 2). Serial is the workhorse and stays the only channel
//! most call sites use; this exists to mirror the *key* output (the boot
//! banner, panics) onto the emulated screen too, per Doc 2 section 3
//! ("useful for a visible banner and panic, but serial is the workhorse").
//!
//! The buffer lives at physical address 0xb8000 (25 rows x 80 columns, 2
//! bytes per cell: an ASCII byte and a color-attribute byte) -- ordinary
//! physical memory the bootloader's default video-mode stub (see
//! `bootloader`'s `video_mode/vga_text_80x25.s`) sets the CPU into before
//! Rust ever runs. Reached through the same physical-memory offset mapping
//! every other physical-address access in Flint already uses
//! (`memory::paging::physical_memory_offset`), not a raw identity-mapped
//! pointer -- Flint makes no assumption that low physical addresses are
//! ever identity-mapped into the kernel's own virtual address space.

use spin::Mutex;
use volatile::Volatile;
use x86_64::VirtAddr;

const BUFFER_HEIGHT: usize = 25;
const BUFFER_WIDTH: usize = 80;
const VGA_PHYS_ADDR: u64 = 0xb8000;

/// Light gray on black -- a plain, legible default; Doc 2 section 3 asks
/// for a "visible banner and panic," not a themed display.
const DEFAULT_COLOR: u8 = 0x07;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
struct ScreenChar(u16);

impl ScreenChar {
    fn new(ascii: u8, color: u8) -> ScreenChar {
        ScreenChar((color as u16) << 8 | ascii as u16)
    }
}

#[repr(transparent)]
struct Buffer {
    chars: [[Volatile<ScreenChar>; BUFFER_WIDTH]; BUFFER_HEIGHT],
}

struct Writer {
    column: usize,
    buffer: &'static mut Buffer,
}

impl Writer {
    fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.new_line(),
            byte => {
                if self.column >= BUFFER_WIDTH {
                    self.new_line();
                }
                let row = BUFFER_HEIGHT - 1;
                let col = self.column;
                // Printable ASCII only; anything else (non-ASCII UTF-8
                // continuation bytes, control characters besides '\n')
                // becomes the VGA text mode's own "unprintable" glyph
                // (0xfe, a filled square) rather than garbage.
                let printable = matches!(byte, 0x20..=0x7e);
                let out = if printable { byte } else { 0xfe };
                self.buffer.chars[row][col].write(ScreenChar::new(out, DEFAULT_COLOR));
                self.column += 1;
            }
        }
    }

    fn write_str_raw(&mut self, s: &str) {
        for byte in s.bytes() {
            self.write_byte(byte);
        }
    }

    fn new_line(&mut self) {
        for row in 1..BUFFER_HEIGHT {
            for col in 0..BUFFER_WIDTH {
                let moved = self.buffer.chars[row][col].read();
                self.buffer.chars[row - 1][col].write(moved);
            }
        }
        self.clear_row(BUFFER_HEIGHT - 1);
        self.column = 0;
    }

    fn clear_row(&mut self, row: usize) {
        let blank = ScreenChar::new(b' ', DEFAULT_COLOR);
        for col in 0..BUFFER_WIDTH {
            self.buffer.chars[row][col].write(blank);
        }
    }
}

impl core::fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.write_str_raw(s);
        Ok(())
    }
}

static WRITER: Mutex<Option<Writer>> = Mutex::new(None);

/// Brings up VGA text output.
///
/// # Safety
/// Must be called exactly once, after `memory::paging::init` has
/// established the physical-memory offset mapping this reaches 0xb8000
/// through (`crate::init_memory` calls this immediately after
/// `memory::init`), and before any code calls `vga_print!`/`vga_println!`.
pub unsafe fn init() {
    let offset: VirtAddr = crate::memory::paging::physical_memory_offset();
    let ptr: *mut Buffer = (offset + VGA_PHYS_ADDR).as_mut_ptr();
    // SAFETY: `offset` is the complete physical-memory offset mapping
    // (forwarded from the caller's contract above), so `offset + 0xb8000`
    // is valid, mapped, read-write memory -- the VGA text buffer's own 4000
    // bytes (25 x 80 x 2), which nothing else in the kernel ever writes to.
    let buffer = unsafe { &mut *ptr };
    let mut writer = Writer { column: 0, buffer };
    for row in 0..BUFFER_HEIGHT {
        writer.clear_row(row);
    }
    *WRITER.lock() = Some(writer);
}

#[doc(hidden)]
pub fn _print(args: core::fmt::Arguments) {
    use core::fmt::Write;
    use x86_64::instructions::interrupts;

    // Same discipline as `serial::_print`: hold the lock with interrupts
    // off, so a timer tick that reenters (e.g. printing from a panic
    // inside a handler) can't deadlock on a lock this same core holds.
    interrupts::without_interrupts(|| {
        // A no-op, not a panic, if `init` hasn't run yet (or this is a
        // `cfg(test)` build that never calls it) -- VGA is the *secondary*
        // channel; its absence must never take down serial, the primary
        // one, which is what actually carries every test assertion.
        if let Some(writer) = WRITER.lock().as_mut() {
            let _ = writer.write_fmt(args);
        }
    });
}

/// Prints to the VGA text buffer.
#[macro_export]
macro_rules! vga_print {
    ($($arg:tt)*) => {
        $crate::vga::_print(format_args!($($arg)*))
    };
}

/// Prints to the VGA text buffer, appending a newline.
#[macro_export]
macro_rules! vga_println {
    () => ($crate::vga_print!("\n"));
    ($fmt:expr) => ($crate::vga_print!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => ($crate::vga_print!(
        concat!($fmt, "\n"), $($arg)*));
}

#[cfg(test)]
mod tests {
    use super::WRITER;

    /// Proves the mechanism actually writes through to the buffer and back,
    /// not just that `vga_print!` compiles and doesn't fault: writes a
    /// known string, then reads the exact same cells back out of the
    /// buffer's last row and checks each character round-tripped.
    #[test_case]
    fn printed_text_round_trips_through_the_buffer() {
        let s = "flint vga ok";
        crate::vga_println!("{}", s);

        let mut guard = WRITER.lock();
        let writer = guard.as_mut().expect("vga::init must have run");
        let row = super::BUFFER_HEIGHT - 1;
        // `vga_println!` just moved to a fresh (blank) last row, so the
        // string we printed is one row up from wherever the cursor is now.
        for (col, expected) in s.bytes().enumerate() {
            let cell = writer.buffer.chars[row - 1][col].read();
            assert_eq!(cell.0 as u8, expected);
        }
    }
}
