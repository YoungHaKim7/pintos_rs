//! Port of `userprog/process.c` — user-process loading and control.
//!
//! Implements the project-2 assignments: argument passing,
//! process wait/exit with termination messages, and fork with
//! address-space and fd-table duplication.  The per-child wait
//! records (`ChildInfo`) are allocated by the parent and outlive
//! the child's thread page.

use crate::filesys::file::{
    file_close, file_deny_write, file_duplicate, file_length, file_read, file_seek, File,
};
use crate::filesys::filesys_open;
use crate::lib::kernel::list::{
    ListElem, list_begin, list_empty, list_end, list_entry, list_next, list_pop_front,
    list_push_back, list_remove,
};
use crate::lib::string::{memcpy, memset, strlen, strlcpy, strtok_r};
use crate::lib::stdio::Arg;
use crate::threads::interrupt::{IntrFrame, intr_disable, intr_set_level};
use crate::threads::loader::{SEL_UCSEG, SEL_UDSEG};
use crate::threads::malloc::{free, malloc};
use crate::threads::mmu::{
    is_user_pte, is_writable, pml4_activate, pml4_create, pml4_destroy, pml4_for_each,
    pml4_get_page, pml4_set_page,
};
use crate::threads::palloc::{palloc_free_page, palloc_get_page, PAL_USER, PAL_ZERO};
use crate::threads::synch::{sema_down, sema_init, sema_up, Semaphore};
use crate::threads::thread::{
    self, Thread, Tid, PRI_DEFAULT, do_iret, thread_current, thread_create, thread_exit,
    thread_name,
};
use crate::threads::vaddr::{PGSIZE, USER_STACK, is_user_vaddr};
use crate::userprog::syscall::{FLAG_IF, FLAG_MBS};
use crate::userprog::tss::tss_update;

/* Record of a child process, owned by the parent's children
list.  Freed by the parent in process_wait / process_exit, or by
the child itself if the parent dies first (orphaned). */
#[repr(C)]
pub struct ChildInfo {
    pub elem: ListElem, /* Link in the parent's children list. */
    pub tid: Tid,       /* Child's thread id. */
    pub exit_status: i32,
    pub wait_sema: Semaphore, /* Up'd by the child when it exits. */
    pub exec_sema: Semaphore, /* Up'd once: load/fork result ready. */
    pub exec_success: bool,   /* Result of the initial exec/fork. */
    pub exec_done: bool,      /* exec_sema already up'd. */
    pub exited: bool,         /* Child has stored its exit status. */
    pub orphaned: bool,       /* Parent died; child frees this itself. */
}

impl ChildInfo {
    pub const ELEM_OFFSET: usize = core::mem::offset_of!(ChildInfo, elem);
}

/* Aux passed to initd: the command-line page, with the ChildInfo
pointer parked at its very end. */
const INFO_OFF: usize = PGSIZE as usize - 8;

/* Aux passed to __do_fork.  Lives on the parent's kernel stack
while the parent blocks in the fork syscall; the child copies out
what it needs before completing the handshake. */
#[repr(C)]
struct ForkCtx {
    parent: *mut Thread,
    parent_if: *mut IntrFrame,
    info: *mut ChildInfo,
}

unsafe fn child_info_new(tid: Tid) -> *mut ChildInfo {
    unsafe {
        let info = malloc(core::mem::size_of::<ChildInfo>()) as *mut ChildInfo;
        if info.is_null() {
            return core::ptr::null_mut();
        }
        (*info).elem = ListElem {
            prev: core::ptr::null_mut(),
            next: core::ptr::null_mut(),
        };
        (*info).tid = tid;
        (*info).exit_status = 0;
        (*info).wait_sema = Semaphore::EMPTY;
        (*info).exec_sema = Semaphore::EMPTY;
        sema_init(&raw mut (*info).wait_sema, 0);
        sema_init(&raw mut (*info).exec_sema, 0);
        (*info).exec_success = false;
        (*info).exec_done = false;
        (*info).exited = false;
        (*info).orphaned = false;
        info
    }
}

/* General process initializer for initd and other processes. */
unsafe fn process_init() {
    unsafe {
        let _current = thread_current();
    }
}

