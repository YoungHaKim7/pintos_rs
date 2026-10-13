//! Port of `devices/intq.c` and `include/devices/intq.h`.
//!
//! An "interrupt queue", a circular buffer shared between kernel
//! threads and external interrupt handlers.
//!
//! Interrupt queue functions can be called from kernel threads or
//! from external interrupt handlers.  Except for `intq_init()`,
//! interrupts must be off in either case.
//!
//! The interrupt queue has the structure of a "monitor".  Locks and
//! condition variables from `threads/synch.rs` cannot be used in
//! this case, as they normally would, because they can only protect
//! kernel threads from one another, not from interrupt handlers.

use crate::ASSERT;
use crate::threads::interrupt::{IntrLevel, intr_context, intr_get_level};
use crate::threads::thread::{Thread, thread_block, thread_current, thread_unblock};

/* Queue buffer size, in bytes. */
pub const INTQ_BUFSIZE: usize = 64;

/* A circular queue of bytes.  The C original also carried a
`struct lock lock` ("only one thread may wait at once"); this
port's struct, per the port plan, tracks only the raw waiter
pointers. */
#[repr(C)]
pub struct Intq {
    /* Queue. */
    pub buf: [u8; INTQ_BUFSIZE], /* Buffer. */
    pub head: usize,             /* New data is written here. */
    pub tail: usize,             /* Old data is read here. */

    /* Waiting threads. */
    pub not_empty: *mut Thread, /* Thread waiting for not-empty condition. */
    pub not_full: *mut Thread,  /* Thread waiting for not-full condition. */
}

/* Initializes interrupt queue Q. */
pub unsafe fn intq_init(q: *mut Intq) {
    unsafe {
        (*q).not_full = core::ptr::null_mut();
        (*q).not_empty = core::ptr::null_mut();
        (*q).head = 0;
        (*q).tail = 0;
    }
}

/* Returns true if Q is empty, false otherwise. */
pub unsafe fn intq_empty(q: *const Intq) -> bool {
    ASSERT!(intr_get_level() == IntrLevel::Off);
    unsafe { (*q).head == (*q).tail }
}

/* Returns true if Q is full, false otherwise. */
pub unsafe fn intq_full(q: *const Intq) -> bool {
    ASSERT!(intr_get_level() == IntrLevel::Off);
    unsafe { next((*q).head) == (*q).tail }
}

/* Removes a byte from Q and returns it.
Q must not be empty if called from an interrupt handler.
Otherwise, if Q is empty, first sleeps until a byte is
added. */
pub unsafe fn intq_getc(q: *mut Intq) -> u8 {
    unsafe {
        ASSERT!(intr_get_level() == IntrLevel::Off);
        while intq_empty(q) {
            ASSERT!(!intr_context());
            intq_no_empty(q);
        }

        let byte = (*q).buf[(*q).tail];
        (*q).tail = next((*q).tail);
        signal(q, core::ptr::addr_of_mut!((*q).not_full));
        byte
    }
}

/* Adds BYTE to the end of Q.
Q must not be full if called from an interrupt handler.
Otherwise, if Q is full, first sleeps until a byte is
removed. */
pub unsafe fn intq_putc(q: *mut Intq, byte: u8) {
    unsafe {
        ASSERT!(intr_get_level() == IntrLevel::Off);
        while intq_full(q) {
            ASSERT!(!intr_context());
            intq_no_full(q);
        }

        (*q).buf[(*q).head] = byte;
        (*q).head = next((*q).head);
        signal(q, core::ptr::addr_of_mut!((*q).not_empty));
    }
}

/* Returns the position after POS within an intq. */
fn next(pos: usize) -> usize {
    (pos + 1) % INTQ_BUFSIZE
}

/* Waits (blocks the current thread) until Q is not empty.  Q must
be empty.  The C original is the static helper `wait` specialized
to Q's not_empty member; the C caller also held Q's `lock` around
the wait, which this port's Intq does without. */
pub unsafe fn intq_no_empty(q: *mut Intq) {
    unsafe {
        ASSERT!(!intr_context());
        ASSERT!(intr_get_level() == IntrLevel::Off);
        ASSERT!(intq_empty(q));

        (*q).not_empty = thread_current();
        thread_block();
    }
}

/* Waits (blocks the current thread) until Q is not full.  Q must
be full.  The C original is the static helper `wait` specialized
to Q's not_full member. */
pub unsafe fn intq_no_full(q: *mut Intq) {
    unsafe {
        ASSERT!(!intr_context());
        ASSERT!(intr_get_level() == IntrLevel::Off);
        ASSERT!(intq_full(q));

        (*q).not_full = thread_current();
        thread_block();
    }
}

/* WAITER must be the address of Q's not_empty or not_full member,
and the associated condition must be true.  If a thread is waiting
for the condition, wakes it up and resets the waiting thread. */
unsafe fn signal(q: *mut Intq, waiter: *mut *mut Thread) {
    unsafe {
        ASSERT!(intr_get_level() == IntrLevel::Off);
        ASSERT!(
            (waiter == core::ptr::addr_of_mut!((*q).not_empty) && !intq_empty(q))
                || (waiter == core::ptr::addr_of_mut!((*q).not_full) && !intq_full(q))
        );

        if !(*waiter).is_null() {
            thread_unblock(*waiter);
            *waiter = core::ptr::null_mut();
        }
    }
}
