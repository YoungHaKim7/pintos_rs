//! Port of `tests/threads/priority-condvar.c` — tests that
//! cond_signal() wakes up the highest-priority thread waiting in
//! cond_wait().

use crate::lib::stdio::Arg;
use crate::threads::synch::{
    Condition, Lock, cond_init, cond_signal, cond_wait, lock_acquire, lock_init, lock_release,
};
use crate::threads::thread::{
    PRI_DEFAULT, PRI_MIN, THREAD_MLFQS, thread_create, thread_name, thread_set_priority,
};

use super::snprintf;
use crate::tests::msg;

static mut LOCK: Lock = Lock::EMPTY;
static mut CONDITION: Condition = Condition::EMPTY;

pub unsafe fn test_priority_condvar() {
    unsafe {
        /* This test does not work with the MLFQS. */
        crate::ASSERT!(!THREAD_MLFQS);

        lock_init(&raw mut LOCK);
        cond_init(&raw mut CONDITION);

        thread_set_priority(PRI_MIN);
        for i in 0..10 {
            let priority = PRI_DEFAULT - (i + 7) % 10 - 1;
            let mut name = [0u8; 16];
            snprintf(&mut name, "priority %d", &[Arg::I(priority as i64)]);
            thread_create(
                name.as_ptr(),
                priority,
                Some(priority_condvar_thread),
                core::ptr::null_mut(),
            );
        }

        for _ in 0..10 {
            lock_acquire(&raw mut LOCK);
            msg("Signaling...", &[]);
            cond_signal(&raw mut CONDITION, &raw mut LOCK);
            lock_release(&raw mut LOCK);
        }
    }
}

unsafe extern "C" fn priority_condvar_thread(_aux: *mut ()) {
    unsafe {
        msg("Thread %s starting.", &[Arg::Raw(thread_name())]);
        lock_acquire(&raw mut LOCK);
        cond_wait(&raw mut CONDITION, &raw mut LOCK);
        msg("Thread %s woke up.", &[Arg::Raw(thread_name())]);
        lock_release(&raw mut LOCK);
    }
}