/* Starts the first userland program, called "initd", loaded from
FILE_NAME.
The new thread may be scheduled (and may even exit) before
process_create_initd() returns.  Returns the initd's thread id,
or TID_ERROR if the thread cannot be created.
Notice that THIS SHOULD BE CALLED ONCE. */
pub unsafe fn process_create_initd(file_name: *const u8) -> Tid {
    unsafe {
        /* Make a copy of FILE_NAME.
        Otherwise there's a race between the caller and load(). */
        let fn_copy = palloc_get_page(0) as *mut u8;
        if fn_copy.is_null() {
            return thread::TID_ERROR;
        }
        strlcpy(fn_copy, file_name, PGSIZE as usize);

        /* Child record for process_wait; park it at the end of
        the command-line page where initd picks it up. */
        let info = child_info_new(0);
        if info.is_null() {
            palloc_free_page(fn_copy);
            return thread::TID_ERROR;
        }
        *(fn_copy.add(INFO_OFF) as *mut *mut ChildInfo) = info;

        /* The thread is named after the program (first word). */
        let mut name = [0u8; 16];
        let mut n = 0;
        while n + 1 < name.len() && *file_name.add(n) != 0 && *file_name.add(n) != b' ' {
            name[n] = *file_name.add(n);
            n += 1;
        }

        /* Create a new thread to execute FILE_NAME. */
        let tid = thread_create(name.as_ptr(), PRI_DEFAULT, Some(initd), fn_copy as *mut ());
        if tid == thread::TID_ERROR {
            palloc_free_page(fn_copy);
            free(info as *mut u8);
            return thread::TID_ERROR;
        }
        (*info).tid = tid;
        list_push_back(
            &raw mut (*thread_current()).children,
            &raw mut (*info).elem,
        );
        tid
    }
}

/* A thread function that launches the first user process. */
unsafe extern "C" fn initd(f_name: *mut ()) {
    unsafe {
        let page = f_name as *mut u8;
        let info = *(page.add(INFO_OFF) as *mut *mut ChildInfo);
        (*thread_current()).child_info = info as *mut core::ffi::c_void;

        process_init();

        if process_exec(page) < 0 {
            /* Load failed: die with exit(-1); process_exit
            reports the failure to the waiting exec(). */
            (*thread_current()).exit_status = -1;
            thread_exit();
        }
        /* NOT_REACHED on success: process_exec irets to
        userland. */
    }
}

/* Clones the current process as `name`.  Returns the new
process's thread id, or TID_ERROR if the thread cannot be
created. */
pub unsafe fn process_fork(name: *const u8, if_: *mut IntrFrame) -> Tid {
    unsafe {
        let parent = thread_current();

        let info = child_info_new(0);
        if info.is_null() {
            return thread::TID_ERROR;
        }

        let mut ctx = ForkCtx {
            parent,
            parent_if: if_,
            info,
        };

        let tid = thread_create(
            name,
            PRI_DEFAULT,
            Some(__do_fork),
            &raw mut ctx as *mut ForkCtx as *mut (),
        );
        if tid == thread::TID_ERROR {
            free(info as *mut u8);
            return thread::TID_ERROR;
        }
        (*info).tid = tid;
        list_push_back(&raw mut (*parent).children, &raw mut (*info).elem);

        /* Wait until the child finishes duplicating our context,
        then report success. */
        sema_down(&raw mut (*info).exec_sema);
        if !(*info).exec_success {
            list_remove(&raw mut (*info).elem);
            free(info as *mut u8);
            return thread::TID_ERROR;
        }
        tid
    }
}

/* Duplicate the parent's address space by passing this function
to pml4_for_each. */
fn duplicate_pte(pte: *mut u64, va: *mut u8, aux: *mut u8) -> bool {
    unsafe {
        let current = thread_current();
        let parent = aux as *mut Thread;

        /* 1. If the parent's page is a kernel page, then return
        immediately. */
        if !is_user_pte(pte) {
            return true;
        }

        /* 2. Resolve VA from the parent's page map level 4. */
        let parent_page = pml4_get_page((*parent).pml4, va);

        /* 3. Allocate a new PAL_USER page for the child. */
        let newpage = palloc_get_page(PAL_USER | PAL_ZERO) as *mut u8;
        if newpage.is_null() {
            return false;
        }

        /* 4. Duplicate the parent's page to the new page. */
        memcpy(newpage, parent_page, PGSIZE as usize);
        let writable = is_writable(pte);

        /* 5. Add the new page to the child's page table at VA
        with the parent's writability. */
        if !pml4_set_page((*current).pml4, va, newpage, writable) {
            /* 6. Failed to insert the page: back out. */
            palloc_free_page(newpage);
            return false;
        }
        true
    }
}

