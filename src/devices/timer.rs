//! Port of `devices/timer.c` — 8254 Programmable Interval Timer driver.
//!
//! See [8254] for hardware details of the 8254 timer chip.

use crate::ASSERT;
use crate::intrinsic::outb;
use crate::lib::kernel::list::{self, List, ListElem};
use crate::printf;
use crate::threads::interrupt::{
    IntrFrame, IntrLevel, intr_context, intr_disable, intr_get_level, intr_register_ext,
    intr_set_level,
};
use crate::threads::thread::{Thread, thread_block, thread_current, thread_tick, thread_unblock};

/* Number of timer interrupts per second. */
pub const TIMER_FREQ: i32 = 100;

/* 8254 timer requires TIMER_FREQ >= 19. */
const _: () = assert!(TIMER_FREQ >= 19);
/* TIMER_FREQ <= 1000 recommended. */
const _: () = assert!(TIMER_FREQ <= 1000);

/* Number of timer ticks since OS booted. */
static mut TICKS: i64 = 0;

/* Number of loops per timer tick.
Initialized by timer_calibrate(). */
static mut LOOPS_PER_TICK: u32 = 0;

/* --- Sleeping threads -------------------------------------------------

The C original's timer_sleep() busy-waited on thread_yield().  This
port implements the Pintos project 1 "alarm" assignment instead:
a thread that sleeps is inserted into SLEEP_LIST (ordered by wake
tick) and blocked; timer_interrupt() unblocks every thread whose
wake tick has arrived. */

/* A blocked thread waiting for its wake-up tick (lives on the
sleeping thread's stack while it is blocked). */
#[repr(C)]
struct SleepElem {
    wake_tick: i64,      /* Tick at which to wake up. */
    thread: *mut Thread, /* The sleeping thread. */
    elem: ListElem,      /* SLEEP_LIST element. */
}

/* List of sleeping threads, sorted by increasing wake_tick.
Zero-initialized like C's uninitialized file-scope static and
made valid by list_init() in timer_init() (timer interrupts are
only registered there, so it is never touched before). */
static mut SLEEP_LIST: List = unsafe { core::mem::zeroed() };

/* SLEEP_LIST order: increasing wake_tick (C list_less_func). */
unsafe fn sleep_less(a: *const ListElem, b: *const ListElem, _aux: *mut ()) -> bool {
    unsafe {
        let sa = list::list_entry::<SleepElem>(
            a as *mut ListElem,
            core::mem::offset_of!(SleepElem, elem),
        );
        let sb = list::list_entry::<SleepElem>(
            b as *mut ListElem,
            core::mem::offset_of!(SleepElem, elem),
        );
        (*sa).wake_tick < (*sb).wake_tick
    }
}

/* Sets up the 8254 Programmable Interval Timer (PIT) to
interrupt PIT_FREQ times per second, and registers the
corresponding interrupt. */
pub fn timer_init() {
    /* 8254 input frequency divided by TIMER_FREQ, rounded to
    nearest. */
    let count: u16 = ((1193180 + TIMER_FREQ / 2) / TIMER_FREQ) as u16;

    unsafe {
        list::list_init(core::ptr::addr_of_mut!(SLEEP_LIST));

        outb(0x43, 0x34); /* CW: counter 0, LSB then MSB, mode 2, binary. */
        outb(0x40, (count & 0xff) as u8);
        outb(0x40, (count >> 8) as u8);

        intr_register_ext(0x20, timer_interrupt, b"8254 Timer\0".as_ptr());
    }
}

/* Calibrates loops_per_tick, used to implement brief delays. */
pub fn timer_calibrate() {
    unsafe {
        ASSERT!(intr_get_level() == IntrLevel::On);
        printf!("Calibrating timer...  ");

        /* Approximate loops_per_tick as the largest power-of-two
        still less than one timer tick. */
        *core::ptr::addr_of_mut!(LOOPS_PER_TICK) = 1u32 << 10;
        while !too_many_loops(LOOPS_PER_TICK.wrapping_shl(1)) {
            *core::ptr::addr_of_mut!(LOOPS_PER_TICK) <<= 1;
            ASSERT!(*core::ptr::addr_of!(LOOPS_PER_TICK) != 0);
        }

        /* Refine the next 8 bits of loops_per_tick. */
        let high_bit = LOOPS_PER_TICK;
        let mut test_bit = high_bit >> 1;
        while test_bit != high_bit >> 10 {
            if !too_many_loops(high_bit | test_bit) {
                *core::ptr::addr_of_mut!(LOOPS_PER_TICK) |= test_bit;
            }
            test_bit >>= 1;
        }

        printf!(
            "%'lu loops/s.\n",
            *core::ptr::addr_of!(LOOPS_PER_TICK) as u64 * TIMER_FREQ as u64
        );
    }
}

/* Returns the number of timer ticks since the OS booted. */
pub fn timer_ticks() -> i64 {
    let old_level = intr_disable();
    let t = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(TICKS)) };
    intr_set_level(old_level);
    t
}

/* Returns the number of timer ticks elapsed since THEN, which
should be a value once returned by timer_ticks(). */
pub fn timer_elapsed(then: i64) -> i64 {
    timer_ticks() - then
}

