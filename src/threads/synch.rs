//! Port of `threads/synch.c` — semaphores, locks, condition variables.
//!
//! Extended over the C baseline for priority scheduling and priority
//! donation (project 1): semaphore waiter lists are kept ordered by
//! descending thread priority, so `sema_up` always wakes the
//! highest-priority waiter (and preempts when it outranks the current
//! thread); `lock_acquire`/`lock_release` trigger donation through
//! `thread::thread_donate_priority()`; `cond_signal` wakes the waiter
//! whose blocked thread has the highest priority.

use crate::{
    lib::kernel::list::{
        List, ListElem, list_empty, list_entry, list_front, list_init, list_insert_ordered,
        list_max, list_pop_front, list_push_back, list_remove,
    },
    threads::{
        interrupt::{intr_context, intr_disable, intr_set_level},
        thread::{
            Thread, thread_block, thread_cmp_priority, thread_current, thread_donate_priority,
            thread_unblock, thread_yield,
        },
    },
};

/* A counting semaphore. */
#[repr(C)]
pub struct Semaphore {
    pub value: u32,    /* Current value. */
    pub waiters: List, /* List of waiting threads. */
}

/* Lock. */
#[repr(C)]
pub struct Lock {
    pub holder: *mut Thread,  /* Thread holding lock (for debugging). */
    pub semaphore: Semaphore, /* Binary semaphore controlling access. */
    /// Link in the holder's `acquired_locks` list (priority
    /// donation bookkeeping; managed by lock_acquire/release).
    pub elem: ListElem,
}

impl Lock {
    /// Offset of `elem` within Lock (for list_entry on the holder's
    /// acquired-locks list).
    pub const ELEM_OFFSET: usize = core::mem::offset_of!(Lock, elem);
}
/// C-compat alias used by thread.rs.
pub const LOCK_ELEM_OFFSET: usize = Lock::ELEM_OFFSET;

/* Condition variable. */
#[repr(C)]
pub struct Condition {
    pub waiters: List, /* List of waiting threads. */
}

impl Condition {
    /// All-zero condition (call `cond_init` before use).
    pub const EMPTY: Condition = Condition {
        waiters: List::EMPTY,
    };
}

/* One semaphore in a list. */
#[repr(C)]
struct SemaphoreElem {
    elem: ListElem,       /* List element. */
    semaphore: Semaphore, /* This semaphore. */
}

/* Initializes semaphore SEMA to VALUE.  A semaphore is a
nonnegative integer along with two atomic operators for
manipulating it:

- down or "P": wait for the value to become positive, then
decrement it.

- up or "V": increment the value (and wake up one waiting
thread, if any). */
pub unsafe fn sema_init(sema: *mut Semaphore, value: u32) {
    unsafe {
        crate::ASSERT!(sema != core::ptr::null_mut());

        (*sema).value = value;
        list_init(&raw mut (*sema).waiters);
    }
}

/* Down or "P" operation on a semaphore.  Waits for SEMA's value
to become positive and then atomically decrements it.

This function may sleep, so it must not be called within an
interrupt handler.  This function may be called with
interrupts disabled, but if it sleeps then the next scheduled
thread will probably turn interrupts back on. This is
sema_down function. */
pub unsafe fn sema_down(sema: *mut Semaphore) {
    unsafe {
        crate::ASSERT!(sema != core::ptr::null_mut());
        crate::ASSERT!(!intr_context());

        let old_level = intr_disable();
        while (*sema).value == 0 {
            list_push_back(&raw mut (*sema).waiters, &raw mut (*thread_current()).elem);
            /* We are now visible on the waiters list: if this
            semaphore belongs to a lock held by another thread,
            propagate priority donation up the holder chain before
            going to sleep. */
            thread_donate_priority();
            thread_block();
        }
        (*sema).value -= 1;
        intr_set_level(old_level);
    }
}