/* A thread function that copies the parent's execution context.
parent->tf does not hold the userland context of the process, so
the syscall's interrupt frame travels via ForkCtx. */
unsafe extern "C" fn __do_fork(aux: *mut ()) {
    unsafe {
        let ctx = aux as *mut ForkCtx;
        let parent = (*ctx).parent;
        let parent_if = (*ctx).parent_if;
        let info = (*ctx).info;
        (*thread_current()).child_info = info as *mut core::ffi::c_void;

        let mut if_: IntrFrame = core::mem::zeroed();
        let mut succ = true;

        /* 1. Read the cpu context to local stack. */
        memcpy(
            &raw mut if_ as *mut u8,
            parent_if as *const u8,
            core::mem::size_of::<IntrFrame>(),
        );

        /* 2. Duplicate PT. */
        let current = thread_current();
        (*current).pml4 = pml4_create();
        if (*current).pml4.is_null() {
            succ = false;
        }

        if succ {
            process_activate(current);
            if !pml4_for_each((*parent).pml4, duplicate_pte, parent as *mut u8) {
                succ = false;
            }
        }

        if succ {
            /* Duplicate the fd table (and the executable). */
            (*current).next_fd = (*parent).next_fd;
            for fd in 0..(*parent).fd_table.len() {
                let f = (*parent).fd_table[fd];
                if !f.is_null() {
                    (*current).fd_table[fd] = file_duplicate(f);
                }
            }
            if !(*parent).executable.is_null() {
                (*current).executable = file_duplicate((*parent).executable);
            }
        }

        if succ {
            process_init();

            /* The child of fork() returns 0. */
            if_.R.rax = 0;

            report_exec_result(info, true);

            /* Finally, switch to the newly created process. */
            do_iret(&raw mut if_);
        } else {
            report_exec_result(info, false);
            thread_exit();
        }
    }
}

/* Reports the initial exec/fork result to the parent exactly
once (the exec handshake semaphore). */
unsafe fn report_exec_result(info: *mut ChildInfo, success: bool) {
    unsafe {
        let old_level = intr_disable();
        if !(*info).exec_done {
            (*info).exec_success = success;
            (*info).exec_done = true;
            sema_up(&raw mut (*info).exec_sema);
        }
        intr_set_level(old_level);
    }
}

/* Switch the current execution context to the f_name.
Returns -1 on fail. */
pub unsafe fn process_exec(f_name: *mut u8) -> i32 {
    unsafe {
        let file_name = f_name;

        /* We cannot use the intr_frame in the thread structure.
        This is because when the current thread is rescheduled, it
        stores the execution information to the member. */
        let mut _if: IntrFrame = core::mem::zeroed();
        _if.ds = SEL_UDSEG;
        _if.es = SEL_UDSEG;
        _if.ss = SEL_UDSEG;
        _if.cs = SEL_UCSEG;
        _if.eflags = FLAG_IF | FLAG_MBS;

        /* We first kill the current context. */
        process_cleanup();

        /* And then load the binary. */
        let success = load(file_name, &raw mut _if);

        /* If load failed, quit. */
        palloc_free_page(file_name);
        if !success {
            return -1;
        }

        /* If load succeeded, report to a waiting exec() before
        entering userland (never returns). */
        if success {
            let info = (*thread_current()).child_info as *mut ChildInfo;
            if !info.is_null() {
                report_exec_result(info, true);
            }
        }

        /* Start the switched process. */
        do_iret(&raw mut _if);

        /* NOT_REACHED */
        -1
    }
}

/* Waits for thread TID to die and returns its exit status.  If
it was terminated by the kernel (i.e. killed due to an
exception), returns -1.  If TID is invalid or if it was not a
child of the calling process, or if process_wait() has already
been successfully called for the given TID, returns -1
immediately, without waiting. */
pub unsafe fn process_wait(child_tid: Tid) -> i32 {
    unsafe {
        let curr = thread_current();

        /* Find the child's record. */
        let mut e = list_begin(&raw const (*curr).children);
        while e != list_end(&raw const (*curr).children) {
            let info = list_entry::<ChildInfo>(e, ChildInfo::ELEM_OFFSET);
            if (*info).tid == child_tid {
                /* Wait for it to die and collect the status. */
                sema_down(&raw mut (*info).wait_sema);
                let status = (*info).exit_status;
                list_remove(&raw mut (*info).elem);
                free(info as *mut u8);
                return status;
            }
            e = list_next(e);
        }
        -1
    }
}

