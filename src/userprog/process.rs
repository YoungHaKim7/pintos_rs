//! Port of `userprog/process.c` — user-process loading and control.
//!
//! M1 subset: only the pieces the scheduler core needs
//! (process_activate during context switch and a process_exit
//! stub).  The full loader/argument-passing/exec machinery is the
//! userprog milestone.

use crate::threads::thread::Thread;
use crate::threads::{init, mmu, vaddr};

/* Activates the new thread's page tables: if the thread has its
own pml4 (a user process), switch to it; otherwise fall back to
the kernel's base page table.  Also points the TSS's kernel stack
pointer at the new thread's stack page. */
pub unsafe fn process_activate(next: *mut Thread) {
    unsafe {
        /* Load the new page table. */
        if !(*next).pml4.is_null() {
            mmu::pml4_activate((*next).pml4);
        } else {
            mmu::pml4_activate(init::BASE_PML4);
        }

        /* Update the TSS's rsp0 to point at the new thread's
        stack. */
        crate::userprog::tss::tss_update(next as *mut u8);
        let _ = vaddr::PGSIZE;
    }
}

/* Frees the current process's resources and exits.  M1 stub: the
full cleanup (fd table, spt, pml4 teardown, child bookkeeping)
arrives with the userprog milestone. */
pub unsafe fn process_exit() {
    unsafe {}
}
