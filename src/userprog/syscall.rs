//! Port of `userprog/syscall.c` — system-call entry and dispatch.
//!
//! x86-64 fast path: the `syscall` instruction routes through MSRs;
//! asm/syscall-entry.S builds the interrupt frame and calls
//! `syscall_handler`.  The dispatch implements the project-2
//! system calls (halt/exit/fork/exec/wait, file syscalls, dup2).

use crate::devices::input::input_getc;
use crate::filesys::file::{
    file_close, file_duplicate, file_length, file_read, file_seek, file_tell, file_write,
};
use crate::filesys::{filesys_create, filesys_open, filesys_remove};
use crate::intrinsic::write_msr;
use crate::lib::kernel::console::putbuf;
use crate::lib::string::{strlen, strlcpy};
use crate::threads::interrupt::IntrFrame;
use crate::threads::loader::{SEL_KCSEG, SEL_UCSEG};
use crate::threads::mmu::pml4_get_page;
use crate::threads::palloc::{palloc_free_page, palloc_get_page};
use crate::threads::thread::{self, thread_current, thread_exit};
use crate::threads::vaddr::{PGSIZE, is_user_vaddr};
use crate::userprog::process::{ChildInfo, process_create_initd, process_fork, process_wait};

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

/* System call numbers (lib/syscall-nr.h). */
const SYS_HALT: u64 = 0;
const SYS_EXIT: u64 = 1;
const SYS_FORK: u64 = 2;
const SYS_EXEC: u64 = 3;
const SYS_WAIT: u64 = 4;
const SYS_CREATE: u64 = 5;
const SYS_REMOVE: u64 = 6;
const SYS_OPEN: u64 = 7;
const SYS_FILESIZE: u64 = 8;
const SYS_READ: u64 = 9;
const SYS_WRITE: u64 = 10;
const SYS_SEEK: u64 = 11;
const SYS_TELL: u64 = 12;
const SYS_CLOSE: u64 = 13;
const SYS_MMAP: u64 = 14;
const SYS_MUNMAP: u64 = 15;
const SYS_DUP2: u64 = 22;

/* Largest fd index in thread::fd_table. */
const FD_MAX: i32 = 128;

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
        interrupts until the syscall_entry swaps the userland
        stack to the kernel-mode stack.  Therefore, we mask these
        flags. */
        write_msr(
            MSR_SYSCALL_MASK,
            FLAG_IF | FLAG_TF | FLAG_DF | FLAG_IOPL | FLAG_AC | FLAG_NT,
        );
    }
}

/* ------------------------------------------------------------------------ */
/* User-memory validation                                                    */
/* ------------------------------------------------------------------------ */

/* Terminates the process with exit status -1 (bad pointer). */
unsafe fn exit_bad() -> ! {
    unsafe {
        (*thread_current()).exit_status = -1;
        thread_exit();
    }
}

/* Returns true if the user virtual address is mapped in the
current process's page table. */
unsafe fn user_addr_ok(uaddr: u64) -> bool {
    unsafe {
        is_user_vaddr(uaddr)
            && !pml4_get_page((*thread_current()).pml4, uaddr as *const u8).is_null()
    }
}

/* Validates a user string (must be fully mapped, NUL included).
Returns the same pointer on success; exits on failure. */
unsafe fn validate_str(s: *const u8) -> *const u8 {
    unsafe {
        let mut p = s;
        loop {
            if !user_addr_ok(p as u64) {
                exit_bad();
            }
            if *p == 0 {
                return s;
            }
            p = p.add(1);
        }
    }
}

/* Validates a user buffer of SIZE bytes. */
unsafe fn validate_buf(buf: *const u8, size: usize) -> *const u8 {
    unsafe {
        if size == 0 {
            return buf;
        }
        let start = buf as u64;
        let end = start + size as u64;
        if end < start {
            exit_bad();
        }
        let mut page = start & !(PGSIZE - 1);
        while page < end {
            if !user_addr_ok(page) {
                exit_bad();
            }
            page += PGSIZE;
        }
        buf
    }
}

/* ------------------------------------------------------------------------ */
/* File-descriptor helpers                                                   */
/* ------------------------------------------------------------------------ */

/* Looks up the file for FD, exiting with -1 on bad input. */
unsafe fn file_for_fd(fd: i64) -> *mut crate::filesys::file::File {
    unsafe {
        if fd < 2 || fd >= FD_MAX as i64 {
            exit_bad();
        }
        let f = (*thread_current()).fd_table[fd as usize];
        if f.is_null() {
            exit_bad();
        }
        f
    }
}

/* ------------------------------------------------------------------------ */
/* The main system call interface                                            */
/* ------------------------------------------------------------------------ */

