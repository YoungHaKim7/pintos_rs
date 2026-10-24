//! Port of `tests/threads/priority-donate-chain.c` — the main
//! thread set its priority to PRI_MIN and creates 7 threads
//! (thread 1..7) with priorities PRI_MIN + 3, 6, 9, 12, ...
//! The main thread initializes 8 locks: lock 0..7 and acquires
//! lock 0.
//!
//! When thread[i] starts, it first acquires lock[i] (unless
//! i == 7.)  Subsequently, thread[i] attempts to acquire
//! lock[i-1], which is held by thread[i-1], except for lock[0],
//! which is held by the main thread.  Because the lock is held,
//! thread[i] donates its priority to thread[i-1], which donates
//! to thread[i-2], and so on until the main thread receives the
//! donation.

use crate::lib::stdio::Arg;
use crate::threads::synch::{Lock, lock_acquire, lock_init, lock_release};
use crate::threads::thread::{
    PRI_MIN, THREAD_MLFQS, thread_create, thread_get_priority, thread_name,
    thread_set_priority,
};

use super::snprintf;
use crate::tests::msg;

const NESTING_DEPTH: usize = 8;

#[repr(C)]
#[derive(Clone, Copy)]
struct LockPair {
    second: *mut Lock,
    first: *mut Lock,
}

pub unsafe fn test_priority_donate_chain() {
    unsafe {
        let mut locks = [Lock::EMPTY; NESTING_DEPTH - 1];
        let mut lock_pairs: [LockPair; NESTING_DEPTH] = [LockPair {
            second: core::ptr::null_mut(),
            first: core::ptr::null_mut(),
        }; NESTING_DEPTH];

        /* This test does not work with the MLFQS. */
        crate::ASSERT!(!THREAD_MLFQS);

        thread_set_priority(PRI_MIN);

        for i in 0..NESTING_DEPTH - 1 {
            lock_init(&raw mut locks[i]);
        }

        lock_acquire(&raw mut locks[0]);
        msg("%s got lock.", &[Arg::Raw(thread_name())]);

        for i in 1..NESTING_DEPTH {
            let mut name = [0u8; 16];
            let thread_priority = PRI_MIN + i as i32 * 3;
            lock_pairs[i].first = if i < NESTING_DEPTH - 1 {
                &raw mut locks[i]
            } else {
                core::ptr::null_mut()
            };
            lock_pairs[i].second = &raw mut locks[i - 1];

            snprintf(&mut name, "thread %d", &[Arg::I(i as i64)]);
            thread_create(
                name.as_ptr(),
                thread_priority,
                Some(donor_thread_func),
                &raw mut lock_pairs[i] as *mut LockPair as *mut (),
            );
            msg(
                "%s should have priority %d.  Actual priority: %d.",
                &[
                    Arg::Raw(thread_name()),
                    Arg::I(thread_priority as i64),
                    Arg::I(thread_get_priority() as i64),
                ],
            );

            snprintf(&mut name, "interloper %d", &[Arg::I(i as i64)]);
            thread_create(
                name.as_ptr(),
                thread_priority - 1,
                Some(interloper_thread_func),
                core::ptr::null_mut(),
            );
        }

        lock_release(&raw mut locks[0]);
        msg(
            "%s finishing with priority %d.",
            &[
                Arg::Raw(thread_name()),
                Arg::I(thread_get_priority() as i64),
            ],
        );
    }
}

unsafe extern "C" fn donor_thread_func(locks_: *mut ()) {
    unsafe {
        let locks = locks_ as *mut LockPair;

        if !(*locks).first.is_null() {
            lock_acquire((*locks).first);
        }

        lock_acquire((*locks).second);
        msg("%s got lock", &[Arg::Raw(thread_name())]);

        lock_release((*locks).second);
        msg(
            "%s should have priority %d. Actual priority: %d",
            &[
                Arg::Raw(thread_name()),
                Arg::I(((NESTING_DEPTH - 1) * 3) as i64),
                Arg::I(thread_get_priority() as i64),
            ],
        );

        if !(*locks).first.is_null() {
            lock_release((*locks).first);
        }

        msg(
            "%s finishing with priority %d.",
            &[
                Arg::Raw(thread_name()),
                Arg::I(thread_get_priority() as i64),
            ],
        );
    }
}

unsafe extern "C" fn interloper_thread_func(_arg: *mut ()) {
    unsafe {
        msg("%s finished.", &[Arg::Raw(thread_name())]);
    }
}
