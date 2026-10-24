//! Port of `tests/threads/` — threads test suite.
//!
//! One Rust module per C source file; the test table lives in
//! `tests/mod.rs` (port of tests/threads/tests.c).

pub mod alarm_negative;
pub mod alarm_priority;
pub mod alarm_simultaneous;
pub mod alarm_wait;
pub mod alarm_zero;
pub mod priority_change;
pub mod priority_condvar;
pub mod priority_donate_chain;
pub mod priority_donate_lower;
pub mod priority_donate_multiple;
pub mod priority_donate_multiple2;
pub mod priority_donate_nest;
pub mod priority_donate_one;
pub mod priority_donate_sema;
pub mod priority_fifo;
pub mod priority_preempt;
pub mod priority_sema;

pub use alarm_negative::test_alarm_negative;
pub use alarm_priority::test_alarm_priority;
pub use alarm_simultaneous::test_alarm_simultaneous;
pub use alarm_wait::{test_alarm_multiple, test_alarm_single};
pub use alarm_zero::test_alarm_zero;
pub use priority_change::test_priority_change;
pub use priority_condvar::test_priority_condvar;
pub use priority_donate_chain::test_priority_donate_chain;
pub use priority_donate_lower::test_priority_donate_lower;
pub use priority_donate_multiple::test_priority_donate_multiple;
pub use priority_donate_multiple2::test_priority_donate_multiple2;
pub use priority_donate_nest::test_priority_donate_nest;
pub use priority_donate_one::test_priority_donate_one;
pub use priority_donate_sema::test_priority_donate_sema;
pub use priority_fifo::test_priority_fifo;
pub use priority_preempt::test_priority_preempt;
pub use priority_sema::test_priority_sema;

/* C: snprintf(name, sizeof name, ...).  Shared by the tests. */
pub unsafe fn snprintf(buf: &mut [u8], format: &str, args: &[crate::lib::stdio::Arg]) {
    unsafe {
        crate::lib::stdio::vsnprintf(buf.as_mut_ptr(), buf.len(), format, args);
    }
}
