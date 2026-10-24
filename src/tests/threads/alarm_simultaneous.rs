//! Port of `tests/threads/alarm-simultaneous.c` — creates N
//! threads, each of which sleeps a different, fixed duration, M
//! times.  Records the wake-up order and verifies that it is
//! valid.

use crate::devices::timer::{timer_sleep, timer_ticks};
use crate::lib::stdio::Arg;
use crate::threads::malloc::{free, malloc};
use crate::threads::thread::{PRI_DEFAULT, THREAD_MLFQS, thread_create, thread_yield};

use super::snprintf;
use crate::tests::msg;

/* Information about the test. */
#[repr(C)]
struct SleepTest {
    start: i64,           /* Current time at start of test. */
    iterations: i32,      /* Number of iterations per thread. */
    output_pos: *mut i32, /* Current position in output buffer. */
}

pub unsafe fn test_alarm_simultaneous() {
    unsafe {
        test_sleep(3, 5);
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
        msg("Each thread sleeps 10 ticks each time.", &[]);
        msg("Within an iteration, all threads should wake up on the same tick.", &[]);

        /* Allocate memory. */
        let output = malloc(
            core::mem::size_of::<i32>() * (iterations * thread_cnt * 2) as usize,
        ) as *mut i32;
        if output.is_null() {
            crate::PANIC!("couldn't allocate memory for test");
        }

        /* Initialize test. */
        let mut test = SleepTest {
            start: timer_ticks() + 100,
            iterations,
            output_pos: output,
        };

        /* Start threads. */
        for i in 0..thread_cnt {
            let mut name = [0u8; 16];
            snprintf(&mut name, "thread %d", &[Arg::I(i as i64)]);
            thread_create(name.as_ptr(), PRI_DEFAULT, Some(sleeper), &raw mut test as *mut ());
        }

        /* Wait long enough for all the threads to finish. */
        timer_sleep(100 + iterations as i64 * 10 + 100);

        /* Print completion order. */
        msg(
            "iteration 0, thread 0: woke up after %d ticks",
            &[Arg::I(*output as i64)],
        );
        let mut i = 1;
        while (output.add(i) as *const i32) < (test.output_pos as *const i32) {
            msg(
                "iteration %d, thread %d: woke up %d ticks later",
                &[
                    Arg::I((i / thread_cnt as usize) as i64),
                    Arg::I((i % thread_cnt as usize) as i64),
                    Arg::I((*output.add(i) - *output.add(i - 1)) as i64),
                ],
            );
            i += 1;
        }

        free(output as *mut u8);
    }
}

/* Sleeper thread. */
unsafe extern "C" fn sleeper(test_: *mut ()) {
    unsafe {
        let test = test_ as *mut SleepTest;

        /* Make sure we're at the beginning of a timer tick. */
        timer_sleep(1);

        for i in 1..=(*test).iterations {
            let sleep_until = (*test).start + i as i64 * 10;
            timer_sleep(sleep_until - timer_ticks());
            *(*test).output_pos = (timer_ticks() - (*test).start) as i32;
            (*test).output_pos = (*test).output_pos.add(1);
            thread_yield();
        }
    }
}