/* Exit the process.  This function is called by thread_exit(). */
pub unsafe fn process_exit() {
    unsafe {
        let curr = thread_current();

        /* Only user processes print the termination message. */
        if !(*curr).pml4.is_null() {
            crate::printf!(
                "%s: exit(%d)\n",
                Arg::Raw(thread_name()),
                Arg::I((*curr).exit_status as i64)
            );
        }

        /* Close every open file descriptor. */
        for fd in 0..(*curr).fd_table.len() {
            let f = (*curr).fd_table[fd];
            if !f.is_null() {
                file_close(f);
                (*curr).fd_table[fd] = core::ptr::null_mut();
            }
        }

        /* Re-enable writing to (and close) the executable. */
        if !(*curr).executable.is_null() {
            file_close((*curr).executable);
            (*curr).executable = core::ptr::null_mut();
        }

        /* Orphan our children: each remaining record is either
        already exited (child stored its status; we free it) or
        still running (the child frees the record itself on
        exit).  The flags are consistent because both sides run
        under intr_disable. */
        while !list_empty(&raw const (*curr).children) {
            let e = list_pop_front(&raw mut (*curr).children);
            let info = list_entry::<ChildInfo>(e, ChildInfo::ELEM_OFFSET);
            let old_level = intr_disable();
            if (*info).exited {
                free(info as *mut u8);
            } else {
                (*info).orphaned = true;
            }
            intr_set_level(old_level);
        }

        /* Tell our own parent (if it is still around). */
        let info = (*curr).child_info as *mut ChildInfo;
        if !info.is_null() {
            let old_level = intr_disable();
            if (*info).orphaned {
                free(info as *mut u8);
            } else {
                (*info).exit_status = (*curr).exit_status;
                (*info).exited = true;
                sema_up(&raw mut (*info).wait_sema);
                /* If the initial exec never reported (early
                death), report failure now. */
                if !(*info).exec_done {
                    (*info).exec_success = false;
                    (*info).exec_done = true;
                    sema_up(&raw mut (*info).exec_sema);
                }
            }
            intr_set_level(old_level);
            (*curr).child_info = core::ptr::null_mut();
        }

        process_cleanup();
    }
}

/* Free the current process's resources. */
unsafe fn process_cleanup() {
    unsafe {
        let curr = thread_current();

        /* Destroy the current process's page directory and switch
        back to the kernel-only page directory. */
        let pml4 = (*curr).pml4;
        if !pml4.is_null() {
            /* Correct ordering here is crucial.  We must set
            cur->pml4 to NULL before switching page directories,
            so that a timer interrupt can't switch back to the
            process page directory.  We must activate the base
            page directory before destroying the process's page
            directory, or our active page directory will be one
            that's been freed (and cleared). */
            (*curr).pml4 = core::ptr::null_mut();
            pml4_activate(core::ptr::null_mut());
            pml4_destroy(pml4);
        }
    }
}

/* Sets up the CPU for running user code in the new thread.
This function is called on every context switch. */
pub unsafe fn process_activate(next: *mut Thread) {
    unsafe {
        /* Activate the thread's page tables. */
        if !(*next).pml4.is_null() {
            pml4_activate((*next).pml4);
        } else {
            pml4_activate(crate::threads::init::BASE_PML4);
        }

        /* Set the thread's kernel stack for use in processing
        interrupts. */
        tss_update(next as *mut u8);
    }
}

/* ------------------------------------------------------------------------ */
/* ELF loading                                                               */
/* ------------------------------------------------------------------------ */

/* We load ELF binaries.  The following definitions are taken
from the ELF specification, [ELF1], more-or-less verbatim. */

const EI_NIDENT: usize = 16;

const PT_LOAD: u32 = 1; /* Loadable segment. */
const PT_DYNAMIC: u32 = 2; /* Dynamic linking info. */
const PT_INTERP: u32 = 3; /* Name of dynamic loader. */
const PT_NOTE: u32 = 4; /* Auxiliary info. */
const PT_SHLIB: u32 = 5; /* Reserved. */
const PT_PHDR: u32 = 6; /* Program header table. */
const PT_STACK: u32 = 0x6474_e551; /* Stack segment. */

