//! Port of `tests/threads/priority-donate-lower.c` — the main
//! thread acquires a lock.  Then it creates a higher-priority
//! thread that blocks acquiring the lock, causing it to donate
//! their priorities to the main thread.  The main thread attempts
//! to lower its priority, which should not take effect until the
//! donation is released.

use crate::lib::stdio::Arg;
use crate::threads::synch::{Lock, lock_acquire, lock_init, lock_release};
use crate::threads::thread::{
    PRI_DEFAULT, THREAD_MLFQS, thread_create, thread_get_priority, thread_set_priority,
};

use crate::tests::msg;

pub unsafe fn test_priority_donate_lower() {
    unsafe {
        let mut lock = Lock::EMPTY;

        /* This test does not work with the MLFQS. */
        crate::ASSERT!(!THREAD_MLFQS);

        /* Make sure our priority is the default. */
        crate::ASSERT!(thread_get_priority() == PRI_DEFAULT);

        lock_init(&raw mut lock);
        lock_acquire(&raw mut lock);
        thread_create(
            b"acquire\0".as_ptr(),
            PRI_DEFAULT + 10,
            Some(acquire_thread_func),
            &raw mut lock as *mut Lock as *mut (),
        );
        msg(
            "Main thread should have priority %d.  Actual priority: %d.",
            &[
                Arg::I((PRI_DEFAULT + 10) as i64),
                Arg::I(thread_get_priority() as i64),
            ],
        );

        msg("Lowering base priority...", &[]);
        thread_set_priority(PRI_DEFAULT - 10);
        msg(
            "Main thread should have priority %d.  Actual priority: %d.",
            &[
                Arg::I((PRI_DEFAULT + 10) as i64),
                Arg::I(thread_get_priority() as i64),
            ],
        );
        lock_release(&raw mut lock);
        msg("acquire must already have finished.", &[]);
        msg(
            "Main thread should have priority %d.  Actual priority: %d.",
            &[
                Arg::I((PRI_DEFAULT - 10) as i64),
                Arg::I(thread_get_priority() as i64),
            ],
        );
    }
}

unsafe extern "C" fn acquire_thread_func(lock_: *mut ()) {
    unsafe {
        let lock = lock_ as *mut Lock;

        lock_acquire(lock);
        msg("acquire: got the lock", &[]);
        lock_release(lock);
        msg("acquire: done", &[]);
    }
}