/* Down or "P" operation on a semaphore, but only if the
semaphore is not already 0.  Returns true if the semaphore is
decremented, false otherwise.

This function may be called from an interrupt handler. */
pub unsafe fn sema_try_down(sema: *mut Semaphore) -> bool {
    unsafe {
        crate::ASSERT!(sema != core::ptr::null_mut());

        let old_level = intr_disable();

        let success;
        if (*sema).value > 0 {
            (*sema).value -= 1;
            success = true;
        } else {
            success = false;
        }
        intr_set_level(old_level);

        success
    }
}

/* list_max "less" twin of thread_cmp_priority: true when A's
thread has strictly LOWER priority than B's (so list_max
selects the highest-priority waiter). */
unsafe fn thread_less_priority(a: *const ListElem, b: *const ListElem, aux: *mut ()) -> bool {
    unsafe { thread_cmp_priority(b, a, aux) }
}

/* Up or "V" operation on a semaphore.  Increments SEMA's value
and wakes up one thread of those waiting for SEMA, if any.

This function may be called from an interrupt handler. */
pub unsafe fn sema_up(sema: *mut Semaphore) {
    unsafe {
        crate::ASSERT!(sema != core::ptr::null_mut());

        let old_level = intr_disable();

        /* Increment the value BEFORE waking (and preempting): the
        woken thread re-checks `value == 0` in its sema_down loop,
        and a preemption triggered below must not observe the old
        value or it would re-block with no one left to wake it. */
        (*sema).value += 1;

        if !list_empty(&raw const (*sema).waiters) {
            /* Wake the highest-priority waiter.  Selecting with
            list_max (rather than keeping the list sorted at
            insert time) stays correct when a waiter's priority
            changes while it sleeps, e.g. through donation. */
            let max = list_max(
                &raw const (*sema).waiters,
                thread_less_priority,
                core::ptr::null_mut(),
            );
            list_remove(max);
            let t =
                list_entry::<Thread>(max, core::mem::offset_of!(Thread, elem));
            thread_unblock(t);

            /* If the woken thread has higher priority than the
            running thread, yield the CPU. */
            let cur = thread_current();
            if thread_cmp_priority(
                core::ptr::addr_of!((*t).elem),
                core::ptr::addr_of!((*cur).elem),
                core::ptr::null_mut(),
            ) {
                thread_yield();
            }
        }
        intr_set_level(old_level);
    }
}

/* Initializes LOCK.  A lock can be held by at most a single
thread at any given time.  Our locks are not "recursive", that
is, it is an error for the thread currently holding a lock to
try to acquire that lock.

A lock is a specialization of a semaphore with an initial
value of 1.  The difference between a lock and such a
semaphore is twofold.  First, a semaphore can have a value
greater than 1, but a lock can only be owned by a single
thread at a time.  Second, a semaphore does not have an owner,
meaning that one thread can "down" the semaphore and then
another one "up" it, but with a lock the same thread must both
acquire and release it.  When these restrictions prove
onerous, that's a good sign that a semaphore should be used,
instead. */
pub unsafe fn lock_init(lock: *mut Lock) {
    unsafe {
        crate::ASSERT!(lock != core::ptr::null_mut());

        (*lock).holder = core::ptr::null_mut();
        (*lock).elem.prev = core::ptr::null_mut();
        (*lock).elem.next = core::ptr::null_mut();
        sema_init(&raw mut (*lock).semaphore, 1);
    }
}

/* Acquires LOCK, sleeping until it becomes available if
necessary.  The lock must not already be held by the current
thread.

This function may sleep, so it must not be called within an
interrupt handler.  This function may be called with
interrupts disabled, but interrupts will be turned back on if
we need to sleep. */
pub unsafe fn lock_acquire(lock: *mut Lock) {
    unsafe {
        crate::ASSERT!(lock != core::ptr::null_mut());
        crate::ASSERT!(!intr_context());
        crate::ASSERT!(!lock_held_by_current_thread(lock));

        if !(*lock).holder.is_null() {
            /* The lock is held by another thread: we are about to
            join its waiters and sleep.  Record the lock we are
            waiting on and trigger priority donation so the holder
            (and, transitively, anyone the holder is waiting on)
            runs at our priority until we get the lock back. */
            (*thread_current()).waiting_lock = lock;
            thread_donate_priority();
        }
        sema_down(&raw mut (*lock).semaphore);
        (*thread_current()).waiting_lock = core::ptr::null_mut();
        (*lock).holder = thread_current();
        /* We hold it now: it joins our acquired list for future
        donation refreshes. */
        list_push_back(
            &raw mut (*thread_current()).acquired_locks,
            &raw mut (*lock).elem,
        );
    }
}

