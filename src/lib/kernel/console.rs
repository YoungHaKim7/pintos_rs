//! Port of `lib/kernel/console.c` — console output layer.
//!
//! The console fans every character out to both the VGA display and
//! the serial port.  M0 subset: the console lock is not yet present
//! (`use_console_lock` stays false, matching C's early-boot behavior
//! before `console_init`); lock discipline arrives with synch.c.

use crate::devices::serial::serial_putc;
use crate::devices::vga::vga_putc;
use crate::lib::stdio::{self, Arg};
use crate::threads::interrupt::intr_context;

/* True in ordinary circumstances: we want to use the console
lock to avoid mixing output between threads.  False in early
boot or after a kernel panic. */
static mut USE_CONSOLE_LOCK: bool = false;

/* Number of characters written to console. */
static mut WRITE_CNT: i64 = 0;

/* Enable console locking.  (M1: initializes the console lock.) */
pub fn console_init() {
    // M0: locks unavailable; C leaves use_console_lock false until here.
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

/* Acquires the console lock.  M0: no-op (use_console_lock false). */
fn acquire_console() {
    debug_assert!(!intr_context());
}

/* Releases the console lock.  M0: no-op. */
fn release_console() {
    debug_assert!(!intr_context());
}

/* Returns true if the current thread "has" the console lock,
which is trivially so when no locking is in use. */
fn console_locked_by_current_thread() -> bool {
    intr_context() || unsafe { !USE_CONSOLE_LOCK }
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
