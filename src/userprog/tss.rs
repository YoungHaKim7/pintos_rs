//! Port of `userprog/tss.c` — the Task-State Segment.
//!
//! The tss is only used for one thing: stack switching for
//! interrupts that occur in user mode (rsp0).  When the scheduler
//! switches threads it also points rsp0 at the new thread's kernel
//! stack (see process_activate).

use crate::threads::palloc::{PAL_ASSERT, PAL_ZERO, palloc_get_page};
use crate::threads::vaddr::PGSIZE;

/* The Task-State Segment (x86-64 layout, packed). */
#[repr(C, packed)]
pub struct TaskState {
    pub res1: u32,
    pub rsp0: u64, /* Ring 0 stack pointer: the only used field. */
    pub rsp1: u64,
    pub rsp2: u64,
    pub res2: u64,
    pub ist1: u64,
    pub ist2: u64,
    pub ist3: u64,
    pub ist4: u64,
    pub ist5: u64,
    pub ist6: u64,
    pub ist7: u64,
    pub res3: u64,
    pub res4: u16,
    pub iomb: u16,
}

/* Kernel tss (C symbol name: asm/syscall-entry.S loads rsp0 from
`tss+4` directly). */
#[unsafe(no_mangle)]
pub static mut tss: *mut TaskState = core::ptr::null_mut();

/* Initializes the kernel tss. */
pub unsafe fn tss_init() {
    unsafe {
        /* Our tss is never used in a call gate or task gate, so
        only a few fields of it are ever referenced, and those are
        the only ones we initialize. */
        tss = palloc_get_page(PAL_ASSERT | PAL_ZERO) as *mut TaskState;
        tss_update(crate::threads::thread::thread_current() as *mut u8);
    }
}

/* Returns the kernel tss. */
pub unsafe fn tss_get() -> *mut TaskState {
    unsafe {
        crate::ASSERT!(!tss.is_null());
        tss
    }
}

/* Sets the ring 0 stack pointer in the tss to point to the end of
the thread stack (NEXT is the thread pointer, i.e. its page
base). */
pub unsafe fn tss_update(next: *mut u8) {
    unsafe {
        crate::ASSERT!(!tss.is_null());
        (*tss).rsp0 = next as u64 + PGSIZE;
    }
}