const PF_W: u32 = 2; /* Writable. */

/* Executable header.  See [ELF1] 1-4 to 1-8. */
#[repr(C)]
struct Elf64Hdr {
    e_ident: [u8; EI_NIDENT],
    e_type: u16,
    e_machine: u16,
    e_version: u32,
    e_entry: u64,
    e_phoff: u64,
    e_shoff: u64,
    e_flags: u32,
    e_ehsize: u16,
    e_phentsize: u16,
    e_phnum: u16,
    e_shentsize: u16,
    e_shnum: u16,
    e_shstrndx: u16,
}

#[repr(C)]
struct Elf64Phdr {
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_paddr: u64,
    p_filesz: u64,
    p_memsz: u64,
    p_align: u64,
}

/* Loads an ELF executable from FILE_NAME into the current
thread.  FILE_NAME's page holds the full command line, tokenized
in place for argument passing.  Stores the executable's entry
point into if_.rip and its initial stack pointer into if_.rsp.
Returns true if successful, false otherwise. */
unsafe fn load(file_name: *mut u8, if_: *mut IntrFrame) -> bool {
    unsafe {
        let t = thread_current();
        let mut ehdr: Elf64Hdr = core::mem::zeroed();
        let mut file: *mut File = core::ptr::null_mut();

        /* Tokenize the command line in place; the page becomes a
        sequence of NUL-terminated words.  argp is a malloc'd
        array of the word pointers. */
        let mut argc: usize = 0;
        {
            let mut p = file_name;
            loop {
                while *p == b' ' {
                    p = p.add(1);
                }
                if *p == 0 {
                    break;
                }
                argc += 1;
                while *p != 0 && *p != b' ' {
                    p = p.add(1);
                }
            }
        }
        let argp = malloc(argc.max(1) * core::mem::size_of::<*mut u8>()) as *mut *mut u8;

        let success: bool = 'load: {
            if argc == 0 {
                crate::printf!("load: empty command line\n");
                break 'load false;
            }

            {
                let mut save_ptr: *mut u8 = core::ptr::null_mut();
                let delims = b" \0".as_ptr();
                let mut token = strtok_r(file_name, delims, &raw mut save_ptr);
                let mut i = 0;
                while !token.is_null() {
                    *argp.add(i) = token;
                    i += 1;
                    token = strtok_r(core::ptr::null_mut(), delims, &raw mut save_ptr);
                }
            }

            /* Allocate and activate the page directory. */
            (*t).pml4 = pml4_create();
            if (*t).pml4.is_null() {
                break 'load false;
            }
            process_activate(thread_current());

            /* Open the executable file (the first word). */
            file = filesys_open(*argp);
            if file.is_null() {
                crate::printf!("load: %s: open failed\n", Arg::Raw(*argp));
                break 'load false;
            }

            /* Read and verify the executable header. */
            if file_read(
                file,
                &raw mut ehdr as *mut Elf64Hdr as *mut u8,
                core::mem::size_of::<Elf64Hdr>() as i32,
            ) != core::mem::size_of::<Elf64Hdr>() as i32
                || ehdr.e_ident[..7] != *b"\x7fELF\x02\x01\x01"
                || ehdr.e_type != 2
                || ehdr.e_machine != 0x3E /* amd64 */
                || ehdr.e_version != 1
                || ehdr.e_phentsize as usize != core::mem::size_of::<Elf64Phdr>()
                || ehdr.e_phnum > 1024
            {
                crate::printf!(
                    "load: %s: error loading executable\n",
                    Arg::Raw(*argp)
                );
                break 'load false;
            }

            /* Read program headers. */
            let mut file_ofs = ehdr.e_phoff as i64;
            for i in 0..ehdr.e_phnum {
                let mut phdr: Elf64Phdr = core::mem::zeroed();

                if file_ofs < 0 || file_ofs > file_length(file) as i64 {
                    break 'load false;
                }
                file_seek(file, file_ofs as i32);

                if file_read(
                    file,
                    &raw mut phdr as *mut Elf64Phdr as *mut u8,
                    core::mem::size_of::<Elf64Phdr>() as i32,
                ) != core::mem::size_of::<Elf64Phdr>() as i32
                {
                    break 'load false;
                }
                file_ofs += core::mem::size_of::<Elf64Phdr>() as i64;
                match phdr.p_type {
                    PT_DYNAMIC | PT_INTERP | PT_SHLIB => break 'load false,
                    PT_LOAD => {
                        if !validate_segment(&phdr, file) {
                            break 'load false;
                        }
                        let writable = (phdr.p_flags & PF_W) != 0;
                        let file_page = phdr.p_offset & !(PGSIZE - 1);
                        let mem_page = phdr.p_vaddr & !(PGSIZE - 1);
                        let page_offset = phdr.p_vaddr & (PGSIZE - 1);
                        let (read_bytes, zero_bytes): (u32, u32);
                        if phdr.p_filesz > 0 {
                            /* Normal segment: read the initial part
                            from disk and zero the rest. */
                            let rb = page_offset + phdr.p_filesz;
                            read_bytes = rb as u32;
                            zero_bytes = (round_up(page_offset + phdr.p_memsz, PGSIZE) - rb)
                                as u32;
                        } else {
                            /* Entirely zero: don't read anything
                            from disk. */
                            read_bytes = 0;
                            zero_bytes = round_up(page_offset + phdr.p_memsz, PGSIZE) as u32;
                        }
                        if !load_segment(
                            file,
                            file_page as i64,
                            mem_page as *mut u8,
                            read_bytes,
                            zero_bytes,
                            writable,
                        ) {
                            break 'load false;
                        }
                    }
                    _ => {
                        /* Ignore this segment. */
                    }
                }
            }

            /* Set up the stack at USER_STACK. */
            if !setup_stack(if_) {
                break 'load false;
            }

            /* Argument passing: push argv onto the new stack. */
            if !setup_arguments(if_, argp, argc) {
                break 'load false;
            }

            /* Start address. */
            (*if_).rip = ehdr.e_entry;

            /* Deny writes to the running executable. */
            file_deny_write(file);
            (*t).executable = file;
            file = core::ptr::null_mut(); /* ownership moved to the thread */

            true
        };

        /* done: we arrive here whether the load is successful or
        not. */
        if !file.is_null() {
            file_close(file);
        }
        if !argp.is_null() {
            free(argp as *mut u8);
        }
        success
    }
}

