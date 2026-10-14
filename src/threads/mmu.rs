//! Port of `threads/mmu.c` + `include/threads/mmu.h` — x86-64 page
//! tables: the page map level 4 (pml4) hierarchy, page-table walks,
//! and per-PTE state (dirty/accessed bits, present-bit clearing).

use crate::{
    intrinsic::{invlpg, lcr3, rcr3},
    lib::string::memcpy,
    threads::{
        init::BASE_PML4,
        palloc::{PAL_ZERO, palloc_free_page, palloc_get_page},
        pte::{
            PDPESHIFT, PDXSHIFT, PML4SHIFT, PTE_A, PTE_D, PTE_P, PTE_U, PTE_W, PTXSHIFT, pdx, pml4,
            pte_addr, ptx,
        },
        vaddr::{PGSIZE, is_user_vaddr, pg_ofs, pg_round_down, ptov, vtop},
    },
};

/* Number of entries in one page-table page: PGSIZE / sizeof(uint64_t*). */
const PGENT_CNT: u64 = PGSIZE / 8;

/* Callback applied to each present pte by pml4_for_each
(C's `typedef bool pte_for_each_func (uint64_t *pte, void *va,
void *aux)'). */
pub type PteForEachFunc = fn(pte: *mut u64, va: *mut u8, aux: *mut u8) -> bool;

unsafe fn pgdir_walk(pdp: *mut u64, va: u64, create: bool) -> *mut u64 {
    unsafe {
        let idx = pdx(va);
        if !pdp.is_null() {
            let pte = *pdp.add(idx as usize);
            if pte & PTE_P == 0 {
                if create {
                    let new_page = palloc_get_page(PAL_ZERO);
                    if !new_page.is_null() {
                        *pdp.add(idx as usize) = vtop(new_page as u64) | PTE_U | PTE_W | PTE_P;
                    } else {
                        return core::ptr::null_mut();
                    }
                } else {
                    return core::ptr::null_mut();
                }
            }
            return ptov(pte_addr(*pdp.add(idx as usize)) + 8 * ptx(va)) as *mut u64;
        }
        core::ptr::null_mut()
    }
}

unsafe fn pdpe_walk(pdpe: *mut u64, va: u64, create: bool) -> *mut u64 {
    unsafe {
        let mut pte: *mut u64 = core::ptr::null_mut();
        /* (Qualified: the `pdpe' parameter shadows the index fn.) */
        let idx = crate::threads::pte::pdpe(va);
        let mut allocated = false;
        if !pdpe.is_null() {
            let pde = *pdpe.add(idx as usize);
            if pde & PTE_P == 0 {
                if create {
                    let new_page = palloc_get_page(PAL_ZERO);
                    if !new_page.is_null() {
                        *pdpe.add(idx as usize) = vtop(new_page as u64) | PTE_U | PTE_W | PTE_P;
                        allocated = true;
                    } else {
                        return core::ptr::null_mut();
                    }
                } else {
                    return core::ptr::null_mut();
                }
            }
            pte = pgdir_walk(
                ptov(pte_addr(*pdpe.add(idx as usize))) as *mut u64,
                va,
                create,
            );
        }
        if pte.is_null() && allocated {
            palloc_free_page(ptov(pte_addr(*pdpe.add(idx as usize))) as *mut u8);
            *pdpe.add(idx as usize) = 0;
        }
        pte
    }
}

/* Returns the address of the page table entry for virtual
 * address VADDR in page map level 4, pml4.
 * If PML4E does not have a page table for VADDR, behavior depends
 * on CREATE.  If CREATE is true, then a new page table is
 * created and a pointer into it is returned.  Otherwise, a null
 * pointer is returned. */
