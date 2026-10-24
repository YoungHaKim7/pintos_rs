//! Port of `tests/threads/tests.c` — kernel test harness.
//!
//! Threads tests are linked into the kernel and selected by name
//! via the kernel command line (`run TEST` action with
//! `-threads-tests`).  The full userprog/vm/filesys test suites
//! run as user processes and arrive with their milestones.

pub mod threads;

use crate::lib::kernel::console::{putchar, vprintf};
use crate::lib::stdio::Arg;
use crate::lib::string::strcmp;

struct Test {
    name: &'static [u8], /* NUL-terminated test name. */
    function: unsafe fn(),
}

static TESTS: &[Test] = &[
    Test {
        name: b"alarm-single\0",
        function: threads::test_alarm_single,
    },
    Test {
        name: b"alarm-multiple\0",
        function: threads::test_alarm_multiple,
    },
    Test {
        name: b"alarm-simultaneous\0",
        function: threads::test_alarm_simultaneous,
    },
    Test {
        name: b"alarm-priority\0",
        function: threads::test_alarm_priority,
    },
    Test {
        name: b"alarm-zero\0",
        function: threads::test_alarm_zero,
    },
    Test {
        name: b"alarm-negative\0",
        function: threads::test_alarm_negative,
    },
    Test {
        name: b"priority-change\0",
        function: threads::test_priority_change,
    },
    Test {
        name: b"priority-donate-one\0",
        function: threads::test_priority_donate_one,
    },
    Test {
        name: b"priority-donate-multiple\0",
        function: threads::test_priority_donate_multiple,
    },
    Test {
        name: b"priority-donate-multiple2\0",
        function: threads::test_priority_donate_multiple2,
    },
    Test {
        name: b"priority-donate-nest\0",
        function: threads::test_priority_donate_nest,
    },
    Test {
        name: b"priority-donate-sema\0",
        function: threads::test_priority_donate_sema,
    },
    Test {
        name: b"priority-donate-lower\0",
        function: threads::test_priority_donate_lower,
    },
    Test {
        name: b"priority-donate-chain\0",
        function: threads::test_priority_donate_chain,
    },
    Test {
        name: b"priority-fifo\0",
        function: threads::test_priority_fifo,
    },
    Test {
        name: b"priority-preempt\0",
        function: threads::test_priority_preempt,
    },
    Test {
        name: b"priority-sema\0",
        function: threads::test_priority_sema,
    },
    Test {
        name: b"priority-condvar\0",
        function: threads::test_priority_condvar,
    },
];

/* Name of the running test. */
static mut TEST_NAME: *const u8 = b"\0".as_ptr();

/* Runs the test named NAME. */
pub unsafe fn run_test(name: *const u8) {
    unsafe {
        for t in TESTS {
            if strcmp(name, t.name.as_ptr()) == 0 {
                TEST_NAME = t.name.as_ptr();
                msg("begin", &[]);
                (t.function)();
                msg("end", &[]);
                return;
            }
        }
        crate::PANIC!("no test named \"%s\"", Arg::Raw(name));
    }
}

/* Prints FORMAT as if with printf(), prefixing the output by the
name of the test and following it with a new-line character. */
pub fn msg(format: &str, args: &[Arg]) {
    unsafe {
        crate::printf!("(%s) ", Arg::Raw(TEST_NAME));
        vprintf(format, args);
        putchar(b'\n');
    }
}

/* Prints failure message FORMAT as if with printf(), prefixing
the output by the name of the test and FAIL: and following it
with a new-line character, and then panics the kernel. */
pub fn fail(format: &str, args: &[Arg]) {
    unsafe {
        crate::printf!("(%s) FAIL: ", Arg::Raw(TEST_NAME));
        vprintf(format, args);
        putchar(b'\n');

        crate::PANIC!("test failed");
    }
}

/* Prints a message indicating the current test passed. */
pub fn pass() {
    unsafe {
        crate::printf!("(%s) PASS\n", Arg::Raw(TEST_NAME));
    }
}
