//! The IDT: exception handlers, the double-fault IST stack, and the two
//! hardware interrupts wired up so far (the PIT timer and the PS/2
//! keyboard). Set up right after output so faults become visible instead of
//! silent reboots.

use crate::gdt;
use crate::{log_warn, serial_print, serial_println};
use core::arch::naked_asm;
use core::sync::atomic::{AtomicU64, Ordering};
use lazy_static::lazy_static;
use pic8259::ChainedPics;
use spin::Mutex;
use x86_64::instructions::port::Port;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};
use x86_64::VirtAddr;

/// The legacy PIC remaps IRQ0-15 onto interrupt vectors 32-47, clear of the
/// CPU exception vectors 0-31, so a hardware interrupt can never collide
/// with (and be misread as) a CPU exception.
pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard,
}

impl InterruptIndex {
    fn as_u8(self) -> u8 {
        self as u8
    }
}

pub static PICS: Mutex<ChainedPics> =
    // SAFETY: 32 and 40 are clear of the CPU exception range (0-31) and do
    // not overlap each other (8 vectors per PIC), which is ChainedPics::new's
    // documented safety requirement.
    unsafe { Mutex::new(ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET)) };

/// Timer ticks since boot, driven by IRQ0. `Relaxed` is enough: this is a
/// monotonically increasing counter with no other memory it needs to
/// synchronize with, only used for `uptime`/scheduling cadence.
pub static TICKS: AtomicU64 = AtomicU64::new(0);

pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        unsafe {
            // SAFETY: each `*_entry` trampoline captures the GPRs via a
            // fixed sequence of `mov [addr], reg` instructions -- no push,
            // no stack or flags touched -- then `jmp`s directly to the real
            // handler's entry point with the CPU-pushed exception frame
            // completely untouched, so the real handler's own compiler-
            // generated frame/error-code parsing sees exactly the state it
            // would have seen had the CPU jumped to it directly (what
            // `set_handler_fn` would otherwise have wired up). See the
            // `gpr_capture_trampoline!` definitions below.
            idt.page_fault
                .set_handler_addr(VirtAddr::new(page_fault_entry as *const () as u64));
            idt.general_protection_fault.set_handler_addr(VirtAddr::new(
                general_protection_fault_entry as *const () as u64,
            ));
            // DOUBLE_FAULT_IST_INDEX names a stack that gdt::init will have
            // loaded into the TSS before interrupts are enabled (flint::init
            // calls gdt::init before interrupts::init_idt); the CPU switches
            // to it automatically before even reaching the trampoline's
            // first instruction, same as it always did for the plain
            // `set_handler_fn` registration this replaces.
            idt.double_fault
                .set_handler_addr(VirtAddr::new(double_fault_entry as *const () as u64))
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        }
        idt[InterruptIndex::Timer.as_u8()].set_handler_fn(timer_interrupt_handler);
        idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(keyboard_interrupt_handler);
        crate::syscall::register(&mut idt);
        idt
    };
}

pub fn init_idt() {
    IDT.load();
}

const PIT_FREQUENCY_HZ: u32 = 1_193_182;

/// Reprograms PIT channel 0 (the source of IRQ0) from its BIOS default of
/// ~18.2 Hz to `hz`, so the scheduler has a known, reasonably fine-grained
/// tick period instead of whatever the firmware happened to leave it at.
pub fn init_pit(hz: u32) {
    let divisor = (PIT_FREQUENCY_HZ / hz) as u16;
    let mut command: Port<u8> = Port::new(0x43);
    let mut channel0: Port<u8> = Port::new(0x40);
    // SAFETY: 0x43/0x40 are the standard 8253/8254 PIT command and
    // channel-0 data ports; this sequence (command byte, then low byte,
    // then high byte of the reload value) is the documented way to
    // reprogram channel 0's rate, and nothing else in the kernel touches
    // these ports.
    unsafe {
        command.write(0x36u8); // channel 0, lobyte/hibyte access, mode 3 (square wave)
        channel0.write((divisor & 0xff) as u8);
        channel0.write((divisor >> 8) as u8);
    }
}