fn round_up(v: u64, to: u64) -> u64 {
    (v + to - 1) & !(to - 1)
}

/* Checks whether PHDR describes a valid, loadable segment in
FILE and returns true if so, false otherwise. */
unsafe fn validate_segment(phdr: *const Elf64Phdr, file: *mut File) -> bool {
    unsafe {
        /* p_offset and p_vaddr must have the same page offset. */
        if ((*phdr).p_offset & (PGSIZE - 1)) != ((*phdr).p_vaddr & (PGSIZE - 1)) {
            return false;
        }

        /* p_offset must point within FILE. */
        if (*phdr).p_offset > file_length(file) as u64 {
            return false;
        }

        /* p_memsz must be at least as big as p_filesz. */
        if (*phdr).p_memsz < (*phdr).p_filesz {
            return false;
        }

        /* The segment must not be empty. */
        if (*phdr).p_memsz == 0 {
            return false;
        }

        /* The virtual memory region must both start and end
        within the user address space range. */
        if !is_user_vaddr((*phdr).p_vaddr) {
            return false;
        }
        if !is_user_vaddr((*phdr).p_vaddr + (*phdr).p_memsz) {
            return false;
        }

        /* The region cannot "wrap around" across the kernel
        virtual address space. */
        if (*phdr).p_vaddr + (*phdr).p_memsz < (*phdr).p_vaddr {
            return false;
        }

        /* Disallow mapping page 0. */
        if (*phdr).p_vaddr < PGSIZE {
            return false;
        }

        /* It's okay. */
        true
    }
}

