//! Port of `threads/malloc.c` — a simple malloc() with segregated
//! power-of-2 free lists.

use crate::{
    lib::{
        kernel::list::{
            List, ListElem, list_empty, list_init, list_pop_front, list_push_back, list_push_front,
            list_remove,
        },
        string::{memcpy, memset},
    },
    threads::{
        palloc::{palloc_free_multiple, palloc_free_page, palloc_get_multiple, palloc_get_page},
        synch::{Lock, lock_acquire, lock_init, lock_release},
        vaddr::{PGSIZE, pg_ofs, pg_round_down},
    },
};

/* Descriptor. */
#[repr(C)]
struct Desc {
    block_size: usize,       /* Size of each element in bytes. */
    blocks_per_arena: usize, /* Number of blocks in an arena. */
    free_list: List,         /* List of free blocks. */
    lock: Lock,              /* Lock. */
}

/* Magic number for detecting arena corruption. */
const ARENA_MAGIC: u32 = 0x9a548eed;

/* Arena. */
#[repr(C)]
struct Arena {
    magic: u32,      /* Always set to ARENA_MAGIC. */
    desc: *mut Desc, /* Owning descriptor, null for big block. */
    free_cnt: usize, /* Free blocks; pages in big block. */
}

/* Free block. */
#[repr(C)]
struct Block {
    free_elem: ListElem, /* Free list element. */
}

/* Our set of descriptors. */
static mut DESCS: [Desc; 10] = [const {
    Desc {
        block_size: 0,
        blocks_per_arena: 0,
        free_list: List::EMPTY,
        lock: Lock::EMPTY,
    }
}; 10];
static mut DESC_CNT: usize = 0; /* Number of descriptors. */

/* Initializes the malloc() descriptors. */
pub unsafe fn malloc_init() {
    unsafe {
        let mut block_size: usize = 16;
        while block_size < PGSIZE as usize / 2 {
            let d = DESCS.as_mut_ptr().add(DESC_CNT);
            DESC_CNT += 1;
            crate::ASSERT!(DESC_CNT <= DESCS.len());
            (*d).block_size = block_size;
            (*d).blocks_per_arena = (PGSIZE as usize - core::mem::size_of::<Arena>()) / block_size;
            list_init(&raw mut (*d).free_list);
            lock_init(&raw mut (*d).lock);
            block_size *= 2;
        }
    }
}

/* Obtains and returns a new block of at least SIZE bytes.
Returns a null pointer if memory is not available. */
pub unsafe fn malloc(size: usize) -> *mut u8 {
    unsafe {
        /* A null pointer satisfies a request for 0 bytes. */
        if size == 0 {
            return core::ptr::null_mut();
        }

        /* Find the smallest descriptor that satisfies a SIZE-byte
        request. */
        let mut d: *mut Desc = DESCS.as_mut_ptr();
        while (d as usize - DESCS.as_ptr() as usize) / core::mem::size_of::<Desc>() < DESC_CNT {
            if (*d).block_size >= size {
                break;
            }
            d = d.add(1);
        }

        let desc_idx = (d as usize - DESCS.as_ptr() as usize) / core::mem::size_of::<Desc>();
        if desc_idx == DESC_CNT {
            /* SIZE is too big for any descriptor.
            Allocate enough pages to hold SIZE plus an arena. */
            let page_cnt =
                (size + core::mem::size_of::<Arena>() + PGSIZE as usize - 1) / PGSIZE as usize;
            let a = palloc_get_multiple(0, page_cnt) as *mut Arena;
            if a.is_null() {
                return core::ptr::null_mut();
            }

            /* Initialize the arena to indicate a big block of
            PAGE_CNT pages, and return it. */
            (*a).magic = ARENA_MAGIC;
            (*a).desc = core::ptr::null_mut();
            (*a).free_cnt = page_cnt;
            return a.add(1) as *mut u8;
        }

        lock_acquire(&raw mut (*d).lock);

        /* If the free list is empty, create a new arena. */
        if list_empty(&raw const (*d).free_list) {
            /* Allocate a page. */
            let a = palloc_get_page(0) as *mut Arena;
            if a.is_null() {
                lock_release(&raw mut (*d).lock);
                return core::ptr::null_mut();
            }

            /* Initialize arena and add its blocks to the free
            list. */
            (*a).magic = ARENA_MAGIC;
            (*a).desc = d;
            (*a).free_cnt = (*d).blocks_per_arena;
            for i in 0..(*d).blocks_per_arena {
                let b = arena_to_block(a, i);
                list_push_back(&raw mut (*d).free_list, &raw mut (*b).free_elem);
            }
        }

        /* Get a block from free list and return it. */
        let b = list_pop_front(&raw mut (*d).free_list) as *mut Block;
        let a = block_to_arena(b);
        (*a).free_cnt -= 1;
        lock_release(&raw mut (*d).lock);
        b as *mut u8
    }
}

