//! Port of `tests/threads/priority-donate-one.c` — the main
//! thread acquires a lock.  Then it creates two higher-priority
//! threads that block acquiring the lock, causing them to donate
//! their priorities to the main thread.  When the main thread
//! releases the lock, the other threads should acquire it in
//! priority order.

use crate::lib::stdio::Arg;
use crate::threads::synch::{Lock, lock_acquire, lock_init, lock_release};
use crate::threads::thread::{PRI_DEFAULT, THREAD_MLFQS, thread_create, thread_get_priority};

use crate::tests::msg;

pub unsafe fn test_priority_donate_one() {
    unsafe {
        let mut lock = Lock::EMPTY;

        /* This test does not work with the MLFQS. */
        crate::ASSERT!(!THREAD_MLFQS);

        /* Make sure our priority is the default. */
        crate::ASSERT!(thread_get_priority() == PRI_DEFAULT);

        lock_init(&raw mut lock);
        lock_acquire(&raw mut lock);
        thread_create(
            b"acquire1\0".as_ptr(),
            PRI_DEFAULT + 1,
            Some(acquire1_thread_func),
            &raw mut lock as *mut Lock as *mut (),
        );
        msg(
            "This thread should have priority %d.  Actual priority: %d.",
            &[
                Arg::I((PRI_DEFAULT + 1) as i64),
                Arg::I(thread_get_priority() as i64),
            ],
        );
        thread_create(
            b"acquire2\0".as_ptr(),
            PRI_DEFAULT + 2,
            Some(acquire2_thread_func),
            &raw mut lock as *mut Lock as *mut (),
        );
        msg(
            "This thread should have priority %d.  Actual priority: %d.",
            &[
                Arg::I((PRI_DEFAULT + 2) as i64),
                Arg::I(thread_get_priority() as i64),
            ],
        );
        lock_release(&raw mut lock);
        msg("acquire2, acquire1 must already have finished, in that order.", &[]);
        msg("This should be the last line before finishing this test.", &[]);
    }
}

unsafe extern "C" fn acquire1_thread_func(lock_: *mut ()) {
    unsafe {
        let lock = lock_ as *mut Lock;

        lock_acquire(lock);
        msg("acquire1: got the lock", &[]);
        lock_release(lock);
        msg("acquire1: done", &[]);
    }
}

unsafe extern "C" fn acquire2_thread_func(lock_: *mut ()) {
    unsafe {
        let lock = lock_ as *mut Lock;

        lock_acquire(lock);
        msg("acquire2: got the lock", &[]);
        lock_release(lock);
        msg("acquire2: done", &[]);
    }
}