pub unsafe fn pml4e_walk(pml4e: *mut u64, va: u64, create: bool) -> *mut u64 {
    unsafe {
        let mut pte: *mut u64 = core::ptr::null_mut();
        let idx = pml4(va);
        let mut allocated = false;
        if !pml4e.is_null() {
            let pdpe = *pml4e.add(idx as usize);
            if pdpe & PTE_P == 0 {
                if create {
                    let new_page = palloc_get_page(PAL_ZERO);
                    if !new_page.is_null() {
                        *pml4e.add(idx as usize) = vtop(new_page as u64) | PTE_U | PTE_W | PTE_P;
                        allocated = true;
                    } else {
                        return core::ptr::null_mut();
                    }
                } else {
                    return core::ptr::null_mut();
                }
            }
            pte = pdpe_walk(
                ptov(pte_addr(*pml4e.add(idx as usize))) as *mut u64,
                va,
                create,
            );
        }
        if pte.is_null() && allocated {
            palloc_free_page(ptov(pte_addr(*pml4e.add(idx as usize))) as *mut u8);
            *pml4e.add(idx as usize) = 0;
        }
        pte
    }
}

/* Creates a new page map level 4 (pml4) has mappings for kernel
 * virtual addresses, but none for user virtual addresses.
 * Returns the new page directory, or a null pointer if memory
 * allocation fails. */
pub unsafe fn pml4_create() -> *mut u64 {
    unsafe {
        let pml4 = palloc_get_page(0);
        if !pml4.is_null() {
            memcpy(pml4, BASE_PML4 as *const u8, PGSIZE as usize);
        }
        pml4 as *mut u64
    }
}

unsafe fn pt_for_each(
    pt: *mut u64,
    func: PteForEachFunc,
    aux: *mut u8,
    pml4_index: u32,
    pdp_index: u32,
    pdx_index: u32,
) -> bool {
    unsafe {
        for i in 0..PGENT_CNT as u32 {
            let pte = pt.add(i as usize);
            if (*pte) & PTE_P != 0 {
                let va = (((pml4_index as u64) << PML4SHIFT)
                    | ((pdp_index as u64) << PDPESHIFT)
                    | ((pdx_index as u64) << PDXSHIFT)
                    | ((i as u64) << PTXSHIFT)) as *mut u8;
                if !func(pte, va, aux) {
                    return false;
                }
            }
        }
        true
    }
}

unsafe fn pgdir_for_each(
    pdp: *mut u64,
    func: PteForEachFunc,
    aux: *mut u8,
    pml4_index: u32,
    pdp_index: u32,
) -> bool {
    unsafe {
        for i in 0..PGENT_CNT as u32 {
            let pte = ptov(*pdp.add(i as usize));
            if pte & PTE_P != 0 {
                if !pt_for_each(
                    pte_addr(pte) as *mut u64,
                    func,
                    aux,
                    pml4_index,
                    pdp_index,
                    i,
                ) {
                    return false;
                }
            }
        }
        true
    }
}

unsafe fn pdp_for_each(pdp: *mut u64, func: PteForEachFunc, aux: *mut u8, pml4_index: u32) -> bool {
    unsafe {
        for i in 0..PGENT_CNT as u32 {
            let pde = ptov(*pdp.add(i as usize));
            if pde & PTE_P != 0 {
                if !pgdir_for_each(pte_addr(pde) as *mut u64, func, aux, pml4_index, i) {
                    return false;
                }
            }
        }
        true
    }
}

/* Apply FUNC to each available pte entries including kernel's. */
pub unsafe fn pml4_for_each(pml4: *mut u64, func: PteForEachFunc, aux: *mut u8) -> bool {
    unsafe {
        for i in 0..PGENT_CNT as u32 {
            let pdpe = ptov(*pml4.add(i as usize));
            if pdpe & PTE_P != 0 {
                if !pdp_for_each(pte_addr(pdpe) as *mut u64, func, aux, i) {
                    return false;
                }
            }
        }
        true
    }
}

