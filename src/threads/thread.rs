//! Port of `threads/thread.c` — the thread system.
//!
//! Includes the project-1 assignments: strict priority scheduling
//! (ready list kept ordered by priority) with priority donation
//! through acquired locks, and the mlfqs advanced scheduler.

use core::arch::global_asm;

use crate::{
    lib::{
        kernel::list::{
            List, ListElem, list_empty, list_entry, list_init, list_insert_ordered, list_pop_front,
            list_push_back, list_remove,
        },
        string::{memset, strlcpy},
    },
    printf,
    threads::{
        interrupt::{
            IntrFrame, IntrLevel, intr_context, intr_disable, intr_enable, intr_get_level,
            intr_yield_on_return,
        },
        loader::{SEL_KCSEG, SEL_KDSEG},
        palloc::{PAL_ZERO, palloc_free_page, palloc_get_page},
        synch::{Lock, Semaphore, lock_acquire, lock_init, lock_release, sema_down, sema_init},
        vaddr::{PGSIZE, pg_round_down},
    },
};

/* Random value for struct thread's `magic' member.
Used to detect stack overflow.  See the big comment at the top
of thread.h for details. */
pub const THREAD_MAGIC: u32 = 0xcd6abf4b;

/* Random value for basic thread.  Do not modify this value. */
pub const THREAD_BASIC: u32 = 0xd42df210;

/* Thread priorities. */
pub const PRI_MIN: i32 = 0; /* Lowest priority. */
pub const PRI_DEFAULT: i32 = 31; /* Default priority. */
pub const PRI_MAX: i32 = 63; /* Highest priority. */

/* Thread identifier type. */
pub type Tid = i32;
pub const TID_ERROR: Tid = -1; /* Error value for tid_t. */

/* States in a thread's life cycle. */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ThreadStatus {
    Running = 0, /* Running thread. */
    Ready,       /* Not running but ready to run. */
    Blocked,     /* Waiting for an event to trigger. */
    Dying,       /* About to be destroyed. */
}

/// A kernel thread or user process.  Sits at offset 0 of its own
/// 4 kB page; the page above it is the thread's kernel stack.
#[repr(C)]
pub struct Thread {
    /* Owned by thread.rs. */
    pub tid: Tid,             /* Thread identifier. */
    pub status: ThreadStatus, /* Thread state. */
    pub name: [u8; 16],       /* Name (for debugging purposes). */
    pub priority: i32,        /* Effective priority. */

    /* Shared between thread.rs and synch.rs. */
    pub elem: ListElem, /* Ready-list / semaphore-waiter element. */

    /* Owned by userprog/process.rs. */
    pub pml4: *mut u64, /* Page map level 4 */

    /* --- Project 2 (userprog) bookkeeping. --- */
    /// Child-process records (List of process::ChildInfo::elem),
    /// for process_wait.
    pub children: List,
    /// This process's own child-record in its parent's list
    /// (filled in by whoever creates the thread's aux before it
    /// runs); the record outlives this thread's page because the
    /// parent allocates it.
    pub child_info: *mut core::ffi::c_void,
    /// Exit status delivered to the parent via ChildInfo.
    pub exit_status: i32,
    /// Open file table; index 0/1 are stdin/stdout by convention.
    pub fd_table: [*mut crate::filesys::file::File; 128],
    /// Next file descriptor to hand out (starts at 2).
    pub next_fd: i32,
    /// This process's executable (write-denied while running).
    pub executable: *mut crate::filesys::file::File,

    /* Owned by vm/ (supplemental page table). */
    pub spt: crate::vm::SupplementalPageTable,

    /* --- Project 1 (priority donation) bookkeeping. --- */
    /// Locks this thread currently holds (List of *mut Lock via
    /// Lock::elem).
    pub acquired_locks: List,
    /// The lock this thread is blocked waiting on, or null.
    pub waiting_lock: *mut Lock,
    /// Priority before any donation (restored on release).
    pub original_priority: i32,

    /* --- Project 1 (mlfqs) bookkeeping. --- */
    pub nice: i32,
    pub recent_cpu: i64, /* 17.14 fixed-point */

    /* Owned by thread.rs. */
    pub tf: IntrFrame, /* Information for switching */
    pub magic: u32,    /* Detects stack overflow. */
}