/* Tries to acquires LOCK and returns true if successful or false
on failure.  The lock must not already be held by the current
thread.

This function will not sleep, so it may be called within an
interrupt handler. */
pub unsafe fn lock_try_acquire(lock: *mut Lock) -> bool {
    unsafe {
        crate::ASSERT!(lock != core::ptr::null_mut());
        crate::ASSERT!(!lock_held_by_current_thread(lock));

        let success = sema_try_down(&raw mut (*lock).semaphore);
        if success {
            (*lock).holder = thread_current();
        }
        success
    }
}

/* Releases LOCK, which must be owned by the current thread.
This is lock_release function.

An interrupt handler cannot acquire a lock, so it does not
make sense to try to release a lock within an interrupt
handler. */
pub unsafe fn lock_release(lock: *mut Lock) {
    unsafe {
        crate::ASSERT!(lock != core::ptr::null_mut());
        crate::ASSERT!(lock_held_by_current_thread(lock));

        (*lock).holder = core::ptr::null_mut();
        /* No longer on our acquired list. */
        list_remove(&raw mut (*lock).elem);
        sema_up(&raw mut (*lock).semaphore);

        /* The highest-priority waiter no longer donates to us, so
        our effective priority may have dropped: refresh it (the
        donation refresh itself lives in thread.rs), and yield if
        we dropped below a ready thread. */
        let before = (*thread_current()).priority;
        crate::threads::thread::refresh_priority(thread_current());
        if (*thread_current()).priority < before {
            thread_yield();
        }
    }
}

/* Returns true if the current thread holds LOCK, false
otherwise.  (Note that testing whether some other thread holds
a lock would be racy.) */
pub unsafe fn lock_held_by_current_thread(lock: *const Lock) -> bool {
    unsafe {
        crate::ASSERT!(lock != core::ptr::null_mut());

        (*lock).holder == thread_current()
    }
}

/* Initializes condition variable COND.  A condition variable
allows one piece of code to signal a condition and cooperating
code to receive the signal and act upon it. */
pub unsafe fn cond_init(cond: *mut Condition) {
    unsafe {
        crate::ASSERT!(cond != core::ptr::null_mut());

        list_init(&raw mut (*cond).waiters);
    }
}

/* Atomically releases LOCK and waits for COND to be signaled by
some other piece of code.  After COND is signaled, LOCK is
reacquired before returning.  LOCK must be held before calling
this function.

The monitor implemented by this function is "Mesa" style, not
"Hoare" style, that is, sending and receiving a signal are not
an atomic operation.  Thus, typically the caller must recheck
the condition after the wait completes and, if necessary, wait
again.

A given condition variable is associated with only a single
lock, but one lock may be associated with any number of
condition variables.  That is, there is a one-to-many mapping
from locks to condition variables.

This function may sleep, so it must not be called within an
interrupt handler.  This function may be called with
interrupts disabled, but interrupts will be turned back on if
we need to sleep. */
pub unsafe fn cond_wait(cond: *mut Condition, lock: *mut Lock) {
    unsafe {
        /* One semaphore in a list, on our stack: it lives until we
        are signaled (we sleep inside sema_down below). */
        let mut waiter: SemaphoreElem = core::mem::zeroed();

        crate::ASSERT!(cond != core::ptr::null_mut());
        crate::ASSERT!(lock != core::ptr::null_mut());
        crate::ASSERT!(!intr_context());
        crate::ASSERT!(lock_held_by_current_thread(lock));

        sema_init(&raw mut waiter.semaphore, 0);
        list_push_back(&raw mut (*cond).waiters, &raw mut waiter.elem);
        lock_release(lock);
        sema_down(&raw mut waiter.semaphore);
        lock_acquire(lock);
    }
}

