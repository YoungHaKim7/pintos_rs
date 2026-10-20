//! Port of `userprog/gdt.c` — the Global Descriptor Table.
//!
//! The table is 8 entries of 8 bytes each (C's `struct
//! segment_desc`); the 16-byte TSS system descriptor spans entries
//! 5 and 6.  Descriptors are built with integer bit math instead
//! of C bitfields (same layout).

use core::arch::asm;

use crate::intrinsic::{lgdt, lldt, DescPtr};
use crate::threads::loader::{SEL_KCSEG, SEL_KDSEG, SEL_TSS, SEL_UDSEG, SEL_UCSEG, SEL_CNT};
use crate::userprog::tss::{tss_get, TaskState};

/* C's SEG64(type, base, lim, dpl): s=1, p=1, l=1, db=0, g=1,
lim from LIM>>12 / LIM>>28, base split across three fields. */
const fn seg64(type_: u32, base: u64, lim: u64, dpl: u32) -> u64 {
    ((lim >> 12) & 0xffff) as u64
        | ((base & 0xffff) << 16)
        | (((base >> 16) & 0xff) << 32)
        | (((type_ as u64) & 0xf) << 40)
        | (1u64 << 44) /* s */
        | (((dpl as u64) & 0x3) << 45)
        | (1u64 << 47) /* p */
        | (((lim >> 28) & 0xf) << 48)
        | (1u64 << 53) /* l */
        | (1u64 << 55) /* g */
        | (((base >> 24) & 0xff) << 56)
}

static mut GDT: [u64; SEL_CNT as usize] = [
    0, /* SEL_NULL */
    seg64(0xa, 0x0, 0xffffffff, 0), /* SEL_KCSEG: kernel 64-bit code */
    seg64(0x2, 0x0, 0xffffffff, 0), /* SEL_KDSEG: kernel data */
    seg64(0x2, 0x0, 0xffffffff, 3), /* SEL_UDSEG: user data */
    seg64(0xa, 0x0, 0xffffffff, 3), /* SEL_UCSEG: user 64-bit code */
    0, /* SEL_TSS low 8 bytes (filled by gdt_init) */
    0, /* SEL_TSS high 8 bytes */
    0,
];

static mut GDT_DS: DescPtr = DescPtr {
    size: (core::mem::size_of::<[u64; 8]>() - 1) as u16,
    address: 0, /* patched at init (GDT address is not const) */
};

/* Sets up a proper GDT.  The bootstrap loader's GDT didn't
include user-mode selectors or a TSS, but we need both now. */
pub unsafe fn gdt_init() {
    unsafe {
        /* Initialize the TSS descriptor (16-byte system descriptor:
        type 0x9, s=0, p=1, no granularity), split across entries 5
        and 6. */
        let tss = tss_get();
        let tss_addr = tss as u64;
        let size = core::mem::size_of::<TaskState>() as u64;

        let low: u64 = (size & 0xffff)
            | ((tss_addr & 0xffff) << 16)
            | (((tss_addr >> 16) & 0xff) << 32)
            | (0x9u64 << 40) /* type */
            | (0u64 << 44) /* s = 0 */
            | (0u64 << 45) /* dpl */
            | (1u64 << 47) /* p */
            | (((size >> 16) & 0xf) << 48);
        let high: u64 = (tss_addr >> 32) & 0xffffffff;

        GDT[(SEL_TSS >> 3) as usize] = low;
        GDT[((SEL_TSS >> 3) as usize) + 1] = high;

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