impl Thread {
    /// Offset of `elem` within Thread (for list_entry).
    pub const ELEM_OFFSET: usize = core::mem::offset_of!(Thread, elem);
}

/* List of processes in THREAD_READY state, that is, processes
that are ready to run but not actually running. */
static mut READY_LIST: List = List {
    head: ListElem {
        prev: core::ptr::null_mut(),
        next: core::ptr::null_mut(),
    },
    tail: ListElem {
        prev: core::ptr::null_mut(),
        next: core::ptr::null_mut(),
    },
};

/* Idle thread. */
static mut IDLE_THREAD: *mut Thread = core::ptr::null_mut();

/* Initial thread, the thread running init.rs:main(). */
static mut INITIAL_THREAD: *mut Thread = core::ptr::null_mut();

/* Lock used by allocate_tid(). */
static mut TID_LOCK: Lock = Lock::EMPTY;

/* Thread destruction requests. */
static mut DESTRUCTION_REQ: List = List {
    head: ListElem {
        prev: core::ptr::null_mut(),
        next: core::ptr::null_mut(),
    },
    tail: ListElem {
        prev: core::ptr::null_mut(),
        next: core::ptr::null_mut(),
    },
};

/* Statistics. */
static mut IDLE_TICKS: i64 = 0; /* # of timer ticks spent idle. */
static mut KERNEL_TICKS: i64 = 0; /* # of timer ticks in kernel threads. */
static mut USER_TICKS: i64 = 0; /* # of timer ticks in user programs. */

/* Scheduling. */
pub const TIME_SLICE: u32 = 4; /* # of timer ticks to give each thread. */
static mut THREAD_TICKS: u32 = 0; /* # of timer ticks since last yield. */

/* If false (default), use round-robin scheduler.
If true, use multi-level feedback queue scheduler.
Controlled by kernel command-line option "-mlfqs". */
pub static mut THREAD_MLFQS: bool = false;

/* mlfqs load average (17.14 fixed-point). */
pub static mut LOAD_AVG: i64 = 0;

/* EFLAGS.IF. */
const FLAG_IF: u64 = 1 << 9;

/* Returns true if T appears to point to a valid thread. */
#[inline]
pub unsafe fn is_thread(t: *const Thread) -> bool {
    unsafe { !t.is_null() && (*t).magic == THREAD_MAGIC }
}

/* Returns the running thread.
Read the CPU's stack pointer `rsp', and then round that down to
the start of a page.  Since `struct thread' is always at the
beginning of a page and the stack pointer is somewhere in the
middle, this locates the current thread. */
#[inline]
pub unsafe fn running_thread() -> *mut Thread {
    unsafe { pg_round_down(crate::intrinsic::rrsp()) as *mut Thread }
}

// Global descriptor table for thread_start.  Because the gdt is
// set up after thread_init, we first load a temporary gdt.
static mut GDT3: [u64; 3] = [0, 0x00af9a000000ffff, 0x00cf92000000ffff];

/* Initializes the threading system by transforming the code
that's currently running into a thread.  This can't work in
general and it is possible in this case only because loader.S
was careful to put the bottom of the stack at a page boundary.

Also initializes the run queue and the tid lock.

After calling this function, be sure to initialize the page
allocator before trying to create any threads with
thread_create().

It is not safe to call thread_current() until this function
finishes. */
pub unsafe fn thread_init() {
    unsafe {
        crate::ASSERT!(intr_get_level() == IntrLevel::Off);

        /* Reload the temporal gdt for the kernel.  This gdt does
        not include the user context.  The kernel will rebuild the
        gdt with user context in gdt_init(). */
        let gdt_ds = crate::intrinsic::DescPtr {
            size: (core::mem::size_of_val(&GDT3) - 1) as u16,
            address: GDT3.as_ptr() as u64,
        };
        crate::intrinsic::lgdt(&gdt_ds);

        /* Init the global thread context. */
        lock_init(&raw mut TID_LOCK);
        list_init(&raw mut READY_LIST);
        list_init(&raw mut DESTRUCTION_REQ);

        /* Set up a thread structure for the running thread. */
        INITIAL_THREAD = running_thread();
        init_thread(INITIAL_THREAD, b"main\0".as_ptr(), PRI_DEFAULT);
        (*INITIAL_THREAD).status = ThreadStatus::Running;
        (*INITIAL_THREAD).tid = allocate_tid();
    }
}