unsafe fn pt_destroy(pt: *mut u64) {
    unsafe {
        for i in 0..PGENT_CNT as u32 {
            let pte = ptov(*pt.add(i as usize));
            if pte & PTE_P != 0 {
                palloc_free_page(pte_addr(pte) as *mut u8);
            }
        }
        palloc_free_page(pt as *mut u8);
    }
}

unsafe fn pgdir_destroy(pdp: *mut u64) {
    unsafe {
        for i in 0..PGENT_CNT as u32 {
            let pte = ptov(*pdp.add(i as usize));
            if pte & PTE_P != 0 {
                pt_destroy(pte_addr(pte) as *mut u64);
            }
        }
        palloc_free_page(pdp as *mut u8);
    }
}

unsafe fn pdpe_destroy(pdpe: *mut u64) {
    unsafe {
        for i in 0..PGENT_CNT as u32 {
            let pde = ptov(*pdpe.add(i as usize));
            if pde & PTE_P != 0 {
                pgdir_destroy(pte_addr(pde) as *mut u64);
            }
        }
        palloc_free_page(pdpe as *mut u8);
    }
}

/* Destroys pml4e, freeing all the pages it references. */
pub unsafe fn pml4_destroy(pml4: *mut u64) {
    unsafe {
        if pml4.is_null() {
            return;
        }
        crate::ASSERT!(pml4 != BASE_PML4);

        /* if PML4 (vaddr) >= 1, it's kernel space by define. */
        let pdpe = ptov(*pml4.add(0));
        if pdpe & PTE_P != 0 {
            pdpe_destroy(pte_addr(pdpe) as *mut u64);
        }
        palloc_free_page(pml4 as *mut u8);
    }
}

/* Loads page directory PD into the CPU's page directory base
 * register. */
pub unsafe fn pml4_activate(pml4: *mut u64) {
    unsafe {
        lcr3(vtop(if !pml4.is_null() {
            pml4 as u64
        } else {
            BASE_PML4 as u64
        }));
    }
}

/* Looks up the physical address that corresponds to user virtual
 * address UADDR in pml4.  Returns the kernel virtual address
 * corresponding to that physical address, or a null pointer if
 * UADDR is unmapped. */
pub unsafe fn pml4_get_page(pml4: *mut u64, uaddr: *const u8) -> *mut u8 {
    unsafe {
        crate::ASSERT!(is_user_vaddr(uaddr as u64));

        let pte = pml4e_walk(pml4, uaddr as u64, false);

        if !pte.is_null() && (*pte & PTE_P) != 0 {
            return (ptov(pte_addr(*pte)) + pg_ofs(uaddr as u64)) as *mut u8;
        }
        core::ptr::null_mut()
    }
}

/* Adds a mapping in page map level 4 PML4 from user virtual page
 * UPAGE to the physical frame identified by kernel virtual address KPAGE.
 * UPAGE must not already be mapped. KPAGE should probably be a page obtained
 * from the user pool with palloc_get_page().
 * If WRITABLE is true, the new page is read/write;
 * otherwise it is read-only.
 * Returns true if successful, false if memory allocation
 * failed. */
pub unsafe fn pml4_set_page(pml4: *mut u64, upage: *mut u8, kpage: *mut u8, rw: bool) -> bool {
    unsafe {
        crate::ASSERT!(pg_ofs(upage as u64) == 0);
        crate::ASSERT!(pg_ofs(kpage as u64) == 0);
        crate::ASSERT!(is_user_vaddr(upage as u64));
        crate::ASSERT!(pml4 != BASE_PML4);

        let pte = pml4e_walk(pml4, upage as u64, true);

        if !pte.is_null() {
            *pte = vtop(kpage as u64) | PTE_P | if rw { PTE_W } else { 0 } | PTE_U;
        }
        !pte.is_null()
    }
}

/* Marks user virtual page UPAGE "not present" in page
 * directory PD.  Later accesses to the page will fault.  Other
 * bits in the page table entry are preserved.
 * UPAGE need not be mapped. */
