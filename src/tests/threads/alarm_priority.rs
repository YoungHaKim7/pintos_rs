//! Port of `tests/threads/alarm-priority.c` — checks that when
//! the alarm clock wakes up threads, the higher-priority threads
//! run first.

use crate::devices::timer::{TIMER_FREQ, timer_elapsed, timer_sleep, timer_ticks};
use crate::lib::stdio::Arg;
use crate::threads::synch::{Semaphore, sema_down, sema_init, sema_up};
use crate::threads::thread::{
    PRI_DEFAULT, PRI_MIN, THREAD_MLFQS, thread_create, thread_name, thread_set_priority,
};

use super::snprintf;
use crate::tests::msg;

static mut WAKE_TIME: i64 = 0;
static mut WAIT_SEMA: Semaphore = Semaphore::EMPTY;

pub unsafe fn test_alarm_priority() {
    unsafe {
        /* This test does not work with the MLFQS. */
        crate::ASSERT!(!THREAD_MLFQS);

        *core::ptr::addr_of_mut!(WAKE_TIME) = timer_ticks() + 5 * TIMER_FREQ as i64;
        sema_init(&raw mut WAIT_SEMA, 0);

        for i in 0..10 {
            let priority = PRI_DEFAULT - (i + 5) % 10 - 1;
            let mut name = [0u8; 16];
            snprintf(&mut name, "priority %d", &[Arg::I(priority as i64)]);
            thread_create(
                name.as_ptr(),
                priority,
                Some(alarm_priority_thread),
                core::ptr::null_mut(),
            );
        }

        thread_set_priority(PRI_MIN);

        for _ in 0..10 {
            sema_down(&raw mut WAIT_SEMA);
        }
    }
}

unsafe extern "C" fn alarm_priority_thread(_aux: *mut ()) {
    unsafe {
        /* Busy-wait until the current time changes. */
        let start_time = timer_ticks();
        while timer_elapsed(start_time) == 0 {
            continue;
        }

        /* Now we know we're at the very beginning of a timer
        tick, so we can call timer_sleep() without worrying about
        races between checking the time and a timer interrupt. */
        timer_sleep(WAKE_TIME - timer_ticks());

        /* Print a message on wake-up. */
        msg("Thread %s woke up.", &[Arg::Raw(thread_name())]);

        sema_up(&raw mut WAIT_SEMA);
    }
}