/* List "less" function for condition-variable waiters: orders two
`SemaphoreElem`s by the priority of the single thread blocked on
each one's semaphore (so `list_max` with this comparator finds
the highest-priority waiter).  A waiter whose semaphore has no
blocked thread yet — the small window in cond_wait between
inserting the element and sleeping on it — counts as lowest
priority. */
unsafe fn sema_waiter_priority_less(a: *const ListElem, b: *const ListElem, _aux: *mut ()) -> bool {
    unsafe {
        let sa = list_entry::<SemaphoreElem>(
            a as *mut ListElem,
            core::mem::offset_of!(SemaphoreElem, elem),
        );
        let sb = list_entry::<SemaphoreElem>(
            b as *mut ListElem,
            core::mem::offset_of!(SemaphoreElem, elem),
        );

        let wa = list_empty(&raw const (*sa).semaphore.waiters);
        let wb = list_empty(&raw const (*sb).semaphore.waiters);
        if wa {
            /* A has no blocked thread (lowest); it is less than B
            unless B is also empty. */
            return !wb;
        }
        if wb {
            return false; /* B is the empty (lowest) one. */
        }

        /* Each waiter semaphore has exactly one blocked thread on
        it.  thread_cmp_priority(x, y) is true when x's priority
        is higher than y's, i.e. "x sorts before y"; less(a, b)
        means a < b, so we ask whether b sorts before a. */
        let ta = list_front(&raw const (*sa).semaphore.waiters);
        let tb = list_front(&raw const (*sb).semaphore.waiters);
        thread_cmp_priority(tb, ta, core::ptr::null_mut())
    }
}

/* If any threads are waiting on COND (protected by LOCK), then
this function signals one of them to wake up from its wait.
LOCK must be held before calling this function.

An interrupt handler cannot acquire a lock, so it does not
make sense to try to signal a condition variable within an
interrupt handler. */
pub unsafe fn cond_signal(cond: *mut Condition, lock: *mut Lock) {
    unsafe {
        crate::ASSERT!(cond != core::ptr::null_mut());
        crate::ASSERT!(lock != core::ptr::null_mut());
        crate::ASSERT!(!intr_context());
        crate::ASSERT!(lock_held_by_current_thread(lock));

        if !list_empty(&raw const (*cond).waiters) {
            /* Signal the waiter whose blocked thread has the
            highest priority. */
            let max = list_max(
                &raw const (*cond).waiters,
                sema_waiter_priority_less,
                core::ptr::null_mut(),
            );
            list_remove(max);
            let waiter =
                list_entry::<SemaphoreElem>(max, core::mem::offset_of!(SemaphoreElem, elem));
            sema_up(&raw mut (*waiter).semaphore);
        }
    }
}

/* Wakes up all threads, if any, waiting on COND (protected by
LOCK).  LOCK must be held before calling this function.

An interrupt handler cannot acquire a lock, so it does not
make sense to try to signal a condition variable within an
interrupt handler. */
pub unsafe fn cond_broadcast(cond: *mut Condition, lock: *mut Lock) {
    unsafe {
        crate::ASSERT!(cond != core::ptr::null_mut());
        crate::ASSERT!(lock != core::ptr::null_mut());

        while !list_empty(&raw const (*cond).waiters) {
            cond_signal(cond, lock);
        }
    }
}

impl Semaphore {
    /// All-zero semaphore (call `sema_init` before use).
    pub const EMPTY: Semaphore = Semaphore {
        value: 0,
        waiters: List::EMPTY,
    };
}

impl Lock {
    /// All-zero lock (call `lock_init` before use).
    pub const EMPTY: Lock = Lock {
        holder: core::ptr::null_mut(),
        semaphore: Semaphore::EMPTY,
        elem: ListElem {
            prev: core::ptr::null_mut(),
            next: core::ptr::null_mut(),
        },
    };
}
