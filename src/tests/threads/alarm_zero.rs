//! Port of `tests/threads/alarm-zero.c` — tests timer_sleep(0),
//! which should return immediately.

use crate::devices::timer::timer_sleep;

use crate::tests::pass;

pub unsafe fn test_alarm_zero() {
    unsafe {
        timer_sleep(0);
        pass();
    }
}
