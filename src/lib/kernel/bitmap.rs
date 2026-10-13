//! Port of `lib/kernel/bitmap.c` + `include/lib/kernel/bitmap.h` —
//! bitmap abstract data type.
//!
//! From the outside, a bitmap is an array of bits.  From the inside,
//! it's an array of `u64` (C's `unsigned long`, 64-bit on x86-64)
//! that simulates an array of bits.
//!
//! The FILESYS-only file input/output helpers (`bitmap_file_size`,
//! `bitmap_read`, `bitmap_write`) are not ported: they depend on
//! `filesys/file.c`.

use crate::threads::malloc::{free, malloc};

/* From the outside, a bitmap is an array of bits.  From the
inside, it's an array of elem_type (defined below) that
simulates an array of bits. */
#[repr(C)]
pub struct Bitmap {
    pub bit_cnt: usize, /* Number of bits. */
    pub bits: *mut u64, /* Elements that represent bits. */
}

/* Element type.

This must be an unsigned integer type at least as wide as int
(C's `unsigned long'; u64 on x86-64).

Each bit represents one bit in the bitmap.
If bit 0 in an element represents bit K in the bitmap,
then bit 1 in the element represents bit K+1 in the bitmap,
and so on. */
type ElemType = u64;

/* Number of bits in an element. */
const ELEM_BITS: usize = core::mem::size_of::<ElemType>() * 8;

/* BITMAP_ERROR (C: SIZE_MAX). */
pub const BITMAP_ERROR: usize = usize::MAX;

/* Returns the index of the element that contains the bit
numbered BIT_IDX. */
fn elem_idx(bit_idx: usize) -> usize {
    bit_idx / ELEM_BITS
}

/* Returns an elem_type where only the bit corresponding to
BIT_IDX is turned on. */
fn bit_mask(bit_idx: usize) -> ElemType {
    1 << (bit_idx % ELEM_BITS)
}

/* Returns the number of elements required for BIT_CNT bits
(C's DIV_ROUND_UP from lib/round.h). */
fn elem_cnt(bit_cnt: usize) -> usize {
    (bit_cnt + (ELEM_BITS - 1)) / ELEM_BITS
}

/* Returns the number of bytes required for BIT_CNT bits. */
fn byte_cnt(bit_cnt: usize) -> usize {
    core::mem::size_of::<ElemType>() * elem_cnt(bit_cnt)
}

/* Returns a bit mask in which the bits actually used in the last
element of B's bits are set to 1 and the rest are set to 0. */
unsafe fn last_mask(b: *const Bitmap) -> ElemType {
    let last_bits = unsafe { (*b).bit_cnt % ELEM_BITS };
    if last_bits != 0 {
        (1 << last_bits) - 1
    } else {
        !0
    }
}

/* Creation and destruction. */

/* Creates and returns a bitmap with BIT_CNT bits of storage,
all set to false, or a null pointer if memory allocation
failed. */
pub unsafe fn bitmap_create(bit_cnt: usize) -> *mut Bitmap {
    unsafe {
        let b = malloc(core::mem::size_of::<Bitmap>()) as *mut Bitmap;
        if !b.is_null() {
            (*b).bit_cnt = bit_cnt;
            (*b).bits = malloc(byte_cnt(bit_cnt)) as *mut u64;
            if !(*b).bits.is_null() || bit_cnt == 0 {
                bitmap_set_all(b, false);
                return b;
            }
            free(b as *mut u8);
        }
        core::ptr::null_mut()
    }
}

/* Creates and returns a bitmap with BIT_CNT bits in the
storage preallocated at BUF, which must provide at least
bitmap_buf_size (BIT_CNT) bytes.  (C: bitmap_create_in_buf
(BIT_CNT, BLOCK, BLOCK_SIZE).) */
pub unsafe fn bitmap_create_in_buf(
    bit_cnt: usize,
    buf: *mut u64,
    bit_cnt_in_buf: usize,
) -> *mut Bitmap {
    crate::ASSERT!(bit_cnt_in_buf >= bitmap_buf_size(bit_cnt));

    unsafe {
        let b = buf as *mut Bitmap;

        (*b).bit_cnt = bit_cnt;
        (*b).bits = b.add(1) as *mut u64;
        bitmap_set_all(b, false);
        b
    }
}

