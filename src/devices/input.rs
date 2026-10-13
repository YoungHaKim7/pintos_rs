//! Port of `devices/input.c` — keyboard/serial input buffer.

use crate::ASSERT;
use crate::devices::intq::{INTQ_BUFSIZE, Intq, intq_full, intq_getc, intq_init, intq_putc};
use crate::devices::serial::serial_notify;
use crate::threads::interrupt::{IntrLevel, intr_disable, intr_get_level, intr_set_level};

/* Stores keys from the keyboard and serial port. */
static mut BUFFER: Intq = Intq {
    buf: [0; INTQ_BUFSIZE],
    head: 0,
    tail: 0,
    not_empty: core::ptr::null_mut(),
    not_full: core::ptr::null_mut(),
};

/* Initializes the input buffer. */
pub fn input_init() {
    unsafe { intq_init(core::ptr::addr_of_mut!(BUFFER)) };
}

/* Adds a key to the input buffer.
Interrupts must be off and the buffer must not be full. */
pub fn input_putc(key: u8) {
    unsafe {
        ASSERT!(intr_get_level() == IntrLevel::Off);
        ASSERT!(!intq_full(core::ptr::addr_of!(BUFFER)));

        intq_putc(core::ptr::addr_of_mut!(BUFFER), key);
        serial_notify();
    }
}

/* Retrieves a key from the input buffer.
If the buffer is empty, waits for a key to be pressed. */
pub fn input_getc() -> u8 {
    let key: u8;

    let old_level = intr_disable();
    unsafe {
        key = intq_getc(core::ptr::addr_of_mut!(BUFFER));
        serial_notify();
    }
    intr_set_level(old_level);

    key
}

/* Returns true if the input buffer is full, false otherwise.
Interrupts must be off. */
pub fn input_full() -> bool {
    ASSERT!(intr_get_level() == IntrLevel::Off);
    unsafe { intq_full(core::ptr::addr_of!(BUFFER)) }
}
