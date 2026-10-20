//! Port of `include/threads/io.h` and `include/intrinsic.h`.
//!
//! Primitive access to x86-64 I/O ports, control registers, descriptor
//! tables and MSRs. All functions are `#[inline]`; they are the unsafe
//! building blocks every device and paging routine is written against.

#![allow(dead_code)]

use core::arch::asm;

/// Physical-address width type used by CR2/CR3 readers.
pub type PAddr = u64;

/* ------------------------------ I/O ports ------------------------------ */

/* Reads and returns a byte from PORT. */
#[inline]
pub unsafe fn inb(port: u16) -> u8 {
    let data: u8;
    unsafe { asm!("in al, dx", lateout("al") data, in("dx") port, options(nomem, nostack)) };
    data
}

/* Reads CNT bytes from PORT, one after another, and stores them
into the buffer starting at ADDR. */
#[inline]
pub unsafe fn insb(port: u16, addr: *mut u8, cnt: usize) {
    unsafe {
        asm!(
            "cld; rep insb",
            inlateout("rdi") addr => _,
            inlateout("rcx") cnt => _,
            in("dx") port,
            options(nostack)
        );
    }
}

/* Reads and returns 16 bits from PORT. */
#[inline]
pub unsafe fn inw(port: u16) -> u16 {
    let data: u16;
    unsafe { asm!("in ax, dx", lateout("ax") data, in("dx") port, options(nomem, nostack)) };
    data
}

/* Reads CNT 16-bit units from PORT into ADDR. */
#[inline]
pub unsafe fn insw(port: u16, addr: *mut u16, cnt: usize) {
    unsafe {
        asm!(
            "cld; rep insw",
            inlateout("rdi") addr => _,
            inlateout("rcx") cnt => _,
            in("dx") port,
            options(nostack)
        );
    }
}

/* Reads and returns 32 bits from PORT. */
#[inline]
pub unsafe fn inl(port: u16) -> u32 {
    let data: u32;
    unsafe { asm!("in eax, dx", lateout("eax") data, in("dx") port, options(nomem, nostack)) };
    data
}

/* Writes byte DATA to PORT. */
#[inline]
pub unsafe fn outb(port: u16, data: u8) {
    unsafe { asm!("out dx, al", in("al") data, in("dx") port, options(nomem, nostack)) };
}

/* Writes to PORT each byte of data in the CNT-byte buffer at ADDR. */
#[inline]
pub unsafe fn outsb(port: u16, addr: *const u8, cnt: usize) {
    unsafe {
        asm!(
            "cld; rep outsb",
            inlateout("rsi") addr => _,
            inlateout("rcx") cnt => _,
            in("dx") port,
            options(nostack)
        );
    }
}

/* Writes the 16-bit DATA to PORT. */
#[inline]
pub unsafe fn outw(port: u16, data: u16) {
    unsafe { asm!("out dx, ax", in("ax") data, in("dx") port, options(nomem, nostack)) };
}

/* Writes to PORT each 16-bit unit of data in the CNT-unit buffer at ADDR. */
#[inline]
pub unsafe fn outsw(port: u16, addr: *const u16, cnt: usize) {
    unsafe {
        asm!(
            "cld; rep outsw",
            inlateout("rsi") addr => _,
            inlateout("rcx") cnt => _,
            in("dx") port,
            options(nostack)
        );
    }
}

/* Writes the 32-bit DATA to PORT. */
#[inline]
pub unsafe fn outl(port: u16, data: u32) {
    unsafe { asm!("out dx, eax", in("eax") data, in("dx") port, options(nomem, nostack)) };
}

/* ------------------------- Interrupt control --------------------------- */

/// Disable interrupts; returns the previous RFLAGS.IF state is available
/// via `read_eflags` before the call.
#[inline]
pub unsafe fn cli() {
    unsafe { asm!("cli", options(nomem, nostack, preserves_flags)) };
}

/// Enable interrupts.
#[inline]
pub unsafe fn sti() {
    unsafe { asm!("sti", options(nomem, nostack, preserves_flags)) };
}

/// Halt until the next interrupt.
#[inline]
pub unsafe fn hlt() {
    unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
}

/* ------------------------- Control registers -------------------------- */

/// Store a page-table base into CR3, activating it immediately.
#[inline]
pub unsafe fn lcr3(val: u64) {
    unsafe { asm!("mov cr3, {0}", in(reg) val, options(nomem, nostack, preserves_flags)) };
}

