//! Port of `threads/palloc.c` — the page allocator.
//!
//! Organizer: hands out memory in page-size (and page-multiple) runs
//! from two pools, one for kernel pages and one for user pages.

use crate::PANIC;
use crate::lib::kernel::bitmap::{
    BITMAP_ERROR, bitmap_all, bitmap_create_in_buf, bitmap_scan_and_flip, bitmap_set_all,
    bitmap_set_multiple, bitmap_size,
};
use crate::lib::kernel::list;
use crate::lib::string::memset;
use crate::printf;
use crate::threads::loader::MULTIBOOT_INFO;
use crate::threads::synch::{Lock, lock_acquire, lock_init, lock_release};
use crate::threads::vaddr::{PGSIZE, pg_no, pg_ofs, pg_round_up, ptov};

/* Page allocator flags. */
pub type PallocFlags = u16;
pub const PAL_ASSERT: u16 = 1; /* Panic on failure. */
pub const PAL_ZERO: u16 = 2; /* Zero page contents. */
pub const PAL_USER: u16 = 4; /* User page. */

/* A memory pool. */
#[repr(C)]
struct Pool {
    lock: Lock,                                        /* Mutual exclusion. */
    used_map: *mut crate::lib::kernel::bitmap::Bitmap, /* Bitmap of free pages. */
    base: *mut u8,                                     /* Base of pool. */
}

/* Two pools: one for kernel data, one for user pages. */
static mut KERNEL_POOL: Pool = Pool {
    lock: Lock {
        holder: core::ptr::null_mut(),
        semaphore: crate::threads::synch::Semaphore {
            value: 0,
            waiters: crate::lib::kernel::list::List {
                head: crate::lib::kernel::list::ListElem {
                    prev: core::ptr::null_mut(),
                    next: core::ptr::null_mut(),
                },
                tail: crate::lib::kernel::list::ListElem {
                    prev: core::ptr::null_mut(),
                    next: core::ptr::null_mut(),
                },
            },
        },
    },
    used_map: core::ptr::null_mut(),
    base: core::ptr::null_mut(),
};
static mut USER_POOL: Pool = Pool {
    lock: Lock {
        holder: core::ptr::null_mut(),
        semaphore: crate::threads::synch::Semaphore {
            value: 0,
            waiters: crate::lib::kernel::list::List {
                head: crate::lib::kernel::list::ListElem {
                    prev: core::ptr::null_mut(),
                    next: core::ptr::null_mut(),
                },
                tail: crate::lib::kernel::list::ListElem {
                    prev: core::ptr::null_mut(),
                    next: core::ptr::null_mut(),
                },
            },
        },
    },
    used_map: core::ptr::null_mut(),
    base: core::ptr::null_mut(),
};

/* Maximum number of pages to put in user pool. */
pub static mut USER_PAGE_LIMIT: usize = usize::MAX;

/* multiboot info (written by the loader at physical 0x7000). */
#[repr(C)]
struct MultibootInfo {
    flags: u32,
    mem_low: u32,
    mem_high: u32,
    __unused: [u32; 8],
    mmap_len: u32,
    mmap_base: u32,
}

/* e820 entry. */
#[repr(C)]
struct E820Entry {
    size: u32,
    mem_lo: u32,
    mem_hi: u32,
    len_lo: u32,
    len_hi: u32,
    type_: u32,
}

/* Represent the range information of the ext_mem/base_mem. */
struct Area {
    start: u64,
    end: u64,
    size: u64,
}

const BASE_MEM_THRESHOLD: u64 = 0x100000;
const ACPI_RECLAIMABLE: u32 = 3;
const USABLE: u32 = 1;

#[inline]
fn append_hilo(hi: u32, lo: u32) -> u64 {
    ((hi as u64) << 32) + (lo as u64)
}

#[inline]
fn div_round_up(a: u64, b: u64) -> u64 {
    (a + b - 1) / b
}

