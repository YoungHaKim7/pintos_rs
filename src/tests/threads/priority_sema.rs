//! Port of `tests/threads/priority-sema.c` — tests that the
//! highest-priority thread waiting on a semaphore is the first to
//! wake up.

use crate::lib::stdio::Arg;
use crate::threads::synch::{Semaphore, sema_down, sema_init, sema_up};
use crate::threads::thread::{
    PRI_DEFAULT, PRI_MIN, THREAD_MLFQS, thread_create, thread_name, thread_set_priority,
};

use super::snprintf;
use crate::tests::msg;

static mut SEMA: Semaphore = Semaphore::EMPTY;

pub unsafe fn test_priority_sema() {
    unsafe {
        /* This test does not work with the MLFQS. */
        crate::ASSERT!(!THREAD_MLFQS);

        sema_init(&raw mut SEMA, 0);
        thread_set_priority(PRI_MIN);
        for i in 0..10 {
            let priority = PRI_DEFAULT - (i + 3) % 10 - 1;
            let mut name = [0u8; 16];
            snprintf(&mut name, "priority %d", &[Arg::I(priority as i64)]);
            thread_create(
                name.as_ptr(),
                priority,
                Some(priority_sema_thread),
                core::ptr::null_mut(),
            );
        }

        for _ in 0..10 {
            sema_up(&raw mut SEMA);
            msg("Back in main thread.", &[]);
        }
    }
}

unsafe extern "C" fn priority_sema_thread(_aux: *mut ()) {
    unsafe {
        sema_down(&raw mut SEMA);
        msg("Thread %s woke up.", &[Arg::Raw(thread_name())]);
    }
}