#[unsafe(no_mangle)]
pub extern "C" fn syscall_handler(f: *mut IntrFrame) {
    unsafe {
        /* SysV argument registers, in order. */
        let rdi = (*f).R.rdi;
        let rsi = (*f).R.rsi;
        let rdx = (*f).R.rdx;

        match (*f).R.rax {
            SYS_HALT => {
                crate::threads::init::power_off();
            }

            SYS_EXIT => {
                (*thread_current()).exit_status = rdi as i32;
                thread_exit();
            }

            SYS_FORK => {
                let name = validate_str(rdi as *const u8);
                (*f).R.rax = process_fork(name, f) as u64;
            }

            SYS_EXEC => {
                /* Copy the command line into a kernel page first:
                the user buffer may disappear under us. */
                let cmd = validate_str(rdi as *const u8);
                let len = strlen(cmd) + 1;
                let page = palloc_get_page(0) as *mut u8;
                if page.is_null() {
                    (*f).R.rax = -1i64 as u64;
                } else {
                    strlcpy(page, cmd, len);
                    let tid = process_create_initd(page);
                    if tid == thread::TID_ERROR {
                        palloc_free_page(page);
                        (*f).R.rax = -1i64 as u64;
                    } else {
                        /* Handshake: wait for the child's load
                        result. */
                        let info = find_child(tid);
                        crate::threads::synch::sema_down(&raw mut (*info).exec_sema);
                        if !(*info).exec_success {
                            crate::lib::kernel::list::list_remove(&raw mut (*info).elem);
                            crate::threads::malloc::free(info as *mut u8);
                            (*f).R.rax = -1i64 as u64;
                        } else {
                            (*f).R.rax = tid as u64;
                        }
                    }
                }
            }

            SYS_WAIT => {
                (*f).R.rax = process_wait(rdi as thread::Tid) as i64 as u64;
            }

            SYS_CREATE => {
                let name = validate_str(rdi as *const u8);
                (*f).R.rax = filesys_create(name, rsi as i32) as u64;
            }

            SYS_REMOVE => {
                let name = validate_str(rdi as *const u8);
                (*f).R.rax = filesys_remove(name) as u64;
            }

            SYS_OPEN => {
                let name = validate_str(rdi as *const u8);
                let file = filesys_open(name);
                if file.is_null() {
                    (*f).R.rax = -1i64 as u64;
                } else {
                    let cur = thread_current();
                    let mut fd = (*cur).next_fd;
                    while fd < FD_MAX && !(*cur).fd_table[fd as usize].is_null() {
                        fd += 1;
                    }
                    if fd >= FD_MAX {
                        file_close(file);
                        (*f).R.rax = -1i64 as u64;
                    } else {
                        (*cur).fd_table[fd as usize] = file;
                        (*cur).next_fd = fd + 1;
                        (*f).R.rax = fd as u64;
                    }
                }
            }

            SYS_FILESIZE => {
                let file = file_for_fd(rdi as i64);
                (*f).R.rax = file_length(file) as i64 as u64;
            }

            SYS_READ => {
                let buf = validate_buf(rsi as *const u8, rdx as usize);
                if rdi == 0 {
                    /* stdin: the keyboard. */
                    let dst = buf as *mut u8;
                    for i in 0..rdx as usize {
                        *dst.add(i) = input_getc();
                    }
                    (*f).R.rax = rdx;
                } else {
                    let file = file_for_fd(rdi as i64);
                    (*f).R.rax = file_read(file, buf as *mut u8, rdx as i32) as i64 as u64;
                }
            }

            SYS_WRITE => {
                let buf = validate_buf(rsi as *const u8, rdx as usize);
                if rdi == 1 || rdi == 2 {
                    /* stdout/stderr: the console. */
                    putbuf(buf, rdx as usize);
                    (*f).R.rax = rdx;
                } else {
                    let file = file_for_fd(rdi as i64);
                    (*f).R.rax = file_write(file, buf, rdx as i32) as i64 as u64;
                }
            }

            SYS_SEEK => {
                let file = file_for_fd(rdi as i64);
                file_seek(file, rsi as i32);
                (*f).R.rax = 0;
            }

            SYS_TELL => {
                let file = file_for_fd(rdi as i64);
                (*f).R.rax = file_tell(file) as i64 as u64;
            }

            SYS_CLOSE => {
                let file = file_for_fd(rdi as i64);
                file_close(file);
                (*thread_current()).fd_table[rdi as usize] = core::ptr::null_mut();
            }

            SYS_DUP2 => {
                let old = rdi as i32;
                let new = rsi as i32;
                if old < 2 || old >= FD_MAX || new < 2 || new >= FD_MAX {
                    exit_bad();
                }
                let cur = thread_current();
                let ofile = (*cur).fd_table[old as usize];
                if ofile.is_null() {
                    exit_bad();
                }
                if old == new {
                    (*f).R.rax = new as u64;
                } else {
                    if !(*cur).fd_table[new as usize].is_null() {
                        file_close((*cur).fd_table[new as usize]);
                    }
                    (*cur).fd_table[new as usize] = file_duplicate(ofile);
                    (*f).R.rax = new as u64;
                }
            }

            SYS_MMAP | SYS_MUNMAP => {
                /* Project 3. */
                (*f).R.rax = -1i64 as u64;
            }

            _ => {
                /* Unimplemented system call (chdir etc.: project
                4): report failure rather than killing the
                process. */
                (*f).R.rax = -1i64 as u64;
            }
        }
    }
}

/* Finds the ChildInfo record for TID in the current thread's
children list (must exist). */
unsafe fn find_child(tid: thread::Tid) -> *mut ChildInfo {
    unsafe {
        use crate::lib::kernel::list::{list_begin, list_end, list_entry, list_next};
        let cur = thread_current();
        let mut e = list_begin(&raw const (*cur).children);
        while e != list_end(&raw const (*cur).children) {
            let info = list_entry::<ChildInfo>(e, ChildInfo::ELEM_OFFSET);
            if (*info).tid == tid {
                return info;
            }
            e = list_next(e);
        }
        crate::PANIC!("exec: child record missing");
    }
}
