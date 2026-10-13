//! pintos_rs — the KAIST Pintos x86-64 teaching kernel, ported to Rust.
//!
//! Boot chain: BIOS → `loader.S` (MBR) → `start.S` (32→64-bit) →
//! `main()` in `threads/init.rs`.  The port is deliberately faithful
//! to the C original: same module layout, same data structures,
//! same algorithms, same console output.

#![no_std]
#![no_main]
#![allow(static_mut_refs)]
#![allow(dead_code)]

mod devices;
mod filesys;
mod intrinsic;
mod lib;
mod tests;
mod threads;
mod userprog;
mod vm;

use core::panic::PanicInfo;

use lib::kernel::console::{console_panic, vprintf};
use lib::stdio::Arg;
use threads::interrupt::intr_disable;

/* printf-style output macros.
C's printf(fmt, ...) becomes printf!(fmt, args...), which routes
through the faithful __vprintf formatter. */

/// Formatted console output (port of `printf`).
#[macro_export]
macro_rules! printf {
    ($fmt:expr) => {
        $crate::lib::kernel::console::vprintf($fmt, &[])
    };
    ($fmt:expr, $($arg:expr),+ $(,)?) => {
        $crate::lib::kernel::console::vprintf($fmt, &[$($crate::lib::stdio::Arg::from($arg)),+])
    };
}

/// Fatal kernel panic (port of `PANIC`).
#[macro_export]
macro_rules! PANIC {
    ($fmt:expr) => {
        $crate::lib::debug::debug_panic(file!(), line!(), $fmt, &[])
    };
    ($fmt:expr, $($arg:expr),+ $(,)?) => {
        $crate::lib::debug::debug_panic(
            file!(),
            line!(),
            $fmt,
            &[$($crate::lib::stdio::Arg::from($arg)),+],
        )
    };
}

/// Assertion (port of `ASSERT`).
#[macro_export]
macro_rules! ASSERT {
    ($cond:expr) => {
        if !$cond {
            $crate::PANIC!(concat!("assertion `", stringify!($cond), "' failed."));
        }
    };
}

/* Kernel panic handler for Rust-level panics (out-of-bounds index,
unwrap on None, ...).  Routes through the same console machinery. */
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    intr_disable();
    console_panic();

    vprintf("Kernel PANIC (rust) ", &[]);
    if let Some(loc) = info.location() {
        vprintf(
            "at %s:%d: ",
            &[Arg::Str(loc.file().as_bytes()), Arg::I(loc.line() as i64)],
        );
    }
    /* Extract the panic payload as a &str when possible (panic!("msg")
    and assertion messages both arrive as &str). */
    let payload = info.payload();
    let msg: &str = if let Some(s) = (payload as &dyn core::any::Any).downcast_ref::<&str>() {
        s
    } else {
        "<non-string panic payload>"
    };
    vprintf("%s.\n", &[Arg::Str(msg.as_bytes())]);

    devices::serial::serial_flush();
    if unsafe { *core::ptr::addr_of!(threads::init::POWER_OFF_WHEN_DONE) } {
        threads::init::power_off();
    }
    loop {}
}