/* Starts preemptive thread scheduling by enabling interrupts.
Also creates the idle thread. */
pub unsafe fn thread_start() {
    unsafe {
        /* Create the idle thread. */
        let mut idle_started = Semaphore::EMPTY;
        sema_init(&raw mut idle_started, 0);
        thread_create(
            b"idle\0".as_ptr(),
            PRI_MIN,
            Some(idle),
            &mut idle_started as *mut Semaphore as *mut (),
        );

        /* Start preemptive thread scheduling. */
        intr_enable();

        /* Wait for the idle thread to initialize idle_thread. */
        sema_down(&raw mut idle_started);
    }
}

/* Called by the timer interrupt handler at each timer tick.
Thus, this function runs in an external interrupt context. */
pub unsafe fn thread_tick() {
    unsafe {
        let t = thread_current();

        /* Update statistics. */
        if t == IDLE_THREAD {
            IDLE_TICKS += 1;
        } else if !(*t).pml4.is_null() {
            USER_TICKS += 1;
        } else {
            KERNEL_TICKS += 1;
        }

        /* mlfqs: every tick bumps the running thread's recent_cpu
        (once per tick at the start of execution). */
        if THREAD_MLFQS {
            (*t).recent_cpu = fp_add_int((*t).recent_cpu, 1);

            /* Once per second: recalculate load_avg and every
            thread's recent_cpu. */
            let ticks = crate::devices::timer::timer_ticks();
            if ticks > 0 && (ticks as u64) % crate::devices::timer::TIMER_FREQ as u64 == 0 {
                mlfqs_update_load_avg();
                mlfqs_foreach_update_recent_cpu();
            }
            /* Recompute the priorities of the running thread every
            fourth tick. */
            if (ticks as u64) % 4 == 0 {
                mlfqs_update_priority(t);
            }
        }

        /* Enforce preemption. */
        THREAD_TICKS += 1;
        if THREAD_TICKS >= TIME_SLICE {
            intr_yield_on_return();
        }
    }
}

/* Prints thread statistics. */
pub unsafe fn thread_print_stats() {
    unsafe {
        printf!(
            "Thread: %lld idle ticks, %lld kernel ticks, %lld user ticks\n",
            IDLE_TICKS,
            KERNEL_TICKS,
            USER_TICKS
        );
    }
}

/* Creates a new kernel thread named NAME with the given initial
PRIORITY, which executes FUNCTION passing AUX as the argument,
and adds it to the ready queue.  Returns the thread identifier
for the new thread, or TID_ERROR if creation fails. */
pub unsafe fn thread_create(
    name: *const u8,
    priority: i32,
    function: Option<unsafe extern "C" fn(*mut ())>,
    aux: *mut (),
) -> Tid {
    unsafe {
        crate::ASSERT!(function.is_some());

        /* Allocate thread. */
        let t = palloc_get_page(PAL_ZERO) as *mut Thread;
        if t.is_null() {
            return TID_ERROR;
        }

        /* Initialize thread. */
        init_thread(t, name, priority);
        let tid = allocate_tid();
        (*t).tid = tid;

        /* Call the kernel_thread if it is scheduled.
        Note) rdi is 1st argument, and rsi is 2nd argument. */
        (*t).tf.rip = kernel_thread as usize as u64;
        (*t).tf.R.rdi = function.map(|f| f as usize).unwrap_or(0) as u64;
        (*t).tf.R.rsi = aux as u64;
        (*t).tf.ds = SEL_KDSEG;
        (*t).tf.es = SEL_KDSEG;
        (*t).tf.ss = SEL_KDSEG;
        (*t).tf.cs = SEL_KCSEG;
        (*t).tf.eflags = FLAG_IF;

        /* Add to run queue. */
        thread_unblock(t);

        /* If the new thread has higher priority, yield the CPU. */
        if (*t).priority > (*thread_current()).priority {
            thread_yield();
        }

        tid
    }
}

/* Puts the current thread to sleep.  It will not be scheduled
again until awoken by thread_unblock().

This function must be called with interrupts turned off. */
pub unsafe fn thread_block() {
    unsafe {
        crate::ASSERT!(!intr_context());
        crate::ASSERT!(intr_get_level() == IntrLevel::Off);
        (*thread_current()).status = ThreadStatus::Blocked;
        schedule();
    }
}

