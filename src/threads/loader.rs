//! Port of `include/threads/loader.h` — constants fixed by the PC BIOS
//! boot protocol and the pintos MBR loader.

#![allow(dead_code)]

/* Constants fixed by the PC BIOS. */
pub const LOADER_BASE: u64 = 0x7c00; /* Physical address of loader's base. */
pub const LOADER_END: u64 = 0x7e00; /* Physical address of end of loader. */

/* Physical address of kernel base. */
pub const LOADER_KERN_BASE: u64 = 0x8004_0000_0000;

/* Physical address where the loader loads the kernel. */
pub const LOADER_PHYS_BASE: u64 = 0x200000;

/* Multiboot info block written by the loader at boot. */
pub const MULTIBOOT_INFO: u64 = 0x7000;
pub const MULTIBOOT_FLAG: u64 = MULTIBOOT_INFO;
pub const MULTIBOOT_MMAP_LEN: u64 = MULTIBOOT_INFO + 44;
pub const MULTIBOOT_MMAP_ADDR: u64 = MULTIBOOT_INFO + 48;

pub const E820_MAP: u64 = MULTIBOOT_INFO + 52;
pub const E820_MAP4: u64 = MULTIBOOT_INFO + 56;

/* Important loader physical addresses. */
pub const LOADER_SIG_LEN: u64 = 2;
pub const LOADER_ARGS_LEN: u64 = 128;
pub const LOADER_ARG_CNT_LEN: u64 = 4;
pub const LOADER_SIG: u64 = LOADER_END - LOADER_SIG_LEN; /* 0xaa55 BIOS signature. */
pub const LOADER_ARGS: u64 = LOADER_SIG - LOADER_ARGS_LEN;
pub const LOADER_ARG_CNT: u64 = LOADER_ARGS - LOADER_ARG_CNT_LEN;

/* GDT selectors defined by the loader.
More selectors are defined by userprog/gdt.rs. */
pub const SEL_NULL: u16 = 0x00; /* Null selector. */
pub const SEL_KCSEG: u16 = 0x08; /* Kernel code selector. */
pub const SEL_KDSEG: u16 = 0x10; /* Kernel data selector. */
pub const SEL_UDSEG: u16 = 0x1b; /* User data selector. */
pub const SEL_UCSEG: u16 = 0x23; /* User code selector. */
pub const SEL_TSS: u16 = 0x28; /* Task-state segment. */
pub const SEL_CNT: u64 = 8; /* Number of segments. */

/* Boot info accessors: the loader passes data through fixed
physical addresses (mapped at KERN_BASE), not registers. */

/// Kernel virtual address of the command-line argument count.
#[inline]
pub fn ptov_arg_cnt() -> u64 {
    crate::threads::vaddr::ptov(LOADER_ARG_CNT)
}

/// Kernel virtual address of the command-line argument strings.
#[inline]
pub fn ptov_args() -> u64 {
    crate::threads::vaddr::ptov(LOADER_ARGS)
}