pub unsafe fn pml4_clear_page(pml4: *mut u64, upage: *mut u8) {
    unsafe {
        crate::ASSERT!(pg_ofs(upage as u64) == 0);
        crate::ASSERT!(is_user_vaddr(upage as u64));

        let pte = pml4e_walk(pml4, upage as u64, false);

        if !pte.is_null() && (*pte & PTE_P) != 0 {
            *pte &= !PTE_P;
            if rcr3() == vtop(pml4 as u64) {
                invlpg(upage as u64);
            }
        }
    }
}

/* Returns true if the PTE for virtual page VPAGE in PML4 is dirty,
 * that is, if the page has been modified since the PTE was
 * installed.
 * Returns false if PML4 contains no PTE for VPAGE. */
pub unsafe fn pml4_is_dirty(pml4: *mut u64, vpage: *const u8) -> bool {
    unsafe {
        let pte = pml4e_walk(pml4, vpage as u64, false);
        !pte.is_null() && (*pte & PTE_D) != 0
    }
}

/* Set the dirty bit to DIRTY in the PTE for virtual page VPAGE
 * in PML4. */
pub unsafe fn pml4_set_dirty(pml4: *mut u64, vpage: *const u8, dirty: bool) {
    unsafe {
        let pte = pml4e_walk(pml4, vpage as u64, false);
        if !pte.is_null() {
            if dirty {
                *pte |= PTE_D;
            } else {
                /* NB: C does `*pte &= ~(uint32_t) PTE_D', which also
                clears the upper 32 bits of the entry; preserved
                verbatim. */
                *pte &= (!(PTE_D as u32)) as u64;
            }

            if rcr3() == vtop(pml4 as u64) {
                invlpg(vpage as u64);
            }
        }
    }
}

/* Returns true if the PTE for virtual page VPAGE in PML4 has been
 * accessed recently, that is, between the time the PTE was
 * installed and the last time it was cleared.  Returns false if
 * PML4 contains no PTE for VPAGE. */
pub unsafe fn pml4_is_accessed(pml4: *mut u64, vpage: *const u8) -> bool {
    unsafe {
        let pte = pml4e_walk(pml4, vpage as u64, false);
        !pte.is_null() && (*pte & PTE_A) != 0
    }
}

/* Sets the accessed bit to ACCESSED in the PTE for virtual page
VPAGE in PD. */
pub unsafe fn pml4_set_accessed(pml4: *mut u64, vpage: *const u8, accessed: bool) {
    unsafe {
        let pte = pml4e_walk(pml4, vpage as u64, false);
        if !pte.is_null() {
            if accessed {
                *pte |= PTE_A;
            } else {
                /* NB: C does `*pte &= ~(uint32_t) PTE_A', which also
                clears the upper 32 bits of the entry; preserved
                verbatim. */
                *pte &= (!(PTE_A as u32)) as u64;
            }

            if rcr3() == vtop(pml4 as u64) {
                invlpg(vpage as u64);
            }
        }
    }
}

/* --------------------- mmu.h macro helpers --------------------- */

/* Returns true if the page table entry PTE is writable
(C's `is_writable' macro). */
#[inline]
pub unsafe fn is_writable(pte: *const u64) -> bool {
    unsafe { (*pte & PTE_W) != 0 }
}

/* Returns true if PTE maps a user page (C's `is_user_pte'). */
#[inline]
pub unsafe fn is_user_pte(pte: *const u64) -> bool {
    unsafe { (*pte & PTE_U) != 0 }
}

/* Returns true if PTE maps a kernel page (C's `is_kern_pte'). */
#[inline]
pub unsafe fn is_kern_pte(pte: *const u64) -> bool {
    unsafe { !is_user_pte(pte) }
}

/* Returns the physical address mapped by PTE, page-rounded down
(C's `pte_get_paddr'). */
#[inline]
pub unsafe fn pte_get_paddr(pte: *const u64) -> u64 {
    unsafe { pg_round_down(*pte) }
}