/* Transitions a blocked thread T to the ready-to-run state.
This is an error if T is not blocked.

This function does not preempt the running thread. */
pub unsafe fn thread_unblock(t: *mut Thread) {
    unsafe {
        crate::ASSERT!(is_thread(t));

        let old_level = intr_disable();
        crate::ASSERT!((*t).status == ThreadStatus::Blocked);
        /* Keep the ready list ordered by (descending) priority. */
        list_insert_ordered(
            &raw mut READY_LIST,
            &raw mut (*t).elem,
            thread_cmp_priority,
            core::ptr::null_mut(),
        );
        (*t).status = ThreadStatus::Ready;
        intr_enable_if(old_level);
    }
}

#[inline]
unsafe fn intr_enable_if(old: IntrLevel) {
    unsafe {
        crate::threads::interrupt::intr_set_level(old);
    }
}

/* Returns the name of the running thread. */
pub unsafe fn thread_name() -> *const u8 {
    unsafe { (*thread_current()).name.as_ptr() }
}

/* Returns the running thread.  This is running_thread() plus a
couple of sanity checks. */
pub unsafe fn thread_current() -> *mut Thread {
    unsafe {
        let t = running_thread();

        /* Make sure T is really a thread.  If either of these
        assertions fire, then your thread may have overflowed its
        stack. */
        crate::ASSERT!(is_thread(t));
        crate::ASSERT!((*t).status == ThreadStatus::Running);

        t
    }
}

/* Returns the running thread's tid. */
pub unsafe fn thread_tid() -> Tid {
    unsafe { (*thread_current()).tid }
}

/* Deschedules the current thread and destroys it.  Never returns
to the caller. */
pub unsafe fn thread_exit() -> ! {
    unsafe {
        crate::ASSERT!(!intr_context());

        crate::userprog::process::process_exit();

        /* Just set our status to dying and schedule another
        process.  We will be destroyed during the call to
        schedule(). */
        intr_disable();
        do_schedule(ThreadStatus::Dying);
        unreachable!("thread_exit returned");
    }
}

/* Yields the CPU.  The current thread is not put to sleep and
may be scheduled again immediately at the scheduler's whim. */
pub unsafe fn thread_yield() {
    unsafe {
        let curr = thread_current();

        crate::ASSERT!(!intr_context());

        let old_level = intr_disable();
        if curr != IDLE_THREAD {
            list_insert_ordered(
                &raw mut READY_LIST,
                &raw mut (*curr).elem,
                thread_cmp_priority,
                core::ptr::null_mut(),
            );
        }
        do_schedule(ThreadStatus::Ready);
        crate::threads::interrupt::intr_set_level(old_level);
    }
}

/* Sets the current thread's priority to NEW_PRIORITY. */
pub unsafe fn thread_set_priority(new_priority: i32) {
    unsafe {
        let curr = thread_current();
        (*curr).original_priority = new_priority;

        /* If not holding any locks (or the new priority is still
        the highest), refresh the effective priority directly. */
        let donated = max_donated_priority(curr);
        let new_effective = if new_priority > donated {
            new_priority
        } else {
            donated
        };

        if new_effective != (*curr).priority {
            (*curr).priority = new_effective;
            thread_yield();
        }
    }
}

/* Returns the current thread's (effective) priority. */
pub unsafe fn thread_get_priority() -> i32 {
    unsafe { (*thread_current()).priority }
}

#[inline]
unsafe fn thread_get_priority_of(t: *const Thread) -> i32 {
    unsafe { (*t).priority }
}

/* --- Priority donation (project 1) -------------------------------- */

/* List-less function comparing two threads by priority for
list_insert_ordered/list_max on the ready list and semaphore
waiter lists.  Returns true if A's thread has strictly higher
priority than B's. */
pub unsafe fn thread_cmp_priority(a: *const ListElem, b: *const ListElem, _aux: *mut ()) -> bool {
    unsafe {
        let ta = list_entry::<Thread>(a as *mut ListElem, Thread::ELEM_OFFSET);
        let tb = list_entry::<Thread>(b as *mut ListElem, Thread::ELEM_OFFSET);
        (*ta).priority > (*tb).priority
    }
}