/* Returns the number of bytes required to accommodate a bitmap
with BIT_CNT bits (for use with bitmap_create_in_buf()). */
pub fn bitmap_buf_size(bit_cnt: usize) -> usize {
    core::mem::size_of::<Bitmap>() + byte_cnt(bit_cnt)
}

/* Destroys bitmap B, freeing its storage.
Not for use on bitmaps created by
bitmap_create_in_buf(). */
pub unsafe fn bitmap_destroy(b: *mut Bitmap) {
    unsafe {
        if !b.is_null() {
            free((*b).bits as *mut u8);
            free(b as *mut u8);
        }
    }
}

/* Bitmap size. */

/* Returns the number of bits in B. */
pub unsafe fn bitmap_size(b: *const Bitmap) -> usize {
    unsafe { (*b).bit_cnt }
}

/* Setting and testing single bits. */

/* Atomically sets the bit numbered IDX in B to VALUE. */
pub unsafe fn bitmap_set(b: *mut Bitmap, idx: usize, value: bool) {
    crate::ASSERT!(!b.is_null());
    unsafe {
        crate::ASSERT!(idx < (*b).bit_cnt);
        if value {
            bitmap_mark(b, idx);
        } else {
            bitmap_reset(b, idx);
        }
    }
}

/* Atomically sets the bit numbered BIT_IDX in B to true. */
pub unsafe fn bitmap_mark(b: *mut Bitmap, bit_idx: usize) {
    unsafe {
        let idx = elem_idx(bit_idx);
        let mask = bit_mask(bit_idx);

        /* This is equivalent to `b->bits[idx] |= mask' except that it
        is guaranteed to be atomic on a uniprocessor machine.  See
        the description of the OR instruction in [IA32-v2b]. */
        core::arch::asm!(
            "lock or qword ptr [{bits}], {mask}",
            bits = in(reg) (*b).bits.add(idx),
            mask = in(reg) mask,
        );
    }
}

/* Atomically sets the bit numbered BIT_IDX in B to false. */
pub unsafe fn bitmap_reset(b: *mut Bitmap, bit_idx: usize) {
    unsafe {
        let idx = elem_idx(bit_idx);
        let mask = bit_mask(bit_idx);

        /* This is equivalent to `b->bits[idx] &= ~mask' except that it
        is guaranteed to be atomic on a uniprocessor machine.  See
        the description of the AND instruction in [IA32-v2a]. */
        core::arch::asm!(
            "lock and qword ptr [{bits}], {mask}",
            bits = in(reg) (*b).bits.add(idx),
            mask = in(reg) !mask,
        );
    }
}

/* Atomically toggles the bit numbered IDX in B;
that is, if it is true, makes it false,
and if it is false, makes it true. */
pub unsafe fn bitmap_flip(b: *mut Bitmap, bit_idx: usize) {
    unsafe {
        let idx = elem_idx(bit_idx);
        let mask = bit_mask(bit_idx);

        /* This is equivalent to `b->bits[idx] ^= mask' except that it
        is guaranteed to be atomic on a uniprocessor machine.  See
        the description of the XOR instruction in [IA32-v2b]. */
        core::arch::asm!(
            "lock xor qword ptr [{bits}], {mask}",
            bits = in(reg) (*b).bits.add(idx),
            mask = in(reg) mask,
        );
    }
}

/* Returns the value of the bit numbered IDX in B. */
pub unsafe fn bitmap_test(b: *const Bitmap, idx: usize) -> bool {
    crate::ASSERT!(!b.is_null());
    unsafe {
        crate::ASSERT!(idx < (*b).bit_cnt);
        (*(*b).bits.add(elem_idx(idx))) & bit_mask(idx) != 0
    }
}

/* Setting and testing multiple bits. */

/* Sets all bits in B to VALUE. */
pub unsafe fn bitmap_set_all(b: *mut Bitmap, value: bool) {
    crate::ASSERT!(!b.is_null());
    unsafe {
        bitmap_set_multiple(b, 0, bitmap_size(b), value);
    }
}

