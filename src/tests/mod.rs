//! Port of `tests/` — kernel test harness.
//!
//! M0: the test table is empty; M1 fills in the threads tests
//! (port of tests/threads/tests.h + tests/main.c harness).

use crate::lib::stdio::Arg;

/// Runs the named test (C's `run_test` from tests/threads/tests.c).
pub fn run_test(name: *const u8) {
    crate::printf!("run_test(): unknown test `%s'\n", Arg::Raw(name));
    crate::PANIC!("no test named `%s'", Arg::Raw(name));
}
