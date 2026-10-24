//! Port of `tests/threads/priority-fifo.c` — creates several
//! threads all at the same priority and ensures that they
//! consistently run in the same round-robin order.

use crate::lib::stdio::Arg;
use crate::threads::malloc::malloc;
use crate::threads::synch::{Lock, lock_acquire, lock_init, lock_release};
use crate::threads::thread::{
    PRI_DEFAULT, THREAD_MLFQS, thread_create, thread_set_priority, thread_yield,
};

use super::snprintf;
use crate::tests::msg;

const THREAD_CNT: usize = 16;
const ITER_CNT: usize = 16;

#[repr(C)]
#[derive(Clone, Copy)]
struct SimpleThreadData {
    id: i32,           /* Sleeper ID. */
    iterations: i32,   /* Iterations so far. */
    lock: *mut Lock,   /* Lock on output. */
    op: *mut *mut i32, /* Output buffer position. */
}

pub unsafe fn test_priority_fifo() {
    unsafe {
        let mut data: [SimpleThreadData; THREAD_CNT] = [SimpleThreadData {
            id: 0,
            iterations: 0,
            lock: core::ptr::null_mut(),
            op: core::ptr::null_mut(),
        }; THREAD_CNT];
        let mut lock = Lock::EMPTY;
        let mut op_ptr: *mut i32 = core::ptr::null_mut();

        /* This test does not work with the MLFQS. */
        crate::ASSERT!(!THREAD_MLFQS);

        /* Make sure our priority is the default. */
        crate::ASSERT!(
            crate::threads::thread::thread_get_priority() == PRI_DEFAULT
        );

        msg(
            "%d threads will iterate %d times in the same order each time.",
            &[Arg::I(THREAD_CNT as i64), Arg::I(ITER_CNT as i64)],
        );
        msg("If the order varies then there is a bug.", &[]);

        let output =
            malloc(core::mem::size_of::<i32>() * THREAD_CNT * ITER_CNT * 2) as *mut i32;
        crate::ASSERT!(!output.is_null());
        op_ptr = output;
        lock_init(&raw mut lock);

        thread_set_priority(PRI_DEFAULT + 2);
        for i in 0..THREAD_CNT {
            let mut name = [0u8; 16];
            snprintf(&mut name, "%d", &[Arg::I(i as i64)]);
            let d = &raw mut data[i];
            (*d).id = i as i32;
            (*d).iterations = 0;
            (*d).lock = &raw mut lock;
            (*d).op = &raw mut op_ptr;
            thread_create(name.as_ptr(), PRI_DEFAULT + 1, Some(simple_thread_func), d as *mut ());
        }

        thread_set_priority(PRI_DEFAULT);
        /* All the other threads now run to termination here. */
        crate::ASSERT!(lock.holder.is_null());

        let mut cnt = 0;
        let mut out = output;
        while out < op_ptr {
            let value = *out;
            crate::ASSERT!(value >= 0 && (value as usize) < THREAD_CNT);
            let d = &raw mut data[value as usize];
            if cnt % THREAD_CNT == 0 {
                crate::printf!("(priority-fifo) iteration:");
            }
            crate::printf!(" %d", Arg::I((*d).id as i64));
            cnt += 1;
            if cnt % THREAD_CNT == 0 {
                crate::printf!("\n");
            }
            (*d).iterations += 1;
            out = out.add(1);
        }
    }
}

unsafe extern "C" fn simple_thread_func(data_: *mut ()) {
    unsafe {
        let data = data_ as *mut SimpleThreadData;

        for _ in 0..ITER_CNT {
            lock_acquire((*data).lock);
            /* C: *(*data->op)++ = data->id; */
            **(*data).op = (*data).id;
            *(*data).op = (*(*data).op).add(1);
            lock_release((*data).lock);
            thread_yield();
        }
    }
}
