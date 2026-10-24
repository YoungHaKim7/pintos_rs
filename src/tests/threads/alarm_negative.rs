//! Port of `tests/threads/alarm-negative.c` — tests
//! timer_sleep(-100).  Only requirement is that it not crash.

use crate::devices::timer::timer_sleep;

use crate::tests::pass;

pub unsafe fn test_alarm_negative() {
    unsafe {
        timer_sleep(-100);
        pass();
    }
}
