//! Port of `threads/init.c` — kernel main program.

use crate::lib::kernel::console::{console_init, vprintf};
use crate::lib::random::random_init;
use crate::lib::stdio::Arg;
use crate::lib::stdlib::atoi;
use crate::lib::string::{memset, strchr, strcmp, strnlen, strtok_r};
use crate::threads::interrupt::{IntrLevel, intr_disable};
use crate::threads::loader::{LOADER_ARGS_LEN, ptov_arg_cnt, ptov_args};
use crate::threads::pte::{PTE_P, PTE_W};
use crate::threads::vaddr::{PGSIZE, ptov};

/* Page-map-level-4 with kernel mappings only. */
pub static mut BASE_PML4: *mut u64 = core::ptr::null_mut();

/* -f: Format the file system? */
pub static mut FORMAT_FILESYS: bool = false;

/* -q: Power off after kernel tasks complete? */
pub static mut POWER_OFF_WHEN_DONE: bool = false;

/* C name compatibility helper. */
pub use POWER_OFF_WHEN_DONE as power_off_when_done;

/* -threads-tests: run threads tests directly instead of exec'ing a
user program (full-featured kernel equivalent of the C build's
per-project variants). */
pub static mut THREAD_TESTS: bool = false;

/* Buffer for argv-style access to the kernel command line. */
static mut ARGV: [*mut u8; (LOADER_ARGS_LEN as usize) / 2 + 1] =
    [core::ptr::null_mut(); (LOADER_ARGS_LEN as usize) / 2 + 1];

/// Pintos main program.  Called by start.S with no arguments.
#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    unsafe {
        /* Clear BSS. */
        bss_init();

        /* Break command line into arguments and parse options. */
        let argv = read_command_line();
        let argv = parse_options(argv);

        /* Initialize ourselves as a thread so we can use locks,
        then enable console locking. */
        crate::threads::thread::thread_init();
        console_init();

        /* Initialize memory system. */
        let mem_end = crate::threads::palloc::palloc_init();
        crate::threads::malloc::malloc_init();
        paging_init(mem_end);

        crate::userprog::tss::tss_init();
        crate::userprog::gdt::gdt_init();

        /* Initialize interrupt handlers. */
        crate::threads::interrupt::intr_init();
        crate::devices::timer::timer_init();
        crate::devices::kbd::kbd_init();
        crate::devices::input::input_init();

        crate::userprog::exception::exception_init();
        crate::userprog::syscall::syscall_init();

        /* Start thread scheduler and enable interrupts. */
        crate::threads::thread::thread_start();
        crate::devices::serial::serial_init_queue();
        crate::devices::timer::timer_calibrate();

        /* Initialize file system (only when a filesystem disk is
        attached; the threads tests run without one). */
        crate::devices::disk::disk_init();
        crate::filesys::filesys_init(FORMAT_FILESYS);

        crate::vm::vm_init();

        vprintf("Boot complete.\n", &[]);

        /* Run actions specified on kernel command line. */
        run_actions(argv);

        /* Finish up. */
        if POWER_OFF_WHEN_DONE {
            power_off();
        }
        crate::threads::thread::thread_exit();
    }
}

/* Clear BSS: the start and end of the segment are recorded by the
linker as _start_bss and _end_bss (see kernel.lds). */
fn bss_init() {
    unsafe {
        unsafe extern "C" {
            static _start_bss: u8;
            static _end_bss: u8;
        }

        let start = core::ptr::addr_of!(_start_bss) as *mut u8;
        let end = core::ptr::addr_of!(_end_bss) as *mut u8;
        memset(start, 0, end.offset_from(start) as usize);
    }
}

/* Populates the page table with the kernel virtual mapping, and
then sets up the CPU to use the new page directory.  Points
base_pml4 to the pml4 it creates. */
unsafe fn paging_init(mem_end: u64) {
    unsafe {
        unsafe extern "C" {
            static start: u8;
            static _end_kernel_text: u8;
        }

        BASE_PML4 = crate::threads::palloc::palloc_get_page(
            crate::threads::palloc::PAL_ASSERT | crate::threads::palloc::PAL_ZERO,
        ) as *mut u64;
        let pml4 = BASE_PML4;

        let text_start = core::ptr::addr_of!(start) as u64;
        let text_end = core::ptr::addr_of!(_end_kernel_text) as u64;

        /* Maps physical address [0 ~ mem_end] to
        [LOADER_KERN_BASE ~ LOADER_KERN_BASE + mem_end]. */
        let mut pa: u64 = 0;
        while pa < mem_end {
            let va = ptov(pa);

            let mut perm = PTE_P | PTE_W;
            if text_start <= va && va < text_end {
                perm &= !PTE_W;
            }

            let pte = crate::threads::mmu::pml4e_walk(pml4, va, true);
            if !pte.is_null() {
                *pte = pa | perm;
            }

            pa += PGSIZE;
        }

        /* Reload cr3. */
        crate::threads::mmu::pml4_activate(core::ptr::null_mut());
    }
}