/* Refreshes the priorities along the chain of locks the current
thread is blocked on: each lock holder's effective priority is
recomputed from the waiters of every lock it holds (nested
donation).  Callers: lock_acquire/sema_down when joining a
waiters list. */
pub unsafe fn thread_donate_priority() {
    unsafe {
        let mut t = thread_current();
        let mut depth = 0;
        loop {
            let wl = (*t).waiting_lock;
            if wl.is_null() {
                break;
            }
            let holder = (*wl).holder;
            if holder.is_null() || holder == t {
                break;
            }
            refresh_priority(holder);
            t = holder;

            /* Depth cap against cycles, as in the reference
            solutions (donation depth above 8 is a bug). */
            depth += 1;
            if depth >= 8 {
                break;
            }
        }
    }
}

/* Recomputes T's effective priority: the max of its original
priority and the highest-priority waiter among all locks T
holds. */
pub(crate) unsafe fn refresh_priority(t: *mut Thread) {
    unsafe {
        let donated = max_donated_priority(t);
        (*t).priority = if (*t).original_priority > donated {
            (*t).original_priority
        } else {
            donated
        };
    }
}

/* Walks the lock graph rooted at T and returns the highest
donated priority (PRI_MIN if there are no donations). */
unsafe fn max_donated_priority(t: *mut Thread) -> i32 {
    unsafe {
        let mut max = PRI_MIN;
        let mut e = list_first(&(*t).acquired_locks);
        while !list_last(&(*t).acquired_locks, e) {
            let lock = list_entry::<Lock>(e, LOCK_ELEM_OFFSET);
            /* The lock's semaphore waiters are the candidate
            donors (kept priority-ordered by synch.rs). */
            let sema = &raw mut (*lock).semaphore;
            let mut w = list_first(&(*sema).waiters);
            while !list_last(&(*sema).waiters, w) {
                let waiter = list_entry::<Thread>(w, Thread::ELEM_OFFSET);
                if (*waiter).priority > max {
                    max = (*waiter).priority;
                }
                w = list_next(w);
            }
            e = list_next(e);
        }
        max
    }
}

/* Offset of `elem` inside Lock's embedded waiter representation.
synch.rs keeps Lock waiters in lock.semaphore.waiters as Thread
elems directly, so locks on the acquired list need their own
link: we reuse lock.semaphore.waiters' head element is NOT safe,
so synch.rs manages Lock::elem.  It is declared here to avoid a
circular dependency: the field lives in synch.rs. */
pub use crate::threads::synch::LOCK_ELEM_OFFSET;

/* --- mlfqs (advanced scheduler, project 1) ------------------------ */

const FSHIFT: u32 = 14; /* 17.14 fixed-point. */
const F: i64 = 1 << FSHIFT;

#[inline]
fn fp_add(a: i64, b: i64) -> i64 {
    a + b
}

#[inline]
fn fp_add_int(a: i64, n: i64) -> i64 {
    a + n * F
}

#[allow(dead_code)]
#[inline]
fn fp_sub_int(a: i64, n: i64) -> i64 {
    a - n * F
}

#[inline]
fn fp_mul_int(x: i64, n: i64) -> i64 {
    x * n
}

#[allow(dead_code)]
#[inline]
fn fp_div_int(x: i64, n: i64) -> i64 {
    x / n
}

#[inline]
fn fp_mul(x: i64, y: i64) -> i64 {
    ((x as i128) * (y as i128) >> FSHIFT) as i64
}

#[inline]
fn fp_div(x: i64, y: i64) -> i64 {
    (((x as i128) << FSHIFT) / (y as i128)) as i64
}

#[inline]
fn fp_round_to_zero(x: i64) -> i64 {
    x / F
}

fn timer_freq() -> i64 {
    crate::devices::timer::TIMER_FREQ as i64
}

/* load_avg = (59/60)*load_avg + (1/60)*ready_threads. */
unsafe fn mlfqs_update_load_avg() {
    unsafe {
        let ready = ready_threads_cnt() as i64;
        let coef = fp_div(fp_mul_int(59 * F, 1), 60 * F);
        let one_sixtieth = fp_div(F, 60 * F);
        LOAD_AVG = fp_add(fp_mul(LOAD_AVG, coef), fp_mul_int(one_sixtieth, ready));
    }
}

