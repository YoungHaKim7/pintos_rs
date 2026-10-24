//! Port of `tests/threads/priority-change.c` — verifies that
//! lowering a thread's priority so that it is no longer the
//! highest-priority thread in the system causes it to yield
//! immediately.

use crate::threads::thread::{
    PRI_DEFAULT, THREAD_MLFQS, thread_create, thread_set_priority,
};

use crate::tests::msg;

pub unsafe fn test_priority_change() {
    unsafe {
        /* This test does not work with the MLFQS. */
        crate::ASSERT!(!THREAD_MLFQS);

        msg("Creating a high-priority thread 2.", &[]);
        thread_create(
            b"thread 2\0".as_ptr(),
            PRI_DEFAULT + 1,
            Some(changing_thread),
            core::ptr::null_mut(),
        );
        msg("Thread 2 should have just lowered its priority.", &[]);
        thread_set_priority(PRI_DEFAULT - 2);
        msg("Thread 2 should have just exited.", &[]);
    }
}

unsafe extern "C" fn changing_thread(_aux: *mut ()) {
    unsafe {
        msg("Thread 2 now lowering priority.", &[]);
        thread_set_priority(PRI_DEFAULT - 1);
        msg("Thread 2 exiting.", &[]);
    }
}