/* Allocates and returns A times B bytes initialized to zeroes.
Returns a null pointer if memory is not available. */
pub unsafe fn calloc(a: usize, b: usize) -> *mut u8 {
    unsafe {
        /* Calculate block size and make sure it fits in usize. */
        let size = a.wrapping_mul(b);
        if size < a || size < b {
            return core::ptr::null_mut();
        }

        /* Allocate and zero memory. */
        let p = malloc(size);
        if !p.is_null() {
            memset(p, 0, size);
        }

        p
    }
}

/* Returns the number of bytes allocated for BLOCK. */
unsafe fn block_size(block: *mut u8) -> usize {
    unsafe {
        let a = block_to_arena(block as *mut Block);
        let d = (*a).desc;

        if !d.is_null() {
            (*d).block_size
        } else {
            (PGSIZE as usize * (*a).free_cnt - pg_ofs(block as u64) as usize) as usize
        }
    }
}

/* Attempts to resize OLD_BLOCK to NEW_SIZE bytes, possibly moving
it in the process.  If successful, returns the new block; on
failure, returns a null pointer.  A call with null OLD_BLOCK is
equivalent to malloc(NEW_SIZE).  A call with zero NEW_SIZE is
equivalent to free(OLD_BLOCK). */
pub unsafe fn realloc(old_block: *mut u8, new_size: usize) -> *mut u8 {
    unsafe {
        if new_size == 0 {
            free(old_block);
            core::ptr::null_mut()
        } else {
            let new_block = malloc(new_size);
            if !old_block.is_null() && !new_block.is_null() {
                let old_size = block_size(old_block);
                let min_size = if new_size < old_size {
                    new_size
                } else {
                    old_size
                };
                memcpy(new_block, old_block, min_size);
                free(old_block);
            }
            new_block
        }
    }
}

/* Frees block P, which must have been previously allocated with
malloc(), calloc(), or realloc(). */
pub unsafe fn free(p: *mut u8) {
    unsafe {
        if !p.is_null() {
            let b = p as *mut Block;
            let a = block_to_arena(b);
            let d = (*a).desc;

            if !d.is_null() {
                /* It's a normal block.  We handle it here. */

                /* Clear the block to help detect use-after-free
                bugs. */
                memset(b as *mut u8, 0xcc, (*d).block_size);

                lock_acquire(&raw mut (*d).lock);

                /* Add block to free list. */
                list_push_front(&raw mut (*d).free_list, &raw mut (*b).free_elem);

                /* If the arena is now entirely unused, free it. */
                (*a).free_cnt += 1;
                if (*a).free_cnt >= (*d).blocks_per_arena {
                    crate::ASSERT!((*a).free_cnt == (*d).blocks_per_arena);
                    for i in 0..(*d).blocks_per_arena {
                        let b = arena_to_block(a, i);
                        list_remove(&raw mut (*b).free_elem);
                    }
                    palloc_free_page(a as *mut u8);
                }

                lock_release(&raw mut (*d).lock);
            } else {
                /* It's a big block.  Free its pages. */
                palloc_free_multiple(a as *mut u8, (*a).free_cnt);
            }
        }
    }
}

/* Returns the arena that block B is inside. */
unsafe fn block_to_arena(b: *mut Block) -> *mut Arena {
    unsafe {
        let a = pg_round_down(b as u64) as *mut Arena;

        /* Check that the arena is valid. */
        crate::ASSERT!((*a).magic == ARENA_MAGIC);

        /* Check that the block is properly aligned for the
        arena. */
        crate::ASSERT!(
            (*a).desc.is_null()
                || ((pg_ofs(b as u64) as usize).wrapping_sub(core::mem::size_of::<Arena>()))
                    % (*(*a).desc).block_size
                    == 0
        );
        crate::ASSERT!(
            (*a).desc.is_null() || pg_ofs(b as u64) as usize == core::mem::size_of::<Arena>()
        );

        a
    }
}

/* Returns the IDX'th block within arena A. */
unsafe fn arena_to_block(a: *mut Arena, idx: usize) -> *mut Block {
    unsafe {
        crate::ASSERT!((*a).magic == ARENA_MAGIC);
        crate::ASSERT!(idx < (*(*a).desc).blocks_per_arena);
        (a as *mut u8).add(core::mem::size_of::<Arena>() + idx * (*(*a).desc).block_size)
            as *mut Block
    }
}
