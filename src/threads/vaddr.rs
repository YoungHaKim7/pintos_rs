//! Port of `include/threads/vaddr.h` — virtual-address helpers.

use crate::threads::loader::LOADER_KERN_BASE;

/* Page offset (bits 0:12). */
pub const PGSHIFT: u64 = 0; /* Index of first offset bit. */
pub const PGBITS: u64 = 12; /* Number of offset bits. */
pub const PGSIZE: u64 = 1 << PGBITS; /* Bytes in a page. */
pub const PGMASK: u64 = ((1u64 << PGBITS) - 1) << PGSHIFT; /* Page offset bits. */

/// Offset within a page.
#[inline]
pub fn pg_ofs(va: u64) -> u64 {
    va & PGMASK
}

/// Page number.
#[inline]
pub fn pg_no(va: u64) -> u64 {
    va >> PGBITS
}

/// Round up to nearest page boundary.
#[inline]
pub fn pg_round_up(va: u64) -> u64 {
    (va + PGSIZE - 1) & !PGMASK
}

/// Round down to nearest page boundary.
#[inline]
pub fn pg_round_down(va: u64) -> u64 {
    va & !PGMASK
}

/// Kernel virtual address start.
pub const KERN_BASE: u64 = LOADER_KERN_BASE;

/// User stack top.
pub const USER_STACK: u64 = 0x47480000;

/// Returns true if VADDR is a user virtual address.
#[inline]
pub fn is_user_vaddr(vaddr: u64) -> bool {
    !is_kernel_vaddr(vaddr)
}

/// Returns true if VADDR is a kernel virtual address.
#[inline]
pub fn is_kernel_vaddr(vaddr: u64) -> bool {
    vaddr >= KERN_BASE
}

/// Returns kernel virtual address at which physical address PADDR is mapped.
#[inline]
pub fn ptov(paddr: u64) -> u64 {
    paddr + KERN_BASE
}

/// Returns physical address at which kernel virtual address VADDR is mapped.
#[inline]
pub fn vtop(vaddr: u64) -> u64 {
    debug_assert!(is_kernel_vaddr(vaddr));
    vaddr - KERN_BASE
}
