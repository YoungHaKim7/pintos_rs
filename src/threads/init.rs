//! Port of `threads/init.c` — kernel main program.
//!
//! M0: covers bss_init, command-line reading/parsing, run_actions,
//! and power_off.  Subsystem initialization (threads, palloc, mmu,
//! devices, ...) is added as each milestone lands; `Boot complete.`
//! and the action runner already behave like the C kernel.

use crate::lib::kernel::console::vprintf;
use crate::lib::random::random_init;
use crate::lib::stdio::Arg;
use crate::lib::stdlib::atoi;
use crate::lib::string::{memset, strchr, strcmp, strnlen, strtok_r};
use crate::threads::interrupt::intr_disable;
use crate::threads::loader::{LOADER_ARGS_LEN, ptov_arg_cnt, ptov_args};
use crate::threads::vaddr::ptov;

/* -q: Power off after kernel tasks complete? */
pub static mut POWER_OFF_WHEN_DONE: bool = false;

/* C name compatibility helper. */
pub use POWER_OFF_WHEN_DONE as power_off_when_done;

/* Buffer for argv-style access to the kernel command line. */
static mut ARGV: [*mut u8; (LOADER_ARGS_LEN as usize) / 2 + 1] =
    [core::ptr::null_mut(); (LOADER_ARGS_LEN as usize) / 2 + 1];

/// Pintos main program.  Called by start.S with no arguments.
#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    /* Clear BSS. */
    bss_init();

    /* Break command line into arguments and parse options. */
    let argv = read_command_line();
    let argv = parse_options(argv);

    /* M1+: thread_init/console_init/palloc/malloc/paging/...
    are inserted here as they are ported. */

    vprintf("Boot complete.\n", &[]);

    /* Run actions specified on kernel command line. */
    run_actions(argv);

    /* Finish up. */
    unsafe {
        if POWER_OFF_WHEN_DONE {
            power_off();
        }
    }
    // M1+: thread_exit().
    intr_disable();
    loop {}
}

/* Clear BSS: the start and end of the segment are recorded by the
linker as _start_bss and _end_bss (see kernel.lds). */
fn bss_init() {
    unsafe {
        #[unsafe(link_name = "_start_bss")]
        static BSS_START: u8 = 0;
        #[unsafe(link_name = "_end_bss")]
        static BSS_END: u8 = 0;

        let start = &BSS_START as *const u8 as *mut u8;
        let end = &BSS_END as *const u8;
        memset(start, 0, end.offset_from(start) as usize);
    }
}

/* Breaks the kernel command line into words and returns them as
an argv-like array. */
unsafe fn read_command_line() -> *mut *mut u8 {
    unsafe {
        let argv = &raw mut ARGV as *mut *mut u8;
        let argc = *(ptov_arg_cnt() as *const u32);
        let mut p = ptov_args() as *mut u8;
        let end = p.wrapping_add(LOADER_ARGS_LEN as usize);

        for i in 0..argc as usize {
            if p >= end {
                crate::PANIC!("command line arguments overflow");
            }

            *argv.add(i) = p;
            p = p.add(strnlen(p, end.offset_from(p) as usize) + 1);
        }
        *argv.add(argc as usize) = core::ptr::null_mut();

        /* Print kernel command line. */
        vprintf("Kernel command line:", &[]);
        for i in 0..argc as usize {
            let arg = *argv.add(i);
            if strchr(arg, b' ').is_null() {
                vprintf(" %s", &[Arg::Raw(arg)]);
            } else {
                vprintf(" '%s'", &[Arg::Raw(arg)]);
            }
        }
        vprintf("\n", &[]);

        argv
    }
}

/* Parses options in ARGV[] and returns the first non-option
argument. */
unsafe fn parse_options(mut argv: *mut *mut u8) -> *mut *mut u8 {
    unsafe {
        loop {
            let arg = *argv;
            if arg.is_null() || *arg != b'-' {
                break;
            }
            argv = argv.add(1);

            let mut save_ptr: *mut u8 = core::ptr::null_mut();
            let name = strtok_r(arg, b"=", &raw mut save_ptr);
            let value = strtok_r(core::ptr::null_mut(), b"", &raw mut save_ptr);

            if strcmp(name, b"-h\0".as_ptr()) == 0 {
                usage();
            } else if strcmp(name, b"-q\0".as_ptr()) == 0 {
                POWER_OFF_WHEN_DONE = true;
            } else if strcmp(name, b"-rs\0".as_ptr()) == 0 {
                random_init(atoi(value) as u32);
            }
            /* M2+: -mlfqs, -ul, -threads-tests, -f (filesys). */
            else {
                crate::PANIC!("unknown option `%s' (use -h for help)", Arg::Raw(name));
            }
        }

        argv
    }
}

/* Runs the task specified in ARGV[1]. */
unsafe fn run_task(argv: *mut *mut u8) {
    unsafe {
        let task = *argv.add(1);

        vprintf("Executing '%s':\n", &[Arg::Raw(task)]);
        crate::tests::run_test(task);
        vprintf("Execution of '%s' complete.\n", &[Arg::Raw(task)]);
    }
}

/* Executes all of the actions specified in ARGV[] up to the null
pointer sentinel. */
unsafe fn run_actions(mut argv: *mut *mut u8) {
    unsafe {
        /* Table of supported actions. */
        struct Action {
            name: &'static [u8],
            argc: usize,
            function: unsafe fn(*mut *mut u8),
        }
        static ACTIONS: &[Action] = &[Action {
            name: b"run",
            argc: 2,
            function: run_task,
        }];

        while !(*argv).is_null() {
            /* Find action name. */
            let name = *argv;
            let a = ACTIONS
                .iter()
                .find(|a| strcmp(name, a.name.as_ptr()) == 0)
                .unwrap_or_else(|| {
                    crate::PANIC!("unknown action `%s' (use -h for help)", Arg::Raw(name))
                });

            /* Check for required arguments. */
            for i in 1..a.argc {
                if (*argv.add(i)).is_null() {
                    crate::PANIC!(
                        "action `%s' requires %d argument(s)",
                        Arg::Raw(name),
                        a.argc as i64 - 1
                    );
                }
            }

            /* Invoke action and advance. */
            (a.function)(argv);
            argv = argv.add(a.argc);
        }
    }
}

/* Prints a kernel command line help message and powers off the
machine. */
fn usage() -> ! {
    vprintf(
        "\nCommand line syntax: [OPTION...] [ACTION...]\n\
         Options must precede actions.\n\
         Actions are executed in the order specified.\n\
         \nAvailable actions:\n\
         \x20 run TEST           Run TEST.\n\
         \nOptions:\n\
         \x20 -h                 Print this help message and power off.\n\
         \x20 -q                 Power off VM after actions or on panic.\n\
         \x20 -rs=SEED           Set random number seed to SEED.\n",
        &[],
    );
    power_off();
}

/* Powers down the machine we're running on, as long as we're
running on Bochs or QEMU. */
#[unsafe(no_mangle)]
pub extern "C" fn power_off() -> ! {
    /* M1+: print_stats() (timer, thread, console, kbd). */
    vprintf("Powering off...\n", &[]);
    unsafe {
        crate::intrinsic::outw(0x604, 0x2000); /* Poweroff command for qemu */
    }
    loop {}
}
