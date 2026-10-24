//! Port of `lib/kernel/console.c` — console output layer.
//!
//! The console fans every character out to both the VGA display and
//! the serial port.  M0 subset: the console lock is not yet present
//! (`use_console_lock` stays false, matching C's early-boot behavior
//! before `console_init`); lock discipline arrives with synch.c.

use crate::{
    devices::{serial::serial_putc, vga::vga_putc},
    lib::stdio::{self, Arg},
    threads::{
        interrupt::intr_context,
        synch::{Lock, lock_acquire, lock_held_by_current_thread, lock_init, lock_release},
    },
};

/* The console lock.
Both the vga and serial layers do their own locking, so it's
safe to call them at any time.
But this lock is useful to prevent simultaneous printf() calls
from mixing their output, which looks confusing. */
static mut CONSOLE_LOCK: Lock = Lock::EMPTY;

/* True in ordinary circumstances: we want to use the console
lock to avoid mixing output between threads, as explained
above.

False in early boot before the point that locks are functional
or the console lock has been initialized, or after a kernel
panics. */
static mut USE_CONSOLE_LOCK: bool = false;

/* It's possible, if you add enough debug output to Pintos, to
try to recursively grab console_lock from a single thread.  We
avoid the problem by simulating a recursive lock with a depth
counter. */
static mut CONSOLE_LOCK_DEPTH: i32 = 0;

/* Number of characters written to console. */
static mut WRITE_CNT: i64 = 0;

/* Enable console locking. */
pub fn console_init() {
    unsafe {
        lock_init(&raw mut CONSOLE_LOCK);
        USE_CONSOLE_LOCK = true;
    }
}

/* Notifies the console that a kernel panic is underway,
which warns it to avoid trying to take the console lock from
now on. */
pub fn console_panic() {
    unsafe {
        USE_CONSOLE_LOCK = false;
    }
}

/* Prints console statistics. */
pub fn console_print_stats() {
    unsafe {
        crate::printf!(
            "Console: %lld characters output\n",
            *core::ptr::addr_of_mut!(WRITE_CNT)
        );
    }
}

/* Acquires the console lock. */
fn acquire_console() {
    unsafe {
        if !intr_context() && USE_CONSOLE_LOCK {
            if lock_held_by_current_thread(&raw const CONSOLE_LOCK) {
                CONSOLE_LOCK_DEPTH += 1;
            } else {
                lock_acquire(&raw mut CONSOLE_LOCK);
            }
        }
    }
}

/* Releases the console lock. */
fn release_console() {
    unsafe {
        if !intr_context() && USE_CONSOLE_LOCK {
            if CONSOLE_LOCK_DEPTH > 0 {
                CONSOLE_LOCK_DEPTH -= 1;
            } else {
                lock_release(&raw mut CONSOLE_LOCK);
            }
        }
    }
}

/* Returns true if the current thread has the console lock,
false otherwise. */
fn console_locked_by_current_thread() -> bool {
    unsafe {
        intr_context()
            || !USE_CONSOLE_LOCK
            || lock_held_by_current_thread(&raw const CONSOLE_LOCK)
    }
}

/// The standard vprintf() function: writes formatted output to both
/// the VGA display and serial port.  Returns the character count.
pub fn vprintf(format: &str, slots: &[Arg]) -> i32 {
    let mut char_cnt: i32 = 0;

    acquire_console();
    stdio::__vprintf(
        format,
        slots,
        vprintf_helper,
        &mut char_cnt as *mut i32 as *mut (),
    );
    release_console();

    char_cnt
}

/* Writes string S to the console, followed by a new-line. */
pub fn puts(s: &[u8]) -> i32 {
    acquire_console();
    for &ch in s {
        putchar_have_lock(ch);
    }
    putchar_have_lock(b'\n');
    release_console();
    0
}

/* Writes the N characters in BUFFER to the console. */
pub fn putbuf(buffer: *const u8, n: usize) {
    acquire_console();
    unsafe {
        for i in 0..n {
            putchar_have_lock(*buffer.add(i));
        }
    }
    release_console();
}

/* Writes C to the vga display and serial port. */
pub fn putchar(c: u8) -> u8 {
    acquire_console();
    putchar_have_lock(c);
    release_console();
    c
}

/* Helper function for vprintf(). */
fn vprintf_helper(c: u8, char_cnt_: *mut ()) {
    let char_cnt = unsafe { &mut *(char_cnt_ as *mut i32) };
    *char_cnt += 1;
    putchar_have_lock(c);
}

/* Writes C to the vga display and serial port.
The caller has already acquired the console lock if
appropriate. */
fn putchar_have_lock(c: u8) {
    debug_assert!(console_locked_by_current_thread());
    unsafe {
        *core::ptr::addr_of_mut!(WRITE_CNT) += 1;
    }
    serial_putc(c);
    vga_putc(c);
}
