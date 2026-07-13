//! The shell's command set: `help`, `echo`, `meminfo`, `ps`, and
//! `ticks`/`uptime` (Doc 2 section 8's exact list). Kept as a pure
//! function from a line of text to a response string, deliberately
//! independent of serial I/O or the ring-3/syscall path, so it is testable
//! directly (`#[test_case]`) without booting into ring 3 -- what actually
//! needs the isolation machinery from M6 is getting the line *in* and the
//! response back *out* across the syscall boundary (`src/syscall/mod.rs`
//! `SYS_READ_LINE` / `SYS_SHELL_DISPATCH`), not the command logic itself.
//!
//! Doc 2 section 8: "The shell is a user-space process, not a kernel
//! feature." The process loop and the decision to keep reading and
//! dispatching lines lives in ring 3 (`user::shell_program`, a
//! hand-written asm loop, since Flint has no ELF loader yet -- M8
//! stretch). Parsing and formatting the response happens here, in the
//! kernel, behind the `SYS_SHELL_DISPATCH` syscall the ring-3 loop calls
//! -- the same shape as a real OS's libc wrapping raw syscalls, just with
//! the split drawn one layer further in because hand-encoding a string
//! parser directly in assembly was judged not worth the risk this late in
//! the build (see DECISIONS.md).

use alloc::string::String;
use alloc::format;

pub fn dispatch(line: &str) -> String {
    let line = line.trim();
    if line.is_empty() {
        return String::new();
    }

    let (cmd, rest) = match line.split_once(' ') {
        Some((cmd, rest)) => (cmd, rest),
        None => (line, ""),
    };

    match cmd {
        "help" => String::from(
            "commands: help, echo <text>, meminfo, ps, ticks (or uptime)",
        ),
        "echo" => String::from(rest),
        "meminfo" => crate::memory::with_frame_allocator(|fa| {
            let free = fa.frames_free();
            let total = fa.frames_total();
            let used = total - free;
            format!(
                "frames: {} used, {} free, {} total (4 KiB each) | heap: {} KiB",
                used,
                free,
                total,
                crate::memory::heap::HEAP_SIZE / 1024
            )
        }),
        "ps" => format!(
            "pid 1: shell (ring 3) | scheduler: {} context switches since boot",
            crate::task::scheduler::switch_count()
        ),
        "ticks" | "uptime" => {
            format!("{} ticks since boot", crate::interrupts::ticks())
        }
        _ => format!("unknown command: {}", cmd),
    }
}

#[cfg(test)]
mod tests {
    use super::dispatch;

    #[test_case]
    fn help_lists_commands() {
        let out = dispatch("help");
        assert!(out.contains("echo"));
        assert!(out.contains("meminfo"));
        assert!(out.contains("ps"));
        assert!(out.contains("ticks"));
    }

    #[test_case]
    fn echo_returns_its_argument() {
        assert_eq!(dispatch("echo hello world"), "hello world");
    }

    #[test_case]
    fn echo_with_no_argument_is_empty() {
        assert_eq!(dispatch("echo"), "");
    }

    #[test_case]
    fn meminfo_reports_frame_counts() {
        let out = dispatch("meminfo");
        assert!(out.contains("free"));
        assert!(out.contains("total"));
    }

    #[test_case]
    fn ps_reports_a_status_line() {
        let out = dispatch("ps");
        assert!(out.contains("shell"));
    }

    #[test_case]
    fn ticks_and_uptime_are_synonyms() {
        let a = dispatch("ticks");
        let b = dispatch("uptime");
        assert!(a.contains("ticks since boot"));
        assert!(b.contains("ticks since boot"));
    }

    #[test_case]
    fn unknown_command_says_so() {
        assert!(dispatch("bogus").contains("unknown command"));
    }

    #[test_case]
    fn blank_line_is_a_no_op() {
        assert_eq!(dispatch(""), "");
        assert_eq!(dispatch("   "), "");
    }
}
