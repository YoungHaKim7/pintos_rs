//! Port of `userprog/syscall.c` — system-call entry setup.
//!
//! x86-64 fast path: the `syscall` instruction routes through MSRs;
//! asm/syscall-entry.S builds the interrupt frame and calls
//! `syscall_handler`.

use crate::intrinsic::write_msr;
use crate::printf;
use crate::threads::interrupt::IntrFrame;
use crate::threads::loader::{SEL_KCSEG, SEL_UCSEG};
use crate::threads::thread::thread_exit;

/* Model-specific registers for the syscall fast path. */
pub const MSR_STAR: u32 = 0xc0000081; /* Segment selector msr */
pub const MSR_LSTAR: u32 = 0xc0000082; /* Long mode SYSCALL target */
pub const MSR_SYSCALL_MASK: u32 = 0xc0000084; /* Mask for the eflags */

/* EFLAGS bits (threads/flags.h). */
pub const FLAG_MBS: u64 = 0x2; /* 1=maskable interrupts enabled. */
pub const FLAG_IF: u64 = 1 << 9; /* 1=interrupts enabled. */
pub const FLAG_TF: u64 = 1 << 8; /* 1=trap flag. */
pub const FLAG_DF: u64 = 1 << 10; /* 1=direction flag. */
pub const FLAG_IOPL: u64 = 3 << 12; /* IOPL=3 (user I/O). */
pub const FLAG_AC: u64 = 1 << 18; /* 1=alignment check. */
pub const FLAG_NT: u64 = 1 << 14; /* 1=nested task. */

unsafe extern "C" {
    #[link_name = "syscall_entry"]
    static SYSCALL_ENTRY: extern "C" fn();
}

/* System call setup: program the syscall MSRs. */
pub unsafe fn syscall_init() {
    unsafe {
        write_msr(
            MSR_STAR,
            ((SEL_UCSEG as u64) - 0x10) << 48 | ((SEL_KCSEG as u64) << 32),
        );
        write_msr(MSR_LSTAR, SYSCALL_ENTRY as usize as u64);

        /* The interrupt service routine should not serve any
        interrupts until the syscall_entry swaps the userland stack
        to the kernel-mode stack.  Therefore, we mask these flags. */
        write_msr(
            MSR_SYSCALL_MASK,
            FLAG_IF | FLAG_TF | FLAG_DF | FLAG_IOPL | FLAG_AC | FLAG_NT,
        );
    }
}

/* The main system call interface.  (Skeleton: the full dispatch
table is the project-2 milestone.) */
#[unsafe(no_mangle)]
pub extern "C" fn syscall_handler(f: *mut IntrFrame) {
    // TODO(userprog): dispatch on (*f).R.rax (SYS_*).
    let _ = f;
    printf!("system call!\n");
    unsafe { thread_exit() };
}
