//! Port of `include/threads/pte.h` — x86-64 hardware page-table entries.

#![allow(dead_code)]

pub const PML4SHIFT: u64 = 39;
pub const PDPESHIFT: u64 = 30;
pub const PDXSHIFT: u64 = 21;
pub const PTXSHIFT: u64 = 12;

#[inline]
pub fn pml4(la: u64) -> u64 {
    (la >> PML4SHIFT) & 0x1FF
}
#[inline]
pub fn pdpe(la: u64) -> u64 {
    (la >> PDPESHIFT) & 0x1FF
}
#[inline]
pub fn pdx(la: u64) -> u64 {
    (la >> PDXSHIFT) & 0x1FF
}
#[inline]
pub fn ptx(la: u64) -> u64 {
    (la >> PTXSHIFT) & 0x1FF
}

/* Flag bits. A PDE/PTE of 0 means "not present". */
pub const PTE_FLAGS: u64 = 0x0000_0000_0000_0fff;
pub const PTE_ADDR_MASK: u64 = 0xffff_ffff_ffff_f000;
pub const PTE_AVL: u64 = 0x0000_0e00;
pub const PTE_P: u64 = 0x1;
pub const PTE_W: u64 = 0x2;
pub const PTE_U: u64 = 0x4;
pub const PTE_A: u64 = 0x20;
pub const PTE_D: u64 = 0x40;

/// Address part of a PTE.
#[inline]
pub fn pte_addr(pte: u64) -> u64 {
    pte & !0xFFF
}