/* recent_cpu = (2*load)/(2*load+1) * recent_cpu + 1, all threads. */
unsafe fn mlfqs_foreach_update_recent_cpu() {
    unsafe {
        thread_foreach(mlfqs_update_recent_cpu, core::ptr::null_mut());
    }
}

unsafe fn mlfqs_update_recent_cpu(t: *mut Thread, _aux: *mut ()) {
    unsafe {
        if t == IDLE_THREAD {
            return;
        }
        let twice_load = fp_mul_int(LOAD_AVG, 2);
        let coef = fp_div(twice_load, fp_add_int(twice_load, 1));
        (*t).recent_cpu = fp_add_int(fp_mul(coef, (*t).recent_cpu), 1);
    }
}

/* priority = PRI_MAX - (recent_cpu / 4) - (nice * 2), clamped. */
unsafe fn mlfqs_update_priority(t: *mut Thread) {
    unsafe {
        if t == IDLE_THREAD {
            return;
        }
        let priority = PRI_MAX as i64
            - fp_round_to_zero(fp_div_int((*t).recent_cpu, 4))
            - ((*t).nice as i64) * 2;
        (*t).priority = priority.clamp(PRI_MIN as i64, PRI_MAX as i64) as i32;
    }
}

/* Counts threads in the ready list plus the running thread
(mlfqs load average definition; the idle thread does not
count). */
unsafe fn ready_threads_cnt() -> usize {
    unsafe {
        let mut cnt = list_size(&raw const READY_LIST);
        let cur = thread_current();
        if cur != IDLE_THREAD && (*cur).status == ThreadStatus::Running {
            cnt += 1;
        }
        cnt
    }
}

/* Sets the current thread's nice value to NICE. */
pub unsafe fn thread_set_nice(nice: i32) {
    unsafe {
        let cur = thread_current();
        (*cur).nice = nice;
        mlfqs_update_priority(cur);
        thread_yield();
    }
}

/* Returns the current thread's nice value. */
pub unsafe fn thread_get_nice() -> i32 {
    unsafe { (*thread_current()).nice }
}

/* Returns 100 times the system load average. */
pub unsafe fn thread_get_load_avg() -> i32 {
    unsafe { fp_div_int(fp_mul_int(LOAD_AVG, 100), F) as i32 }
}

/* Returns 100 times the current thread's recent_cpu value. */
pub unsafe fn thread_get_recent_cpu() -> i32 {
    unsafe { fp_div_int(fp_mul_int((*thread_current()).recent_cpu, 100), F) as i32 }
}

/* Idle thread.  Executes when no other thread is ready to run. */
unsafe extern "C" fn idle(aux: *mut ()) {
    unsafe {
        let idle_started = aux as *mut Semaphore;

        IDLE_THREAD = thread_current();
        crate::threads::synch::sema_up(idle_started);

        loop {
            /* Let someone else run. */
            intr_disable();
            thread_block();

            /* Re-enable interrupts and wait for the next one.

            The `sti' instruction disables interrupts until the
            completion of the next instruction, so these two
            instructions are executed atomically.  This atomicity
            is important; otherwise, an interrupt could be handled
            between re-enabling interrupts and waiting for the
            next one to occur, wasting as much as one clock tick
            worth of time. */
            core::arch::asm!("sti; hlt", options(preserves_flags, nomem, nostack));
        }
    }
}

/* Function used as the basis for a kernel thread. */
unsafe extern "C" fn kernel_thread(function: *mut (), aux: *mut ()) {
    unsafe {
        let f: unsafe extern "C" fn(*mut ()) =
            core::mem::transmute::<*mut (), unsafe extern "C" fn(*mut ())>(function);
        intr_enable(); /* The scheduler runs with interrupts off. */
        f(aux); /* Execute the thread function. */
        thread_exit(); /* If function() returns, kill the thread. */
    }
}