/// Read CR3 (current page-table base).
#[inline]
pub unsafe fn rcr3() -> u64 {
    let val: u64;
    unsafe { asm!("mov {0}, cr3", out(reg) val, options(nomem, nostack, preserves_flags)) };
    val
}

/// Read CR2 (page-fault address).
#[inline]
pub unsafe fn rcr2() -> u64 {
    let val: u64;
    unsafe { asm!("mov {0}, cr2", out(reg) val, options(nomem, nostack, preserves_flags)) };
    val
}

/// Invalidate the TLB entry for ADDR's page.
#[inline]
pub unsafe fn invlpg(addr: u64) {
    unsafe { asm!("invlpg [{0}]", in(reg) addr, options(nostack, preserves_flags)) };
}

/* --------------------------- System registers -------------------------- */

/// GDTR/IDTR pointer (`struct desc_ptr`): 10 bytes, packed —
/// the base sits at offset 2, exactly as LGDT/LIDT read it.
#[repr(C, packed)]
pub struct DescPtr {
    pub size: u16,
    pub address: u64,
}

/// Load the GDT register.
#[inline]
pub unsafe fn lgdt(dtr: *const DescPtr) {
    unsafe { asm!("lgdt [{0}]", in(reg) dtr, options(nostack)) };
}

/// Load the IDT register.
#[inline]
pub unsafe fn lidt(dtr: *const DescPtr) {
    unsafe { asm!("lidt [{0}]", in(reg) dtr, options(nostack)) };
}

/// Load the LDT register.
#[inline]
pub unsafe fn lldt(sel: u16) {
    unsafe { asm!("lldt {0:x}", in(reg) sel, options(nomem, nostack)) };
}

/// Load the task register.
#[inline]
pub unsafe fn ltr(sel: u16) {
    unsafe { asm!("ltr {0:x}", in(reg) sel, options(nomem, nostack)) };
}

/// Read RFLAGS.
#[inline]
pub unsafe fn read_eflags() -> u64 {
    let rflags: u64;
    unsafe { asm!("pushfq; pop {0}", out(reg) rflags, options(preserves_flags)) };
    rflags
}

/// Write a model-specific register.
#[inline]
pub unsafe fn write_msr(ecx: u32, val: u64) {
    let eax = val as u32;
    let edx = (val >> 32) as u32;
    unsafe {
        asm!(
            "wrmsr",
            in("ecx") ecx,
            in("edx") edx,
            in("eax") eax,
            options(nomem, nostack, preserves_flags)
        )
    };
}

/* --------------------------- Raw register reads ------------------------ */

/* The following are used while a fresh thread/kernel stack is being
set up, so they read registers directly. */

#[inline]
pub unsafe fn rrax() -> u64 {
    let val: u64;
    unsafe { asm!("mov {0}, rax", out(reg) val, options(nomem, nostack, preserves_flags)) };
    val
}

#[inline]
pub unsafe fn rrdi() -> u64 {
    let val: u64;
    unsafe { asm!("mov {0}, rdi", out(reg) val, options(nomem, nostack, preserves_flags)) };
    val
}

#[inline]
pub unsafe fn rrsi() -> u64 {
    let val: u64;
    unsafe { asm!("mov {0}, rsi", out(reg) val, options(nomem, nostack, preserves_flags)) };
    val
}

#[inline]
pub unsafe fn rrdx() -> u64 {
    let val: u64;
    unsafe { asm!("mov {0}, rdx", out(reg) val, options(nomem, nostack, preserves_flags)) };
    val
}

#[inline]
pub unsafe fn rrcx() -> u64 {
    let val: u64;
    unsafe { asm!("mov {0}, rcx", out(reg) val, options(nomem, nostack, preserves_flags)) };
    val
}

#[inline]
pub unsafe fn rr8() -> u64 {
    let val: u64;
    unsafe { asm!("mov {0}, r8", out(reg) val, options(nomem, nostack, preserves_flags)) };
    val
}

#[inline]
pub unsafe fn rr9() -> u64 {
    let val: u64;
    unsafe { asm!("mov {0}, r9", out(reg) val, options(nomem, nostack, preserves_flags)) };
    val
}

#[inline]
pub unsafe fn rr10() -> u64 {
    let val: u64;
    unsafe { asm!("mov {0}, r10", out(reg) val, options(nomem, nostack, preserves_flags)) };
    val
}

#[inline]
pub unsafe fn rrsp() -> u64 {
    let val: u64;
    unsafe { asm!("mov {0}, rsp", out(reg) val, options(nomem, nostack, preserves_flags)) };
    val
}