// ===== Register dump on panic (Doc 4 section 5) =====
//
// `extern "x86-interrupt" fn` bodies cannot see the general-purpose
// registers at all -- LLVM's x86-interrupt calling convention saves and
// restores whatever it clobbers in a hidden prologue/epilogue never exposed
// to the handler body, confirmed against the vendored `x86_64` 0.15.5
// source. Genuinely capturing them means intercepting *before* that
// prologue runs, which needs a naked stub. Rather than hand-parse the
// CPU-pushed exception frame ourselves in that stub (real risk of getting
// the error-code/frame layout subtly wrong, exactly the kind of mistake
// this codebase's own docs warn corrupts state silently), each trampolines
// only captures GPRs -- via plain `mov [addr], reg` writes that touch no
// stack or flags state -- into these statics, then `jmp`s straight into the
// existing, unchanged, already-correct `extern "x86-interrupt" fn` handler.
// That handler's own compiler-generated frame/error-code parsing is
// untouched and still does all the real work; it just also has these
// statics available when it builds its panic report.
static F_RAX: AtomicU64 = AtomicU64::new(0);
static F_RBX: AtomicU64 = AtomicU64::new(0);
static F_RCX: AtomicU64 = AtomicU64::new(0);
static F_RDX: AtomicU64 = AtomicU64::new(0);
static F_RSI: AtomicU64 = AtomicU64::new(0);
static F_RDI: AtomicU64 = AtomicU64::new(0);
static F_RBP: AtomicU64 = AtomicU64::new(0);
static F_R8: AtomicU64 = AtomicU64::new(0);
static F_R9: AtomicU64 = AtomicU64::new(0);
static F_R10: AtomicU64 = AtomicU64::new(0);
static F_R11: AtomicU64 = AtomicU64::new(0);
static F_R12: AtomicU64 = AtomicU64::new(0);
static F_R13: AtomicU64 = AtomicU64::new(0);
static F_R14: AtomicU64 = AtomicU64::new(0);
static F_R15: AtomicU64 = AtomicU64::new(0);

/// The general-purpose registers as they were at the moment of the last
/// fault one of the trampolines below caught, formatted for a panic report.
struct GprDump;

impl core::fmt::Display for GprDump {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "rax={:#018x} rbx={:#018x} rcx={:#018x} rdx={:#018x}\n         rsi={:#018x} rdi={:#018x} rbp={:#018x}\n         r8={:#018x} r9={:#018x} r10={:#018x} r11={:#018x}\n         r12={:#018x} r13={:#018x} r14={:#018x} r15={:#018x}",
            F_RAX.load(Ordering::Relaxed),
            F_RBX.load(Ordering::Relaxed),
            F_RCX.load(Ordering::Relaxed),
            F_RDX.load(Ordering::Relaxed),
            F_RSI.load(Ordering::Relaxed),
            F_RDI.load(Ordering::Relaxed),
            F_RBP.load(Ordering::Relaxed),
            F_R8.load(Ordering::Relaxed),
            F_R9.load(Ordering::Relaxed),
            F_R10.load(Ordering::Relaxed),
            F_R11.load(Ordering::Relaxed),
            F_R12.load(Ordering::Relaxed),
            F_R13.load(Ordering::Relaxed),
            F_R14.load(Ordering::Relaxed),
            F_R15.load(Ordering::Relaxed),
        )
    }
}

/// Defines a naked trampoline that captures GPRs into the statics above,
/// then `jmp`s to `$handler`'s real entry point. One shared template
/// instantiated three times below (`page_fault_entry`,
/// `general_protection_fault_entry`, `double_fault_entry`) rather than
/// hand-copied, so a typo in the capture sequence can't silently diverge
/// between them.
macro_rules! gpr_capture_trampoline {
    ($name:ident, $handler:ident) => {
        #[unsafe(naked)]
        unsafe extern "C" fn $name() {
            naked_asm!(
                "mov [{f_rax}], rax",
                "mov [{f_rbx}], rbx",
                "mov [{f_rcx}], rcx",
                "mov [{f_rdx}], rdx",
                "mov [{f_rsi}], rsi",
                "mov [{f_rdi}], rdi",
                "mov [{f_rbp}], rbp",
                "mov [{f_r8}], r8",
                "mov [{f_r9}], r9",
                "mov [{f_r10}], r10",
                "mov [{f_r11}], r11",
                "mov [{f_r12}], r12",
                "mov [{f_r13}], r13",
                "mov [{f_r14}], r14",
                "mov [{f_r15}], r15",
                "jmp {handler}",
                f_rax = sym F_RAX,
                f_rbx = sym F_RBX,
                f_rcx = sym F_RCX,
                f_rdx = sym F_RDX,
                f_rsi = sym F_RSI,
                f_rdi = sym F_RDI,
                f_rbp = sym F_RBP,
                f_r8 = sym F_R8,
                f_r9 = sym F_R9,
                f_r10 = sym F_R10,
                f_r11 = sym F_R11,
                f_r12 = sym F_R12,
                f_r13 = sym F_R13,
                f_r14 = sym F_R14,
                f_r15 = sym F_R15,
                handler = sym $handler,
            )
        }
    };
}

gpr_capture_trampoline!(page_fault_entry, page_fault_handler);
gpr_capture_trampoline!(
    general_protection_fault_entry,
    general_protection_fault_handler
);
gpr_capture_trampoline!(double_fault_entry, double_fault_handler);

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    serial_println!("EXCEPTION: BREAKPOINT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    // A double fault means the first fault's handler itself faulted (most
    // often a kernel stack overflow re-faulting while the CPU tries to push
    // the exception frame). There is no safe way to continue, so report and
    // halt rather than attempt recovery on possibly-corrupt state.
    panic!(
        "EXCEPTION: DOUBLE FAULT\ntask: {:?}\n{}\n{:#?}",
        crate::task::scheduler::current_task_id(),
        GprDump,
        stack_frame
    );
}