/* Does basic initialization of T as a blocked thread named
NAME. */
unsafe fn init_thread(t: *mut Thread, name: *const u8, priority: i32) {
    unsafe {
        crate::ASSERT!(PRI_MIN <= priority && priority <= PRI_MAX);
        crate::ASSERT!(!name.is_null());

        memset(t as *mut u8, 0, core::mem::size_of::<Thread>());
        (*t).status = ThreadStatus::Blocked;
        strlcpy((*t).name.as_mut_ptr(), name, (*t).name.len());
        (*t).tf.rsp = t as u64 + PGSIZE - core::mem::size_of::<*mut u8>() as u64;
        (*t).priority = priority;
        (*t).original_priority = priority;
        (*t).nice = 0;
        (*t).recent_cpu = 0;
        (*t).waiting_lock = core::ptr::null_mut();
        list_init(&raw mut (*t).acquired_locks);
        list_init(&raw mut (*t).children);
        (*t).next_fd = 2;
        (*t).magic = THREAD_MAGIC;
    }
}

/* Chooses and returns the next thread to be scheduled.  Should
return a thread from the run queue, unless the run queue is
empty.  If the run queue is empty, return idle_thread. */
unsafe fn next_thread_to_run() -> *mut Thread {
    unsafe {
        if list_empty(&raw const READY_LIST) {
            IDLE_THREAD
        } else {
            list_entry::<Thread>(list_pop_front(&raw mut READY_LIST), Thread::ELEM_OFFSET)
        }
    }
}

/* Switching the thread by activating the new thread's page
tables, and, if the previous thread is dying, destroying it. */
unsafe fn do_schedule(status: ThreadStatus) {
    unsafe {
        crate::ASSERT!(intr_get_level() == IntrLevel::Off);
        crate::ASSERT!((*thread_current()).status == ThreadStatus::Running);
        while !list_empty(&raw const DESTRUCTION_REQ) {
            let victim = list_entry::<Thread>(
                list_pop_front(&raw mut DESTRUCTION_REQ),
                Thread::ELEM_OFFSET,
            );
            palloc_free_page(victim as *mut u8);
        }
        (*thread_current()).status = status;
        schedule();
    }
}

unsafe fn schedule() {
    unsafe {
        let curr = running_thread();
        let next = next_thread_to_run();

        crate::ASSERT!(intr_get_level() == IntrLevel::Off);
        crate::ASSERT!((*curr).status != ThreadStatus::Running);
        crate::ASSERT!(is_thread(next));
        /* Mark us as running. */
        (*next).status = ThreadStatus::Running;

        /* Start new time slice. */
        THREAD_TICKS = 0;

        /* Activate the new address space. */
        crate::userprog::process::process_activate(next);

        if curr != next {
            /* If the thread we switched from is dying, destroy
            its struct thread.  This must happen late so that
            thread_exit() doesn't pull out the rug under itself.
            We just queue the page free request here because the
            page is currently used by the stack.  The real
            destruction logic runs at the beginning of
            schedule(). */
            if !curr.is_null() && (*curr).status == ThreadStatus::Dying && curr != INITIAL_THREAD {
                crate::ASSERT!(curr != next);
                list_push_back(&raw mut DESTRUCTION_REQ, &raw mut (*curr).elem);
            }

            /* Before switching the thread, we first save the
            information of the currently running thread. */
            thread_launch(next);
        }
    }
}

/* Returns a tid to use for a new thread. */
unsafe fn allocate_tid() -> Tid {
    unsafe {
        static mut NEXT_TID: Tid = 1;

        lock_acquire(&raw mut TID_LOCK);
        let tid = NEXT_TID;
        NEXT_TID += 1;
        lock_release(&raw mut TID_LOCK);

        tid
    }
}

/* ---------------------------------------------------------------------
 * Context-switch assembly (ports of do_iret() and thread_launch()).
 * Written as global_asm with AT&T syntax so the register choreography
 * stays byte-identical to the C original: SysV argument registers are
 * rdi (frame) / rdi,rsi (launch).
 * ------------------------------------------------------------------- */

global_asm!(
    ".globl __do_iret_asm",
    "__do_iret_asm:",
    "    movq %rdi, %rsp",
    "    movq 0(%rsp), %r15",
    "    movq 8(%rsp), %r14",
    "    movq 16(%rsp), %r13",
    "    movq 24(%rsp), %r12",
    "    movq 32(%rsp), %r11",
    "    movq 40(%rsp), %r10",
    "    movq 48(%rsp), %r9",
    "    movq 56(%rsp), %r8",
    "    movq 64(%rsp), %rsi",
    "    movq 72(%rsp), %rdi",
    "    movq 80(%rsp), %rbp",
    "    movq 88(%rsp), %rdx",
    "    movq 96(%rsp), %rcx",
    "    movq 104(%rsp), %rbx",
    "    movq 112(%rsp), %rax",
    "    addq $120, %rsp",
    "    movw 8(%rsp), %ds",
    "    movw 0(%rsp), %es",
    "    addq $32, %rsp",
    "    iretq",
    options(att_syntax)
);

