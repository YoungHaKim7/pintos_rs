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

impl SupplementalPageTable {
    pub const fn empty() -> Self {
        // Hash contains raw pointers; written before threads run.
        Self { table: Hash::EMPTY }
    }
}
