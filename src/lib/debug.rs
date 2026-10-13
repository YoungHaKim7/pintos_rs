//! Port of `lib/kernel/debug.c` — kernel panic machinery.

use crate::lib::kernel::console::{console_panic, vprintf};
use crate::lib::stdio::Arg;
use crate::threads::init::power_off;
use crate::threads::init::power_off_when_done;
use crate::threads::interrupt::intr_disable;

/* Recursion level (0 = first panic). */
static mut LEVEL: i32 = 0;

/* Halts the OS, printing the source location plus a
user-specific message.  (C also printed the enclosing function
name; Rust has no stable equivalent, so file:line only.) */
#[unsafe(no_mangle)]
pub extern "C" fn debug_panic(file: &str, line: u32, message: &str, slots: &[Arg]) -> ! {
    intr_disable();
    console_panic();

    unsafe {
        let level = core::ptr::addr_of_mut!(LEVEL);
        *level += 1;
        if *level == 1 {
            vprintf(
                "Kernel PANIC at %s:%d: ",
                &[Arg::Str(file.as_bytes()), Arg::I(line as i64)],
            );
            vprintf(message, slots);
            vprintf("\n", &[]);

            debug_backtrace();
        } else if *level == 2 {
            vprintf(
                "Kernel PANIC recursion at %s:%d().\n",
                &[Arg::Str(file.as_bytes()), Arg::I(line as i64)],
            );
        }
        /* Level 3+: don't print anything: that's probably why we recursed. */
    }

    crate::devices::serial::serial_flush();
    if unsafe { *core::ptr::addr_of!(power_off_when_done) } {
        power_off();
    }
    loop {}
}

/* Prints the call stack: a list of return addresses, one in each
of the functions we are nested within.  `gdb` or `addr2line`
may be applied to kernel.elf to translate these. */
pub fn debug_backtrace() {
    static mut EXPLAINED: bool = false;

    unsafe {
        let mut frame: *mut *mut *mut u8;
        core::arch::asm!("mov {0}, rbp", out(reg) frame, options(nomem, nostack, preserves_flags));

        vprintf("Call stack:", &[]);
        while !frame.is_null() && !(*frame).is_null() {
            let ret = *frame.add(1);
            vprintf(" %p", &[Arg::P(ret as usize)]);
            frame = *frame as *mut *mut *mut u8;
        }
        vprintf(".\n", &[]);

        if !EXPLAINED {
            EXPLAINED = true;
            vprintf(
                "The `backtrace' program can make call stacks useful.\n\
                 Read \"Backtraces\" in the \"Debugging Tools\" chapter\n\
                 of the Pintos documentation for more information.\n",
                &[],
            );
        }
    }
}