unsafe extern "C" {
    fn __do_iret_asm(tf: *mut IntrFrame);
}

/* Use iretq to launch the thread. */
#[inline(never)]
pub unsafe fn do_iret(tf: *mut IntrFrame) {
    unsafe { __do_iret_asm(tf) }
}

global_asm!(
    ".globl __thread_launch_asm",
    "__thread_launch_asm:",
    "    push %rax",
    "    push %rbx",
    "    push %rcx",
    "    movq %rdi, %rax", /* tf_cur */
    "    movq %rsi, %rcx", /* tf (next thread) */
    "    movq %r15, 0(%rax)",
    "    movq %r14, 8(%rax)",
    "    movq %r13, 16(%rax)",
    "    movq %r12, 24(%rax)",
    "    movq %r11, 32(%rax)",
    "    movq %r10, 40(%rax)",
    "    movq %r9, 48(%rax)",
    "    movq %r8, 56(%rax)",
    "    movq %rsi, 64(%rax)",
    "    movq %rdi, 72(%rax)",
    "    movq %rbp, 80(%rax)",
    "    movq %rdx, 88(%rax)",
    "    pop %rbx", /* saved rcx */
    "    movq %rbx, 96(%rax)",
    "    pop %rbx", /* saved rbx */
    "    movq %rbx, 104(%rax)",
    "    pop %rbx", /* saved rax */
    "    movq %rbx, 112(%rax)",
    "    addq $120, %rax",
    "    movw %es, (%rax)",
    "    movw %ds, 8(%rax)",
    "    addq $32, %rax",
    "    call .L__next", /* read the current rip */
    ".L__next:",
    "    pop %rbx",
    "    addq $(.Lout_iret - .L__next), %rbx",
    "    movq %rbx, 0(%rax)", /* rip */
    "    movw %cs, 8(%rax)",  /* cs */
    "    pushfq",
    "    popq %rbx",
    "    movq %rbx, 16(%rax)", /* eflags */
    "    movq %rsp, 24(%rax)", /* rsp */
    "    movw %ss, 32(%rax)",
    "    movq %rcx, %rdi",
    "    call __do_iret_asm",
    /* When this thread is resumed later, its saved rip points here:
    return to the (preserved) caller stack, exactly like the C
    inline-asm version flowing into its function epilogue. */
    ".Lout_iret:",
    "    ret",
    options(att_syntax)
);

unsafe extern "C" {
    fn __thread_launch_asm(tf_cur: *mut IntrFrame, tf: *mut IntrFrame);
}

/* The main switching logic: save the current register context
into the current thread's intr_frame, then iretq into TH. */
#[inline(never)]
unsafe fn thread_launch(th: *mut Thread) {
    unsafe {
        let tf_cur: *mut IntrFrame = &raw mut (*running_thread()).tf;
        let tf: *mut IntrFrame = &raw mut (*th).tf;
        crate::ASSERT!(intr_get_level() == IntrLevel::Off);
        __thread_launch_asm(tf_cur, tf);
    }
}

/* Invoke the callback ACTION for every thread. */
pub unsafe fn thread_foreach(action: unsafe fn(*mut Thread, *mut ()), aux: *mut ()) {
    unsafe {
        action(running_thread(), aux);

        let mut e = list_first(&raw const READY_LIST);
        while !list_last(&raw const READY_LIST, e) {
            let t = list_entry::<Thread>(e, Thread::ELEM_OFFSET);
            action(t, aux);
            e = list_next(e);
        }
    }
}

/* Small helpers over the landed list API (names used above). */
use crate::lib::kernel::list::{list_begin as list_first, list_end, list_next, list_size};
#[inline]
unsafe fn list_last(l: *const List, e: *mut ListElem) -> bool {
    unsafe { e == list_end(l) }
}
