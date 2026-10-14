//! Port of `userprog/gdt.c` — the Global Descriptor Table.
//!
//! Descriptors are built with integer bit math instead of C
//! bitfields (same layout, verified against the C struct).

use core::arch::asm;

use crate::intrinsic::{DescPtr, lgdt, lldt};
use crate::threads::loader::{
    SEL_CNT, SEL_KCSEG, SEL_KDSEG, SEL_NULL, SEL_TSS, SEL_UCSEG, SEL_UDSEG,
};
use crate::userprog::tss::{TaskState, tss_get};

/// One GDT entry: 8 bytes for code/data segments, 16 bytes for the
/// TSS descriptor (second word holds the upper base bits).
#[derive(Clone, Copy)]
#[repr(C)]
pub struct SegmentDesc {
    pub w0: u64,
    pub w1: u64,
}

impl SegmentDesc {
    const ZERO: SegmentDesc = SegmentDesc { w0: 0, w1: 0 };
}

/* C's SEG64(type, base, lim, dpl): s=1, p=1, l=1, db=0, g=1,
lim fields taken from LIM>>12 / LIM>>28, base split across three
fields. */
const fn seg64(type_: u32, base: u64, lim: u64, dpl: u32) -> SegmentDesc {
    let w0 = ((lim >> 12) & 0xffff) as u64
        | ((base & 0xffff) << 16)
        | (((base >> 16) & 0xff) << 32)
        | (((type_ as u64) & 0xf) << 40)
        | (1u64 << 44) /* s */
        | (((dpl as u64) & 0x3) << 45)
        | (1u64 << 47) /* p */
        | (((lim >> 28) & 0xf) << 48)
        | (1u64 << 53) /* l */
        | (1u64 << 55) /* g */
        | (((base >> 24) & 0xff) << 56);
    SegmentDesc { w0, w1: 0 }
}

static mut GDT: [SegmentDesc; SEL_CNT as usize] = [
    SegmentDesc::ZERO,              /* SEL_NULL */
    seg64(0xa, 0x0, 0xffffffff, 0), /* SEL_KCSEG: kernel 64-bit code */
    seg64(0x2, 0x0, 0xffffffff, 0), /* SEL_KDSEG: kernel data */
    seg64(0x2, 0x0, 0xffffffff, 3), /* SEL_UDSEG: user data */
    seg64(0xa, 0x0, 0xffffffff, 3), /* SEL_UCSEG: user 64-bit code */
    SegmentDesc::ZERO,              /* SEL_TSS: filled by gdt_init */
    SegmentDesc::ZERO,
    SegmentDesc::ZERO,
];

static mut GDT_DS: DescPtr = DescPtr {
    size: (core::mem::size_of::<[SegmentDesc; 8]>() - 1) as u16,
    address: 0, /* patched at init (GDT address is not const) */
};

/* Sets up a proper GDT.  The bootstrap loader's GDT didn't
include user-mode selectors or a TSS, but we need both now. */
pub unsafe fn gdt_init() {
    unsafe {
        /* Initialize the TSS descriptor (16-byte system
        descriptor: type 0x9, s=0, p=1, no granularity). */
        let tss = tss_get();
        let tss_addr = tss as u64;
        let size = core::mem::size_of::<TaskState>() as u64;

        let tss_desc = &mut GDT[(SEL_TSS >> 3) as usize];
        *tss_desc = SegmentDesc {
            w0: (size & 0xffff)
                | ((tss_addr & 0xffff) << 16)
                | (((tss_addr >> 16) & 0xff) << 32)
                | (0x9u64 << 40) /* type */
                | (0u64 << 44) /* s = 0 */
                | (0u64 << 45) /* dpl */
                | (1u64 << 47) /* p */
                | (((size >> 16) & 0xf) << 48),
            w1: ((tss_addr >> 32) & 0xffffffff),
        };

        GDT_DS.address = GDT.as_ptr() as u64;
        lgdt(&GDT_DS);

        /* Reload segment registers. */
        asm!("mov gs, ax", in("ax") SEL_UDSEG, options(nomem, nostack, preserves_flags));
        asm!("mov fs, ax", in("ax") 0u16, options(nomem, nostack, preserves_flags));
        asm!("mov es, ax", in("ax") SEL_KDSEG, options(nomem, nostack, preserves_flags));
        asm!("mov ds, ax", in("ax") SEL_KDSEG, options(nomem, nostack, preserves_flags));
        asm!("mov ss, ax", in("ax") SEL_KDSEG, options(nomem, nostack, preserves_flags));

        /* Far return to reload CS: push the new selector, push the
        return address (the label just after retfq), retfq. */
        asm!(
            "xor eax, eax",
            "mov ax, {0:x}",
            "push rax",
            "lea rax, [rip + 2f]",
            "push rax",
            "retfq",
            "2:",
            in(reg) SEL_KCSEG,
            out("rax") _,
        );

        /* Kill the local descriptor table. */
        lldt(0);
    }
}