/* Sets the CNT bits starting at START in B to VALUE. */
pub unsafe fn bitmap_set_multiple(b: *mut Bitmap, start: usize, cnt: usize, value: bool) {
    crate::ASSERT!(!b.is_null());
    unsafe {
        crate::ASSERT!(start <= (*b).bit_cnt);
        crate::ASSERT!(start + cnt <= (*b).bit_cnt);

        for i in 0..cnt {
            bitmap_set(b, start + i, value);
        }
    }
}

/* Returns the number of bits in B between START and START + CNT,
exclusive, that are set to VALUE. */
pub unsafe fn bitmap_count(b: *const Bitmap, start: usize, cnt: usize, value: bool) -> usize {
    crate::ASSERT!(!b.is_null());
    unsafe {
        crate::ASSERT!(start <= (*b).bit_cnt);
        crate::ASSERT!(start + cnt <= (*b).bit_cnt);

        let mut value_cnt: usize = 0;
        for i in 0..cnt {
            if bitmap_test(b, start + i) == value {
                value_cnt += 1;
            }
        }
        value_cnt
    }
}

/* Returns true if any bits in B between START and START + CNT,
exclusive, are set to VALUE, and false otherwise. */
pub unsafe fn bitmap_contains(b: *const Bitmap, start: usize, cnt: usize, value: bool) -> bool {
    crate::ASSERT!(!b.is_null());
    unsafe {
        crate::ASSERT!(start <= (*b).bit_cnt);
        crate::ASSERT!(start + cnt <= (*b).bit_cnt);

        for i in 0..cnt {
            if bitmap_test(b, start + i) == value {
                return true;
            }
        }
        false
    }
}

/* Returns true if any bits in B between START and START + CNT,
exclusive, are set to true, and false otherwise. */
pub unsafe fn bitmap_any(b: *const Bitmap, start: usize, cnt: usize) -> bool {
    unsafe { bitmap_contains(b, start, cnt, true) }
}

/* Returns true if no bits in B between START and START + CNT,
exclusive, are set to true, and false otherwise. */
pub unsafe fn bitmap_none(b: *const Bitmap, start: usize, cnt: usize) -> bool {
    unsafe { !bitmap_contains(b, start, cnt, true) }
}

/* Returns true if every bit in B between START and START + CNT,
exclusive, is set to true, and false otherwise. */
pub unsafe fn bitmap_all(b: *const Bitmap, start: usize, cnt: usize) -> bool {
    unsafe { !bitmap_contains(b, start, cnt, false) }
}

/* Finding set or unset bits. */

/* Finds and returns the starting index of the first group of CNT
consecutive bits in B at or after START that are all set to
VALUE.
If there is no such group, returns BITMAP_ERROR. */
pub unsafe fn bitmap_scan(b: *const Bitmap, start: usize, cnt: usize, value: bool) -> usize {
    crate::ASSERT!(!b.is_null());
    unsafe {
        crate::ASSERT!(start <= (*b).bit_cnt);

        if cnt <= (*b).bit_cnt {
            let last = (*b).bit_cnt - cnt;

            let mut i = start;
            while i <= last {
                if !bitmap_contains(b, i, cnt, !value) {
                    return i;
                }
                i += 1;
            }
        }
        BITMAP_ERROR
    }
}

/* Finds the first group of CNT consecutive bits in B at or after
START that are all set to VALUE, flips them all to !VALUE,
and returns the index of the first bit in the group.
If there is no such group, returns BITMAP_ERROR.
If CNT is zero, returns 0.
Bits are set atomically, but testing bits is not atomic with
setting them. */
pub unsafe fn bitmap_scan_and_flip(b: *mut Bitmap, start: usize, cnt: usize, value: bool) -> usize {
    unsafe {
        let idx = bitmap_scan(b, start, cnt, value);
        if idx != BITMAP_ERROR {
            bitmap_set_multiple(b, idx, cnt, !value);
        }
        idx
    }
}

/* Debugging. */

/* Dumps the contents of B to the console as hexadecimal. */
pub unsafe fn bitmap_dump(b: *mut Bitmap) {
    unsafe {
        crate::lib::stdio::hex_dump(0, (*b).bits as *const u8, byte_cnt((*b).bit_cnt), false);
    }
}