/* Loads a segment starting at offset OFS in FILE at address
UPAGE.  In total, READ_BYTES + ZERO_BYTES bytes of virtual
memory are initialized: READ_BYTES at UPAGE are read from FILE
at OFS, and ZERO_BYTES at UPAGE + READ_BYTES are zeroed.
Returns true if successful, false if a memory allocation error
or disk read error occurs. */
unsafe fn load_segment(
    file: *mut File,
    ofs: i64,
    upage: *mut u8,
    read_bytes: u32,
    zero_bytes: u32,
    writable: bool,
) -> bool {
    unsafe {
        let mut read_bytes = read_bytes;
        let mut zero_bytes = zero_bytes;
        let mut upage = upage;

        crate::ASSERT!((read_bytes as u64 + zero_bytes as u64) % PGSIZE == 0);
        crate::ASSERT!(upage as u64 & (PGSIZE - 1) == 0);
        crate::ASSERT!(ofs as u64 % PGSIZE == 0);

        file_seek(file, ofs as i32);
        while read_bytes > 0 || zero_bytes > 0 {
            /* Calculate how to fill this page: read
            PAGE_READ_BYTES from FILE and zero the final
            PAGE_ZERO_BYTES bytes. */
            let page_read_bytes = if (read_bytes as u64) < PGSIZE {
                read_bytes
            } else {
                PGSIZE as u32
            };
            let page_zero_bytes = PGSIZE as u32 - page_read_bytes;

            /* Get a page of memory. */
            let kpage = palloc_get_page(PAL_USER) as *mut u8;
            if kpage.is_null() {
                return false;
            }

            /* Load this page. */
            if file_read(file, kpage, page_read_bytes as i32) != page_read_bytes as i32 {
                palloc_free_page(kpage);
                return false;
            }
            memset(kpage.add(page_read_bytes as usize), 0, page_zero_bytes as usize);

            /* Add the page to the process's address space. */
            if !install_page(upage, kpage, writable) {
                crate::printf!("fail\n");
                palloc_free_page(kpage);
                return false;
            }

            /* Advance. */
            read_bytes -= page_read_bytes;
            zero_bytes -= page_zero_bytes;
            upage = upage.add(PGSIZE as usize);
        }
        true
    }
}

/* Create a minimal stack by mapping a zeroed page at the
USER_STACK. */
unsafe fn setup_stack(if_: *mut IntrFrame) -> bool {
    unsafe {
        let mut success = false;

        let kpage = palloc_get_page(PAL_USER | PAL_ZERO) as *mut u8;
        if !kpage.is_null() {
            success = install_page((USER_STACK - PGSIZE) as *mut u8, kpage, true);
            if success {
                (*if_).rsp = USER_STACK;
            } else {
                palloc_free_page(kpage);
            }
        }
        success
    }
}

/* Project 2 argument passing: pushes the argv strings and the
argv array onto the freshly created user stack, then sets up the
SysV argument registers (argc in RDI, argv in RSI). */
unsafe fn setup_arguments(if_: *mut IntrFrame, argv: *const *mut u8, argc: usize) -> bool {
    unsafe {
        const MAX_ARGS: usize = 128;
        if argc > MAX_ARGS {
            return false;
        }
        let mut argv_addrs = [0u64; MAX_ARGS];

        /* Copy the argument strings, last to first, below
        USER_STACK. */
        let mut rsp = USER_STACK;
        for i in (0..argc).rev() {
            let s = *argv.add(i);
            let len = strlen(s) + 1; /* include the NUL */
            rsp -= len as u64;
            memcpy(rsp as *mut u8, s, len);
            argv_addrs[i] = rsp;
        }

        /* argv[argc] (NULL sentinel), then the pointer array in
        ascending order. */
        rsp -= 8;
        *(rsp as *mut u64) = 0;
        for i in (0..argc).rev() {
            rsp -= 8;
            *(rsp as *mut u64) = argv_addrs[i];
        }

        /* Keep rsp 16-byte aligned at process entry. */
        while rsp % 16 != 0 {
            rsp -= 8;
            *(rsp as *mut u64) = 0;
        }

        (*if_).rsp = rsp;
        (*if_).R.rdi = argc as u64;
        (*if_).R.rsi = rsp;
        true
    }
}

/* Adds a mapping from user virtual address UPAGE to kernel
virtual address KPAGE in the page table.
If WRITABLE is true, the user process may modify the page;
otherwise, it is read-only.
UPAGE must not already be mapped.
Returns true on success, false if UPAGE is already mapped or if
memory allocation fails. */
unsafe fn install_page(upage: *mut u8, kpage: *mut u8, writable: bool) -> bool {
    unsafe {
        let t = thread_current();

        /* Verify that there's not already a page at that virtual
        address, then map our page there. */
        pml4_get_page((*t).pml4, upage).is_null()
            && pml4_set_page((*t).pml4, upage, kpage, writable)
    }
}
