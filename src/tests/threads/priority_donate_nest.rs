//! Port of `tests/threads/priority-donate-nest.c` — low-priority
//! main thread L acquires lock A.  Medium-priority thread M then
//! acquires lock B then blocks on acquiring lock A.
//! High-priority thread H then blocks on acquiring lock B.  Thus,
//! thread H donates its priority to M, which in turn donates it
//! to thread L.

use crate::lib::stdio::Arg;
use crate::threads::synch::{Lock, lock_acquire, lock_init, lock_release};
use crate::threads::thread::{
    PRI_DEFAULT, THREAD_MLFQS, thread_create, thread_get_priority, thread_yield,
};

use crate::tests::msg;

#[repr(C)]
struct Locks {
    a: *mut Lock,
    b: *mut Lock,
}

pub unsafe fn test_priority_donate_nest() {
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

        let mut locks = Locks {
            a: &raw mut a,
            b: &raw mut b,
        };
        thread_create(
            b"medium\0".as_ptr(),
            PRI_DEFAULT + 1,
            Some(medium_thread_func),
            &raw mut locks as *mut Locks as *mut (),
        );
        thread_yield();
        msg(
            "Low thread should have priority %d.  Actual priority: %d.",
            &[
                Arg::I((PRI_DEFAULT + 1) as i64),
                Arg::I(thread_get_priority() as i64),
            ],
        );

        thread_create(
            b"high\0".as_ptr(),
            PRI_DEFAULT + 2,
            Some(high_thread_func),
            &raw mut b as *mut Lock as *mut (),
        );
        thread_yield();
        msg(
            "Low thread should have priority %d.  Actual priority: %d.",
            &[
                Arg::I((PRI_DEFAULT + 2) as i64),
                Arg::I(thread_get_priority() as i64),
            ],
        );

        lock_release(&raw mut a);
        thread_yield();
        msg("Medium thread should just have finished.", &[]);
        msg(
            "Low thread should have priority %d.  Actual priority: %d.",
            &[
                Arg::I(PRI_DEFAULT as i64),
                Arg::I(thread_get_priority() as i64),
            ],
        );
    }
}

unsafe extern "C" fn medium_thread_func(locks_: *mut ()) {
    unsafe {
        let locks = locks_ as *mut Locks;

        lock_acquire((*locks).b);
        lock_acquire((*locks).a);

        msg(
            "Medium thread should have priority %d.  Actual priority: %d.",
            &[
                Arg::I((PRI_DEFAULT + 2) as i64),
                Arg::I(thread_get_priority() as i64),
            ],
        );
        msg("Medium thread got the lock.", &[]);

        lock_release((*locks).a);
        thread_yield();

        lock_release((*locks).b);
        thread_yield();

        msg("High thread should have just finished.", &[]);
        msg("Middle thread finished.", &[]);
    }
}

unsafe extern "C" fn high_thread_func(lock_: *mut ()) {
    unsafe {
        let lock = lock_ as *mut Lock;

        lock_acquire(lock);
        msg("High thread got the lock.", &[]);
        lock_release(lock);
        msg("High thread finished.", &[]);
    }
}
