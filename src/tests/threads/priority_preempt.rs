//! Port of `tests/threads/priority-preempt.c` — ensures that a
//! high-priority thread really preempts.

use crate::lib::stdio::Arg;
use crate::threads::thread::{
    PRI_DEFAULT, THREAD_MLFQS, thread_create, thread_get_priority, thread_name, thread_yield,
};

use crate::tests::msg;

pub unsafe fn test_priority_preempt() {
    unsafe {
        /* This test does not work with the MLFQS. */
        crate::ASSERT!(!THREAD_MLFQS);

        /* Make sure our priority is the default. */
        crate::ASSERT!(thread_get_priority() == PRI_DEFAULT);

        thread_create(
            b"high-priority\0".as_ptr(),
            PRI_DEFAULT + 1,
            Some(simple_thread_func),
            core::ptr::null_mut(),
        );
        msg("The high-priority thread should have already completed.", &[]);
    }
}

unsafe extern "C" fn simple_thread_func(_aux: *mut ()) {
    unsafe {
        for i in 0..5 {
            msg(
                "Thread %s iteration %d",
                &[Arg::Raw(thread_name()), Arg::I(i)],
            );
            thread_yield();
        }
        msg("Thread %s done!", &[Arg::Raw(thread_name())]);
    }
}
