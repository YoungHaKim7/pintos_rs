//! Port of `tests/threads/alarm-wait.c` — creates N threads, each
//! of which sleeps a different, fixed duration, M times.  Records
//! the wake-up order and verifies that it is valid.
//!
//! The C file defines both `test_alarm_single` and
//! `test_alarm_multiple`.

use crate::devices::timer::{timer_sleep, timer_ticks};
use crate::lib::stdio::Arg;
use crate::threads::malloc::{free, malloc};
use crate::threads::synch::{Lock, lock_acquire, lock_init, lock_release};
use crate::threads::thread::{PRI_DEFAULT, THREAD_MLFQS, thread_create};

use super::snprintf;
use crate::tests::{fail, msg};

/* Information about the test. */
#[repr(C)]
struct SleepTest {
    start: i64,           /* Current time at start of test. */
    iterations: i32,      /* Number of iterations per thread. */

    /* Output. */
    output_lock: Lock,    /* Lock protecting output buffer. */
    output_pos: *mut i32, /* Current position in output buffer. */
}

/* Information about an individual thread in the test. */
#[repr(C)]
struct SleepThread {
    test: *mut SleepTest, /* Info shared between all threads. */
    id: i32,              /* Sleeper ID. */
    duration: i32,        /* Number of ticks to sleep. */
    iterations: i32,      /* Iterations counted so far. */
}

pub unsafe fn test_alarm_single() {
    unsafe {
        test_sleep(5, 1);
    }
}

pub unsafe fn test_alarm_multiple() {
    unsafe {
        test_sleep(5, 7);
    }
}

/* Runs THREAD_CNT threads that sleep ITERATIONS times each. */
unsafe fn test_sleep(thread_cnt: i32, iterations: i32) {
    unsafe {
        /* This test does not work with the MLFQS. */
        crate::ASSERT!(!THREAD_MLFQS);

        msg(
            "Creating %d threads to sleep %d times each.",
            &[Arg::I(thread_cnt as i64), Arg::I(iterations as i64)],
        );
        msg("Thread 0 sleeps 10 ticks each time,", &[]);
        msg("thread 1 sleeps 20 ticks each time, and so on.", &[]);
        msg("If successful, product of iteration count and", &[]);
        msg("sleep duration will appear in nondescending order.", &[]);

        /* Allocate memory. */
        let threads =
            malloc(core::mem::size_of::<SleepThread>() * thread_cnt as usize) as *mut SleepThread;
        let output = malloc(
            core::mem::size_of::<i32>() * (iterations * thread_cnt * 2) as usize,
        ) as *mut i32;
        if threads.is_null() || output.is_null() {
            crate::PANIC!("couldn't allocate memory for test");
        }

        /* Initialize test. */
        let mut test = SleepTest {
            start: timer_ticks() + 100,
            iterations,
            output_lock: Lock::EMPTY,
            output_pos: output,
        };
        lock_init(&raw mut test.output_lock);

        /* Start threads. */
        for i in 0..thread_cnt {
            let t = threads.add(i as usize);
            let mut name = [0u8; 16];
            snprintf(&mut name, "thread %d", &[Arg::I(i as i64)]);

            (*t).test = &raw mut test;
            (*t).id = i;
            (*t).duration = (i + 1) * 10;
            (*t).iterations = 0;

            thread_create(name.as_ptr(), PRI_DEFAULT, Some(sleeper), t as *mut ());
        }

        /* Wait long enough for all the threads to finish. */
        timer_sleep(100 + thread_cnt as i64 * iterations as i64 * 10 + 100);

        /* Acquire the output lock in case some rogue thread is
        still running. */
        lock_acquire(&raw mut test.output_lock);

        /* Print completion order. */
        let mut product = 0;
        let mut op = output;
        while op < test.output_pos {
            let value = *op;
            crate::ASSERT!(value >= 0 && value < thread_cnt);
            let t = threads.add(value as usize);

            (*t).iterations += 1;
            let new_prod = (*t).iterations * (*t).duration;

            msg(
                "thread %d: duration=%d, iteration=%d, product=%d",
                &[
                    Arg::I((*t).id as i64),
                    Arg::I((*t).duration as i64),
                    Arg::I((*t).iterations as i64),
                    Arg::I(new_prod as i64),
                ],
            );

            if new_prod >= product {
                product = new_prod;
            } else {
                fail(
                    "thread %d woke up out of order (%d > %d)!",
                    &[
                        Arg::I((*t).id as i64),
                        Arg::I(product as i64),
                        Arg::I(new_prod as i64),
                    ],
                );
            }

            op = op.add(1);
        }

        /* Verify that we had the proper number of wakeups. */
        for i in 0..thread_cnt {
            let t = threads.add(i as usize);
            if (*t).iterations != iterations {
                fail(
                    "thread %d woke up %d times instead of %d",
                    &[
                        Arg::I(i as i64),
                        Arg::I((*t).iterations as i64),
                        Arg::I(iterations as i64),
                    ],
                );
            }
        }

        lock_release(&raw mut test.output_lock);
        free(output as *mut u8);
        free(threads as *mut u8);
    }
}

/* Sleeper thread. */
unsafe extern "C" fn sleeper(t_: *mut ()) {
    unsafe {
        let t = t_ as *mut SleepThread;
        let test = (*t).test;

        for i in 1..=(*test).iterations {
            let sleep_until = (*test).start + i as i64 * (*t).duration as i64;
            timer_sleep(sleep_until - timer_ticks());
            lock_acquire(&raw mut (*test).output_lock);
            *(*test).output_pos = (*t).id;
            (*test).output_pos = (*test).output_pos.add(1);
            lock_release(&raw mut (*test).output_lock);
        }
    }
}