/* Suspends execution for approximately TICKS timer ticks.

Unlike the C original, which spun on thread_yield() until the
ticks elapsed, this blocks the calling thread: it is inserted
into SLEEP_LIST ordered by wake tick and blocked until
timer_interrupt() unblocks it.  Thus a sleeping thread consumes
no CPU time. */
pub fn timer_sleep(ticks: i64) {
    if ticks <= 0 {
        /* C's `while (timer_elapsed (start) < ticks)' never
        iterates for TICKS <= 0: return immediately, just like
        the original. */
        return;
    }

    ASSERT!(intr_get_level() == IntrLevel::On);
    ASSERT!(!intr_context());

    let old_level = intr_disable();
    unsafe {
        let mut sleeping_thread = SleepElem {
            wake_tick: timer_ticks() + ticks,
            thread: thread_current(),
            /* List elements need no initialization before being
            linked, as in C. */
            elem: core::mem::zeroed(),
        };

        /* Add to the sleep list, keeping it sorted by wake
        tick. */
        list::list_insert_ordered(
            core::ptr::addr_of_mut!(SLEEP_LIST),
            core::ptr::addr_of_mut!(sleeping_thread.elem),
            sleep_less,
            core::ptr::null_mut(),
        );

        /* Sleep until timer_interrupt() wakes us. */
        thread_block();
    }
    intr_set_level(old_level);
}

/* Suspends execution for approximately MS milliseconds. */
pub fn timer_msleep(ms: i64) {
    real_time_sleep(ms, 1000);
}

/* Suspends execution for approximately US microseconds. */
pub fn timer_usleep(us: i64) {
    real_time_sleep(us, 1000 * 1000);
}

/* Suspends execution for approximately NS nanoseconds. */
pub fn timer_nsleep(ns: i64) {
    real_time_sleep(ns, 1000 * 1000 * 1000);
}

/* Prints timer statistics. */
pub fn timer_print_stats() {
    printf!("Timer: %ld ticks\n", timer_ticks());
}

/* Timer interrupt handler. */
extern "C" fn timer_interrupt(_args: *mut IntrFrame) {
    unsafe {
        *core::ptr::addr_of_mut!(TICKS) += 1;
        thread_tick();
        wake_sleepers();
    }
}

/* Wakes every sleeping thread whose wake tick has arrived.  Runs
in interrupt context with interrupts off, so the raw list
operations and thread_unblock() are safe. */
unsafe fn wake_sleepers() {
    unsafe {
        let sleep_list = core::ptr::addr_of_mut!(SLEEP_LIST);
        let now = core::ptr::read_volatile(core::ptr::addr_of!(TICKS));

        while !list::list_empty(sleep_list) {
            let e = list::list_begin(sleep_list);
            let st = list::list_entry::<SleepElem>(e, core::mem::offset_of!(SleepElem, elem));
            if (*st).wake_tick > now {
                break;
            }
            list::list_remove(e);
            thread_unblock((*st).thread);
        }
    }
}

/* Returns true if LOOPS iterations waits for more than one timer
tick, otherwise false. */
unsafe fn too_many_loops(loops: u32) -> bool {
    unsafe {
        /* Wait for a timer tick. */
        let mut start = core::ptr::read_volatile(core::ptr::addr_of!(TICKS));
        while core::ptr::read_volatile(core::ptr::addr_of!(TICKS)) == start {
            continue;
        }

        /* Run LOOPS loops. */
        start = core::ptr::read_volatile(core::ptr::addr_of!(TICKS));
        busy_wait(loops as i64);

        /* If the tick count changed, we iterated too long. */
        core::ptr::read_volatile(core::ptr::addr_of!(TICKS)) != start
    }
}

/* Iterates through a simple loop LOOPS times, for implementing
brief delays.

Marked NO_INLINE because code alignment can significantly
affect timings, so that if this function was inlined
differently in different places the results would be difficult
to predict. */
#[inline(never)]
unsafe fn busy_wait(mut loops: i64) {
    unsafe {
        while loops > 0 {
            /* C: barrier (). */
            core::ptr::read_volatile(&loops);
            loops -= 1;
        }
    }
}

/* Sleep for approximately NUM/DENOM seconds. */
fn real_time_sleep(num: i64, denom: i32) {
    /* Convert NUM/DENOM seconds into timer ticks, rounding down.

    (NUM / DENOM) s
    ---------------------- = NUM * TIMER_FREQ / DENOM ticks.
    1 s / TIMER_FREQ ticks
    */
    let ticks = num * TIMER_FREQ as i64 / denom as i64;

    unsafe {
        ASSERT!(intr_get_level() == IntrLevel::On);
        if ticks > 0 {
            /* We're waiting for at least one full timer tick.  Use
            timer_sleep() because it will yield the CPU to other
            processes. */
            timer_sleep(ticks);
        } else {
            /* Otherwise, use a busy-wait loop for more accurate
            sub-tick timing.  We scale the numerator and denominator
            down by 1000 to avoid the possibility of overflow. */
            ASSERT!(denom % 1000 == 0);
            busy_wait(
                LOOPS_PER_TICK as i64 * num / 1000 * TIMER_FREQ as i64 / (denom / 1000) as i64,
            );
        }
    }
}
