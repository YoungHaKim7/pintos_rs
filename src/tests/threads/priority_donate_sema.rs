//! Port of `tests/threads/priority-donate-sema.c` — low priority
//! thread L acquires a lock, then blocks downing a semaphore.
//! Medium priority thread M then blocks waiting on the same
//! semaphore.  Next, high priority thread H attempts to acquire
//! the lock, donating its priority to L.
//!
//! Next, the main thread ups the semaphore, waking up L.  L
//! releases the lock, which wakes up H.  H "up"s the semaphore,
//! waking up M.  H terminates, then M, then L, and finally the
//! main thread.

use crate::threads::synch::{
    Lock, Semaphore, lock_acquire, lock_init, lock_release, sema_down, sema_init, sema_up,
};
use crate::threads::thread::{PRI_DEFAULT, THREAD_MLFQS, thread_create, thread_get_priority};

use crate::tests::msg;

#[repr(C)]
struct LockAndSema {
    lock: Lock,
    sema: Semaphore,
}

pub unsafe fn test_priority_donate_sema() {
    unsafe {
        /* This test does not work with the MLFQS. */
        crate::ASSERT!(!THREAD_MLFQS);

        /* Make sure our priority is the default. */
        crate::ASSERT!(thread_get_priority() == PRI_DEFAULT);

        let mut ls = LockAndSema {
            lock: Lock::EMPTY,
            sema: Semaphore::EMPTY,
        };
        lock_init(&raw mut ls.lock);
        sema_init(&raw mut ls.sema, 0);
        thread_create(
            b"low\0".as_ptr(),
            PRI_DEFAULT + 1,
            Some(l_thread_func),
            &raw mut ls as *mut LockAndSema as *mut (),
        );
        thread_create(
            b"med\0".as_ptr(),
            PRI_DEFAULT + 3,
            Some(m_thread_func),
            &raw mut ls as *mut LockAndSema as *mut (),
        );
        thread_create(
            b"high\0".as_ptr(),
            PRI_DEFAULT + 5,
            Some(h_thread_func),
            &raw mut ls as *mut LockAndSema as *mut (),
        );
        sema_up(&raw mut ls.sema);
        msg("Main thread finished.", &[]);
    }
}

unsafe extern "C" fn l_thread_func(ls_: *mut ()) {
    unsafe {
        let ls = ls_ as *mut LockAndSema;

        lock_acquire(&raw mut (*ls).lock);
        msg("Thread L acquired lock.", &[]);
        sema_down(&raw mut (*ls).sema);
        msg("Thread L downed semaphore.", &[]);
        lock_release(&raw mut (*ls).lock);
        msg("Thread L finished.", &[]);
    }
}

unsafe extern "C" fn m_thread_func(ls_: *mut ()) {
    unsafe {
        let ls = ls_ as *mut LockAndSema;

        sema_down(&raw mut (*ls).sema);
        msg("Thread M finished.", &[]);
    }
}

unsafe extern "C" fn h_thread_func(ls_: *mut ()) {
    unsafe {
        let ls = ls_ as *mut LockAndSema;

        lock_acquire(&raw mut (*ls).lock);
        msg("Thread H acquired lock.", &[]);

        sema_up(&raw mut (*ls).sema);
        lock_release(&raw mut (*ls).lock);
        msg("Thread H finished.", &[]);
    }
}
