//! Port of `tests/threads/priority-donate-multiple.c` — the main
//! thread acquires locks A and B, then it creates two
//! higher-priority threads.  Each of these threads blocks
//! acquiring one of the locks and thus donate their priority to
//! the main thread.  The main thread releases the locks in turn
//! and relinquishes its donated priorities.

use crate::lib::stdio::Arg;
use crate::threads::synch::{Lock, lock_acquire, lock_init, lock_release};
use crate::threads::thread::{PRI_DEFAULT, THREAD_MLFQS, thread_create, thread_get_priority};

use crate::tests::msg;

pub unsafe fn test_priority_donate_multiple() {
    unsafe {
        let mut a = Lock::EMPTY;
        let mut b = Lock::EMPTY;

        /* This test does not work with the MLFQS. */
        crate::ASSERT!(!THREAD_MLFQS);

        /* Make sure our priority is the default. */
        crate::ASSERT!(thread_get_priority() == PRI_DEFAULT);

        lock_init(&raw mut a);
        lock_init(&raw mut b);

        lock_acquire(&raw mut a);
        lock_acquire(&raw mut b);

        thread_create(
            b"a\0".as_ptr(),
            PRI_DEFAULT + 1,
            Some(a_thread_func),
            &raw mut a as *mut Lock as *mut (),
        );
        msg(
            "Main thread should have priority %d.  Actual priority: %d.",
            &[
                Arg::I((PRI_DEFAULT + 1) as i64),
                Arg::I(thread_get_priority() as i64),
            ],
        );

        thread_create(
            b"b\0".as_ptr(),
            PRI_DEFAULT + 2,
            Some(b_thread_func),
            &raw mut b as *mut Lock as *mut (),
        );
        msg(
            "Main thread should have priority %d.  Actual priority: %d.",
            &[
                Arg::I((PRI_DEFAULT + 2) as i64),
                Arg::I(thread_get_priority() as i64),
            ],
        );

        lock_release(&raw mut b);
        msg("Thread b should have just finished.", &[]);
        msg(
            "Main thread should have priority %d.  Actual priority: %d.",
            &[
                Arg::I((PRI_DEFAULT + 1) as i64),
                Arg::I(thread_get_priority() as i64),
            ],
        );

        lock_release(&raw mut a);
        msg("Thread a should have just finished.", &[]);
        msg(
            "Main thread should have priority %d.  Actual priority: %d.",
            &[
                Arg::I(PRI_DEFAULT as i64),
                Arg::I(thread_get_priority() as i64),
            ],
        );
    }
}

unsafe extern "C" fn a_thread_func(lock_: *mut ()) {
    unsafe {
        let lock = lock_ as *mut Lock;

        lock_acquire(lock);
        msg("Thread a acquired lock a.", &[]);
        lock_release(lock);
        msg("Thread a finished.", &[]);
    }
}

unsafe extern "C" fn b_thread_func(lock_: *mut ()) {
    unsafe {
        let lock = lock_ as *mut Lock;

        lock_acquire(lock);
        msg("Thread b acquired lock b.", &[]);
        lock_release(lock);
        msg("Thread b finished.", &[]);
    }
}
