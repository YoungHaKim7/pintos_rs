//! Port of `userprog/exception.c` — exception handlers.

use crate::PANIC;
use crate::intrinsic::rcr2;
use crate::printf;
use crate::threads::interrupt::{
    IntrFrame, IntrHandlerFunc, IntrLevel, intr_dump_frame, intr_enable, intr_name,
    intr_register_int,
};
use crate::threads::loader::{SEL_KCSEG, SEL_UCSEG};
use crate::threads::thread::{thread_exit, thread_name};

/* Page-fault error code bits (exception.h PF_*). */
pub const PF_P: u64 = 1 << 0; /* 0: not-present page. 1: rights violation. */
pub const PF_W: u64 = 1 << 1; /* 0: read access. 1: write access. */
pub const PF_U: u64 = 1 << 2; /* 0: kernel access. 1: user access. */

/* Number of page faults processed. */
static mut PAGE_FAULT_CNT: i64 = 0;

extern "C" fn kill(f: *mut IntrFrame) {
    unsafe {
        /* The interrupt frame's code segment value tells us where
        the exception originated. */
        match (*f).cs as u16 {
            /* User's code segment, so it's a user exception: kill
            the user process. */
            SEL_UCSEG => {
                printf!(
                    "%s: dying due to interrupt %#04llx (%s).\n",
                    thread_name(),
                    (*f).vec_no,
                    intr_name((*f).vec_no as u8)
                );
                intr_dump_frame(f);
                thread_exit();
            }

            /* Kernel's code segment, which indicates a kernel bug.
            Kernel code shouldn't throw exceptions. */
            SEL_KCSEG => {
                intr_dump_frame(f);
                PANIC!("Kernel bug - unexpected interrupt in kernel");
            }

            /* Some other code segment?  Shouldn't happen. */
            _ => {
                printf!(
                    "Interrupt %#04llx (%s) in unknown segment %04x.\n",
                    (*f).vec_no,
                    intr_name((*f).vec_no as u8),
                    (*f).cs
                );
                thread_exit();
            }
        }
    }
}

extern "C" fn page_fault(f: *mut IntrFrame) {
    unsafe {
        /* Obtain the faulting address, the virtual address that
        was accessed to cause the fault. */
        let fault_addr = rcr2();

        /* Turn interrupts back on (they were only off so that we
        could be assured of reading CR2 before it changed). */
        intr_enable();

        /* Determine cause. */
        let not_present = (*f).error_code & PF_P == 0;
        let write = (*f).error_code & PF_W != 0;
        let user = (*f).error_code & PF_U != 0;

        /* For project 3 and later: hand the fault to the
        supplemental page table first. */
        if crate::vm::vm_try_handle_fault(f, fault_addr, user, write, not_present) {
            return;
        }

        /* Count page faults. */
        PAGE_FAULT_CNT += 1;

        /* If the fault is a true fault, show info and exit. */
        printf!(
            "Page fault at %p: %s error %s page in %s context.\n",
            fault_addr,
            if not_present {
                "not present".as_bytes()
            } else {
                "rights violation".as_bytes()
            },
            if write {
                "writing".as_bytes()
            } else {
                "reading".as_bytes()
            },
            if user {
                "user".as_bytes()
            } else {
                "kernel".as_bytes()
            },
        );
        kill(f);
    }
}

/* Registers handlers for interrupts that can be caused by user
programs. */
pub unsafe fn exception_init() {
    unsafe {
        /* These exceptions can be raised explicitly by a user
        program, e.g. via the INT, INT3, INTO, and BOUND
        instructions.  Thus, DPL==3. */
        intr_register_int(
            3,
            3,
            IntrLevel::On,
            kill,
            b"#BP Breakpoint Exception\0".as_ptr(),
        );
        intr_register_int(
            4,
            3,
            IntrLevel::On,
            kill,
            b"#OF Overflow Exception\0".as_ptr(),
        );
        intr_register_int(
            5,
            3,
            IntrLevel::On,
            kill,
            b"#BR BOUND Range Exceeded Exception\0".as_ptr(),
        );

        /* These exceptions have DPL==0. */
        intr_register_int(0, 0, IntrLevel::On, kill, b"#DE Divide Error\0".as_ptr());
        intr_register_int(1, 0, IntrLevel::On, kill, b"#DB Debug Exception\0".as_ptr());
        intr_register_int(
            6,
            0,
            IntrLevel::On,
            kill,
            b"#UD Invalid Opcode Exception\0".as_ptr(),
        );
        intr_register_int(
            7,
            0,
            IntrLevel::On,
            kill,
            b"#NM Device Not Available Exception\0".as_ptr(),
        );
        intr_register_int(
            11,
            0,
            IntrLevel::On,
            kill,
            b"#NP Segment Not Present\0".as_ptr(),
        );
        intr_register_int(
            12,
            0,
            IntrLevel::On,
            kill,
            b"#SS Stack Fault Exception\0".as_ptr(),
        );
        intr_register_int(
            13,
            0,
            IntrLevel::On,
            kill,
            b"#GP General Protection Exception\0".as_ptr(),
        );
        intr_register_int(
            16,
            0,
            IntrLevel::On,
            kill,
            b"#MF x87 FPU Floating-Point Error\0".as_ptr(),
        );
        intr_register_int(
            19,
            0,
            IntrLevel::On,
            kill,
            b"#XF SIMD Floating-Point Exception\0".as_ptr(),
        );

        /* Page faults run with interrupts off: the fault address
        is in CR2 and must be read before it can change. */
        intr_register_int(
            14,
            0,
            IntrLevel::Off,
            page_fault,
            b"#PF Page-Fault Exception\0".as_ptr(),
        );
    }
}

/* Prints exception statistics. */
pub unsafe fn exception_print_stats() {
    unsafe {
        printf!("Exception: %lld page faults\n", PAGE_FAULT_CNT);
    }
}

/* Keep the handler-fn type referenced. */
#[allow(unused_imports)]
use IntrHandlerFunc as _;