fn mb_info() -> *mut MultibootInfo {
    ptov(MULTIBOOT_INFO) as *mut MultibootInfo
}

fn mmap_entries() -> *mut E820Entry {
    unsafe {
        ptov(core::ptr::read_unaligned(&raw const (*mb_info()).mmap_base) as u64) as *mut E820Entry
    }
}

fn mmap_len() -> u32 {
    unsafe { core::ptr::read_unaligned(&raw const (*mb_info()).mmap_len) }
}

/* Iterate on the e820 entry, parse the range of basemem and extmem. */
unsafe fn resolve_area_info(base_mem: &mut Area, ext_mem: &mut Area) {
    unsafe {
        let entries = mmap_entries();
        let cnt = mmap_len() as usize / core::mem::size_of::<E820Entry>();

        for i in 0..cnt {
            let entry = &mut *entries.add(i);
            let ty = core::ptr::read_unaligned(&raw const entry.type_);
            if ty == ACPI_RECLAIMABLE || ty == USABLE {
                let start = append_hilo(
                    core::ptr::read_unaligned(&raw const entry.mem_hi),
                    core::ptr::read_unaligned(&raw const entry.mem_lo),
                );
                let size = append_hilo(
                    core::ptr::read_unaligned(&raw const entry.len_hi),
                    core::ptr::read_unaligned(&raw const entry.len_lo),
                );
                let end = start + size;
                printf!("%llx ~ %llx %d\n", start, end, ty);

                let area: &mut Area = if start < BASE_MEM_THRESHOLD {
                    base_mem
                } else {
                    ext_mem
                };

                /* First entry that belongs to this area. */
                if area.size == 0 {
                    area.start = start;
                    area.end = end;
                    area.size = size;
                } else {
                    /* Extend start / end / size. */
                    if area.start > start {
                        area.start = start;
                    }
                    if area.end < end {
                        area.end = end;
                    }
                    area.size += size;
                }
            }
        }
    }
}

/*
 * Populate the pool.
 * All the pages are managed by this allocator, even including the
 * code page.  Basically, give half of memory to kernel, half to
 * user.  We push the base_mem portion to the kernel as much as
 * possible.
 */