/* Breaks the kernel command line into words and returns them as
an argv-like array. */
unsafe fn read_command_line() -> *mut *mut u8 {
    unsafe {
        let argv = &raw mut ARGV as *mut *mut u8;
        /* LOADER_ARG_CNT (0x7d7a) is not 4-byte aligned; C reads it
        unaligned, Rust must use read_unaligned to match. */
        let argc = core::ptr::read_unaligned(ptov_arg_cnt() as *const u32);
        let mut p = ptov_args() as *mut u8;
        let end = p.wrapping_add(LOADER_ARGS_LEN as usize);

        for i in 0..argc as usize {
            if (p as u64) >= end as u64 {
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
            /* NB: byte-string literals need an explicit \0 to act
            as C strings. */
            let name = strtok_r(arg, b"=\0".as_ptr(), &raw mut save_ptr);
            let value = strtok_r(core::ptr::null_mut(), b"\0".as_ptr(), &raw mut save_ptr);

            if strcmp(name, b"-h\0".as_ptr()) == 0 {
                usage();
            } else if strcmp(name, b"-q\0".as_ptr()) == 0 {
                POWER_OFF_WHEN_DONE = true;
            } else if strcmp(name, b"-f\0".as_ptr()) == 0 {
                FORMAT_FILESYS = true;
            } else if strcmp(name, b"-rs\0".as_ptr()) == 0 {
                random_init(atoi(value) as u32);
            } else if strcmp(name, b"-mlfqs\0".as_ptr()) == 0 {
                crate::threads::thread::THREAD_MLFQS = true;
            } else if strcmp(name, b"-ul\0".as_ptr()) == 0 {
                crate::threads::palloc::USER_PAGE_LIMIT = atoi(value) as usize;
            } else if strcmp(name, b"-threads-tests\0".as_ptr()) == 0 {
                THREAD_TESTS = true;
            } else {
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
        if THREAD_TESTS {
            crate::tests::run_test(task);
        } else {
            /* USERPROG path: load and exec the named user program.
            (userprog milestone; until then, threads tests work via
            -threads-tests.) */
            // TODO(userprog): process_wait(process_create_initd(task))
            crate::tests::run_test(task);
        }
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
            name: b"run\0",
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
         \x20 -f                 Format file system disk during power off.\n\
         \x20 -rs=SEED           Set random number seed to SEED.\n\
         \x20 -mlfqs             Use multi-level feedback queue scheduler.\n\
         \x20 -ul=COUNT          Limit user memory to COUNT pages.\n\
         \x20 -threads-tests     Run threads tests directly (full kernel).\n",
        &[],
    );
    power_off();
}

/* Prints statistics. */
unsafe fn print_stats() {
    unsafe {
        vprintf("Clocks: ", &[]);
        crate::devices::timer::timer_print_stats();
        crate::threads::thread::thread_print_stats();
        crate::devices::disk::disk_print_stats();
        crate::lib::kernel::console::console_print_stats();
        crate::devices::kbd::kbd_print_stats();
        crate::userprog::exception::exception_print_stats();
    }
}

/* Powers down the machine we're running on, as long as we're
running on Bochs or QEMU. */
#[unsafe(no_mangle)]
pub extern "C" fn power_off() -> ! {
    unsafe {
        /* File system shutdown. */
        crate::filesys::filesys_done();

        print_stats();

        vprintf("Powering off...\n", &[]);
        crate::intrinsic::outw(0x604, 0x2000); /* Poweroff command for qemu */
    }
    loop {
        core::hint::spin_loop();
    }
}

/* Silence unused-import warnings for items used only under some
configurations. */
#[allow(unused_imports)]
use IntrLevel as _;