extern "x86-interrupt" fn page_fault_handler(
    mut stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let accessed_address =
        x86_64::registers::control::Cr2::read().expect("CR2 held a non-canonical address");

    if error_code.contains(PageFaultErrorCode::USER_MODE) {
        // A page fault from ring 3 is a user-process error, not a kernel
        // bug: report it and let the caller decide (kill/signal). No user
        // processes exist yet (M6), so this arm is exercised once syscalls
        // and user mode land.
        log_warn!(
            "USER PAGE FAULT at {:?}, error code: {:?}\n{:#?}",
            accessed_address,
            error_code,
            stack_frame
        );
        return;
    }

    // A not-present fault (no PROTECTION_VIOLATION bit) inside the lazy
    // region is demand paging working as designed (Doc 2 section 5.2): map
    // a fresh frame and let the faulting instruction re-run. Anything else
    // -- a protection violation, or a not-present fault outside the lazy
    // region -- is a genuine kernel bug and must panic loudly rather than
    // paper over corrupt control flow.
    if !error_code.contains(PageFaultErrorCode::PROTECTION_VIOLATION)
        && crate::memory::paging::is_lazy_region(accessed_address)
    {
        if crate::memory::paging::demand_page(accessed_address).is_ok() {
            return;
        }
    }

    // A fault inside a syscall's fault-safe user copy (Doc 3 sections 4, 7):
    // `copy_from_user_byte`/`copy_to_user_byte` recorded exactly where to
    // resume before making the access that just faulted. Redirect there
    // instead of panicking -- the copy helper turns this into a plain
    // `Err`, so a syscall whose argument was valid at validation time but
    // got unmapped before the copy fails cleanly rather than taking the
    // kernel down with it.
    if let Some(recovery_ip) = crate::syscall::take_copy_recovery_ip() {
        crate::syscall::mark_copy_faulted();
        // SAFETY: `recovery_ip` is a real code address inside this binary
        // (the fallthrough label immediately after the risky access in
        // `copy_from_user_byte`/`copy_to_user_byte`, computed by `lea`
        // right before that access ran), never a value derived from
        // anything user-controlled -- rewriting the saved `rip` to it just
        // resumes execution one instruction later than where it faulted.
        unsafe {
            stack_frame.as_mut().update(|f| {
                f.instruction_pointer = x86_64::VirtAddr::new(recovery_ip);
            });
        }
        return;
    }

    panic!(
        "EXCEPTION: KERNEL PAGE FAULT\nAccessed Address: {:?}\nError Code: {:?}\ntask: {:?}\n{}\n{:#?}",
        accessed_address,
        error_code,
        crate::task::scheduler::current_task_id(),
        GprDump,
        stack_frame
    );
}

extern "x86-interrupt" fn general_protection_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    panic!(
        "EXCEPTION: GENERAL PROTECTION FAULT (error code {:#x})\ntask: {:?}\n{}\n{:#?}",
        error_code,
        crate::task::scheduler::current_task_id(),
        GprDump,
        stack_frame
    );
}

extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    TICKS.fetch_add(1, Ordering::Relaxed);

    // The EOI must go out *before* a possible context switch below: a
    // switch may not return here for a long time (not until this exact
    // task is scheduled again), and the PIC will not deliver another IRQ0
    // (or anything at equal/lower priority) until it sees the EOI for this
    // one.
    //
    // SAFETY: this vector is only ever reached via the PIC's timer IRQ, so
    // acknowledging exactly that vector is correct.
    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
    }

    crate::task::scheduler::timer_tick();
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    use pc_keyboard::{layouts, DecodedKey, HandleControl, Keyboard, ScancodeSet1};

    lazy_static! {
        static ref KEYBOARD: Mutex<Keyboard<layouts::Us104Key, ScancodeSet1>> = Mutex::new(
            Keyboard::new(
                ScancodeSet1::new(),
                layouts::Us104Key,
                HandleControl::Ignore
            )
        );
    }

    let mut keyboard = KEYBOARD.lock();
    // SAFETY: 0x60 is the standard PS/2 data port; reading it here (only
    // from within this IRQ1 handler) is what acknowledges the keyboard
    // controller's byte to the device.
    let mut port = Port::new(0x60);
    let scancode: u8 = unsafe { port.read() };

    if let Ok(Some(key_event)) = keyboard.add_byte(scancode) {
        if let Some(key) = keyboard.process_keyevent(key_event) {
            match key {
                DecodedKey::Unicode(character) => serial_print!("{}", character),
                DecodedKey::RawKey(key) => serial_print!("{:?}", key),
            }
        }
    }

    // SAFETY: same reasoning as the timer handler -- this vector is only
    // reached via IRQ1, so acknowledging it is correct and required.
    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
    }
}

#[cfg(test)]
mod tests {
    #[test_case]
    fn test_breakpoint_exception() {
        x86_64::instructions::interrupts::int3();
    }
}