unsafe fn populate_pools(base_mem: &Area, ext_mem: &Area) {
    unsafe {
        extern "C" {
            static _end: u8;
        }
        let mut free_start = pg_round_up(ptov(core::ptr::addr_of!(_end) as u64));

        let total_pages = (base_mem.size + ext_mem.size) / PGSIZE;
        let half = total_pages / 2;
        let user_pages = if half > USER_PAGE_LIMIT as u64 {
            USER_PAGE_LIMIT as u64
        } else {
            half
        };
        let kern_pages = total_pages - user_pages;

        /* Parse the E820 map to claim the memory region for each
        pool. */
        #[derive(PartialEq)]
        enum State {
            KernStart,
            Kern,
            UserStart,
            User,
        }
        let mut state = State::KernStart;
        let mut rem = kern_pages;
        let mut region_start: u64 = 0;
        let mut end: u64 = 0;

        let entries = mmap_entries();
        let cnt = mmap_len() as usize / core::mem::size_of::<E820Entry>();

        for i in 0..cnt {
            let entry = &mut *entries.add(i);
            let ty = core::ptr::read_unaligned(&raw const entry.type_);
            if ty == ACPI_RECLAIMABLE || ty == USABLE {
                let start = ptov(append_hilo(
                    core::ptr::read_unaligned(&raw const entry.mem_hi),
                    core::ptr::read_unaligned(&raw const entry.mem_lo),
                ));
                let size = append_hilo(
                    core::ptr::read_unaligned(&raw const entry.len_hi),
                    core::ptr::read_unaligned(&raw const entry.len_lo),
                );
                end = start + size;
                let size_in_pg = size / PGSIZE;

                if state == State::KernStart {
                    region_start = start;
                    state = State::Kern;
                }

                match state {
                    State::Kern => {
                        if rem > size_in_pg {
                            rem -= size_in_pg;
                            continue;
                        }
                        /* Generate kernel pool. */
                        init_pool(
                            &raw mut KERNEL_POOL,
                            &mut free_start,
                            region_start,
                            start + rem * PGSIZE,
                        );
                        /* Transition to the next state. */
                        if rem == size_in_pg {
                            rem = user_pages;
                            state = State::UserStart;
                        } else {
                            region_start = start + rem * PGSIZE;
                            rem = user_pages - size_in_pg + rem;
                            state = State::User;
                        }
                    }
                    State::UserStart => {
                        region_start = start;
                        state = State::User;
                    }
                    State::User => {
                        if rem > size_in_pg {
                            rem -= size_in_pg;
                        } else {
                            crate::ASSERT!(rem == size);
                        }
                    }
                    State::KernStart => unreachable!(),
                }
            }
        }

        /* Generate the user pool. */
        init_pool(&raw mut USER_POOL, &mut free_start, region_start, end);

        /* Iterate over the e820_entry.  Set up the usable pages. */
        let usable_bound = free_start;

        for i in 0..cnt {
            let entry = &mut *entries.add(i);
            let ty = core::ptr::read_unaligned(&raw const entry.type_);
            if ty == ACPI_RECLAIMABLE || ty == USABLE {
                let mut start = ptov(append_hilo(
                    core::ptr::read_unaligned(&raw const entry.mem_hi),
                    core::ptr::read_unaligned(&raw const entry.mem_lo),
                ));
                let size = append_hilo(
                    core::ptr::read_unaligned(&raw const entry.len_hi),
                    core::ptr::read_unaligned(&raw const entry.len_lo),
                );
                let end = start + size;

                /* All the pages are unusable below the bound. */
                if end < usable_bound {
                    continue;
                }

                start = pg_round_up(if start >= usable_bound {
                    start
                } else {
                    usable_bound
                });

                loop {
                    let pool = if page_from_pool(&KERNEL_POOL, start as *mut u8) {
                        &raw mut KERNEL_POOL
                    } else if page_from_pool(&USER_POOL, start as *mut u8) {
                        &raw mut USER_POOL
                    } else {
                        crate::ASSERT!(false);
                        unreachable!()
                    };

                    let pool_end =
                        (*pool).base as u64 + bitmap_size((*pool).used_map) as u64 * PGSIZE;
                    let page_idx = (pg_no(start) - pg_no((*pool).base as u64)) as usize;
                    if pool_end < end {
                        let page_cnt = ((pool_end - start) / PGSIZE) as usize;
                        bitmap_set_multiple((*pool).used_map, page_idx, page_cnt, false);
                        start = pool_end;
                        /* C: goto split. */
                    } else {
                        let page_cnt = ((end - start) / PGSIZE) as usize;
                        bitmap_set_multiple((*pool).used_map, page_idx, page_cnt, false);
                        break;
                    }
                }
            }
        }
    }
}

/* Initializes the page allocator and gets the memory size. */
pub unsafe fn palloc_init() -> u64 {
    unsafe {
        let mut base_mem = Area {
            start: 0,
            end: 0,
            size: 0,
        };
        let mut ext_mem = Area {
            start: 0,
            end: 0,
            size: 0,
        };

        resolve_area_info(&mut base_mem, &mut ext_mem);
        printf!("Pintos booting with: \n");
        printf!(
            "\tbase_mem: 0x%llx ~ 0x%llx (Usable: %'llu kB)\n",
            base_mem.start,
            base_mem.end,
            base_mem.size / 1024
        );
        printf!(
            "\text_mem: 0x%llx ~ 0x%llx (Usable: %'llu kB)\n",
            ext_mem.start,
            ext_mem.end,
            ext_mem.size / 1024
        );
        populate_pools(&base_mem, &ext_mem);
        ext_mem.end
    }
}

