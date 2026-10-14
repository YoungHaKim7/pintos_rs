//! Port of `vm/` — virtual memory (project 3).
//!
//! M1 subset: only the supplemental-page-table shell embedded in
//! `struct Thread`.  The full page/fault machinery (lazy loading,
//! frames, eviction, swap, COW) is the project-3 milestone.

use crate::lib::kernel::hash::Hash;

/// Per-thread table of virtual-memory regions.  The C skeleton
/// leaves the layout to the student; this port uses a hash table
/// keyed by the page's virtual address (filled in with the vm
/// milestone).
#[repr(C)]
pub struct SupplementalPageTable {
    /// Hash of `vm::Page` keyed by `va`.
    pub table: Hash,
}

/* The SPT is embedded in `struct Thread`, which is zeroed at
init; `hash_init` is called when a thread gains a user address
space (userprog/process.rs). */

/// Fault-handling hook called from `userprog::exception::page_fault`.
/// M1 stub: nothing is mapped lazily yet, so every fault falls
/// through to the classic kill path.  The project-3 milestone
/// implements spt lookup, claiming, COW and stack growth here.
pub unsafe fn vm_try_handle_fault(
    f: *mut crate::threads::interrupt::IntrFrame,
    addr: u64,
    user: bool,
    write: bool,
    not_present: bool,
) -> bool {
    let _ = (f, addr, user, write, not_present);
    false
}

/// Initializes the VM subsystem.  M1 stub.
pub unsafe fn vm_init() {}