/* Obtains and returns a group of PAGE_CNT contiguous free pages.
If PAL_USER is set, the pages are obtained from the user pool,
otherwise from the kernel pool.  If PAL_ZERO is set in FLAGS, then
the pages are filled with zeros.  If too few pages are available,
returns a null pointer, unless PAL_ASSERT is set in FLAGS, in
which case the kernel panics. */
pub unsafe fn palloc_get_multiple(flags: PallocFlags, page_cnt: usize) -> *mut u8 {
    unsafe {
        let pool = if flags & PAL_USER != 0 {
            &raw mut USER_POOL
        } else {
            &raw mut KERNEL_POOL
        };

        lock_acquire(&raw mut (*pool).lock);
        let page_idx = bitmap_scan_and_flip((*pool).used_map, 0, page_cnt, false);
        lock_release(&raw mut (*pool).lock);

        let pages = if page_idx != BITMAP_ERROR {
            (*pool).base.add(PGSIZE as usize * page_idx)
        } else {
            core::ptr::null_mut()
        };

        if !pages.is_null() {
            if flags & PAL_ZERO != 0 {
                memset(pages, 0, PGSIZE as usize * page_cnt);
            }
        } else if flags & PAL_ASSERT != 0 {
            PANIC!("palloc_get: out of pages");
        }

        pages
    }
}

/* Obtains a single free page and returns its kernel virtual
address. */
pub unsafe fn palloc_get_page(flags: PallocFlags) -> *mut u8 {
    unsafe { palloc_get_multiple(flags, 1) }
}

/* Frees the PAGE_CNT pages starting at PAGES. */
pub unsafe fn palloc_free_multiple(pages: *mut u8, page_cnt: usize) {
    unsafe {
        crate::ASSERT!(pg_ofs(pages as u64) == 0);
        if pages.is_null() || page_cnt == 0 {
            return;
        }

        let pool = if page_from_pool(&KERNEL_POOL, pages) {
            &raw mut KERNEL_POOL
        } else if page_from_pool(&USER_POOL, pages) {
            &raw mut USER_POOL
        } else {
            crate::ASSERT!(false);
            unreachable!()
        };

        let page_idx = (pg_no(pages as u64) - pg_no((*pool).base as u64)) as usize;

        memset(pages, 0xcc, PGSIZE as usize * page_cnt);
        crate::ASSERT!(bitmap_all((*pool).used_map, page_idx, page_cnt));
        bitmap_set_multiple((*pool).used_map, page_idx, page_cnt, false);
    }
}

/* Frees the page at PAGE. */
pub unsafe fn palloc_free_page(page: *mut u8) {
    unsafe { palloc_free_multiple(page, 1) }
}

/* Initializes pool P as starting at START and ending at END. */
unsafe fn init_pool(p: *mut Pool, bm_base: &mut u64, start: u64, end: u64) {
    unsafe {
        /* We'll put the pool's used_map at its base.  Calculate the
        space needed for the bitmap and subtract it from the pool's
        size. */
        let pgcnt = (end - start) / PGSIZE;
        let bm_pages = div_round_up(
            crate::lib::kernel::bitmap::bitmap_buf_size(pgcnt as usize) as u64,
            PGSIZE,
        ) * PGSIZE;

        lock_init(&raw mut (*p).lock);
        (*p).used_map =
            bitmap_create_in_buf(pgcnt as usize, *bm_base as *mut u64, bm_pages as usize);
        (*p).base = start as *mut u8;

        /* Mark all to unusable. */
        bitmap_set_all((*p).used_map, true);

        *bm_base += bm_pages;
    }
}

/* Returns true if PAGE was allocated from POOL, false otherwise. */
unsafe fn page_from_pool(pool: *const Pool, page: *const u8) -> bool {
    unsafe {
        let page_no = pg_no(page as u64);
        let start_page = pg_no((*pool).base as u64);
        let end_page = start_page + bitmap_size((*pool).used_map) as u64;
        page_no >= start_page && page_no < end_page
    }
}

/* Keep the list import referenced (pool uses Lock which embeds List). */
#[allow(unused_imports)]
use list as _;
