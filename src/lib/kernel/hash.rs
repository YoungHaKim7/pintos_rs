//! Port of `lib/kernel/hash.c` + `include/lib/kernel/hash.h` — hash
//! table with chaining.  This data structure is thoroughly documented
//! in the Tour of Pintos for Project 3.
//!
//! This is a standard hash table with chaining.  To locate an element
//! in the table, we compute a hash function over the element's data
//! and use that as an index into an array of doubly linked lists, then
//! linearly search the list.
//!
//! The chain lists do not use dynamic allocation.  Instead, each
//! structure that can potentially be in a hash must embed a
//! [`HashElem`] member.  All of the hash functions operate on these
//! `HashElem`s.  [`hash_entry`] allows conversion from a `HashElem`
//! back to a structure object that contains it.  This is the same
//! technique used in the linked list implementation; refer to
//! [`crate::lib::kernel::list`] for a detailed explanation.
//!
//! Deviations from the C original, imposed by the crate-wide API:
//!
//! - Hash values are `u32`.  (The C original computes a 64-bit
//!   Fowler-Noll-Vo hash; the computation is unchanged and the result
//!   is truncated to 32 bits at return.)
//! - `HashActionFunc` is a plain function type, which is
//!   non-nullable: where C passes a null `destructor` (e.g.
//!   `hash_clear (h, NULL)`), pass [`hash_no_action`] instead.
//! - The iterator stores the current bucket as an index into
//!   `hash.buckets` rather than a `struct list *`.

use crate::lib::kernel::list::{
    List, ListElem, list_begin, list_empty, list_end, list_head, list_init, list_next,
    list_pop_front, list_push_front, list_remove,
};
use crate::threads::malloc::{free, malloc};

/* Hash element. */
#[repr(C)]
pub struct HashElem {
    pub list_elem: ListElem,
}

/* Converts pointer to hash element ELEM into a pointer to the
structure that ELEM is embedded inside.  Pass the byte offset of
the `HashElem` member within the outer structure, i.e.
`core::mem::offset_of!(STRUCT, MEMBER)`.  This is C's
hash_entry(HASH_ELEM, STRUCT, MEMBER) container_of. */
pub unsafe fn hash_entry<T>(e: *mut HashElem, offset: usize) -> *mut T {
    unsafe { (e as *mut u8).sub(offset) as *mut T }
}

/* Computes and returns the hash value for hash element E, given
auxiliary data AUX. */
pub type HashHashFunc = unsafe fn(e: *const HashElem, aux: *mut ()) -> u32;

/* Compares the value of two hash elements A and B, given
auxiliary data AUX.  Returns true if A is less than B, or
false if A is greater than or equal to B. */
pub type HashLessFunc = unsafe fn(a: *const HashElem, b: *const HashElem, aux: *mut ()) -> bool;

/* Performs some operation on hash element E, given auxiliary data
AUX. */
pub type HashActionFunc = unsafe fn(e: *mut HashElem, aux: *mut ());

/* Hash table. */
#[repr(C)]
pub struct Hash {
    pub elem_cnt: usize,    /* Number of elements in table. */
    pub bucket_cnt: usize,  /* Number of buckets, a power of 2. */
    pub buckets: *mut List, /* Array of `bucket_cnt' lists. */
    pub hash: HashHashFunc, /* Hash function. */
    pub less: HashLessFunc, /* Comparison function. */
    pub aux: *mut (),       /* Auxiliary data for `hash' and `less'. */
}

/* A hash table iterator. */
#[repr(C)]
pub struct HashIterator {
    pub hash: *mut Hash,     /* The hash table. */
    pub bucket: usize,       /* Current bucket (index into hash.buckets). */
    pub elem: *mut HashElem, /* Current hash element in current bucket. */
}

/* Do-nothing stand-in for C's NULL `hash_action_func *'.  C's
hash_clear() and hash_destroy() accept a null action; Rust fn
pointers are non-nullable, so pass this function where C passed
NULL. */
pub unsafe fn hash_no_action(_e: *mut HashElem, _aux: *mut ()) {}

/* Converts a `ListElem' that is the first member of a `HashElem'
back into the hash element.  (C's list_elem_to_hash_elem.) */
unsafe fn list_elem_to_hash_elem(le: *mut ListElem) -> *mut HashElem {
    le as *mut HashElem
}

/* Initializes hash table H to compute hash values using HASH and
compare hash elements using LESS, given auxiliary data AUX.
Returns true if successful, false on allocation failure. */
pub unsafe fn hash_init(
    h: *mut Hash,
    hash: HashHashFunc,
    less: HashLessFunc,
    aux: *mut (),
) -> bool {
    unsafe {
        (*h).elem_cnt = 0;
        (*h).bucket_cnt = 4;
        (*h).buckets = malloc(core::mem::size_of::<List>() * (*h).bucket_cnt) as *mut List;
        (*h).hash = hash;
        (*h).less = less;
        (*h).aux = aux;

        if !(*h).buckets.is_null() {
            /* C: hash_clear (h, NULL) — with a null destructor this
            amounts to initializing every bucket as an empty
            list. */
            for i in 0..(*h).bucket_cnt {
                list_init((*h).buckets.add(i));
            }
            true
        } else {
            false
        }
    }
}

/* Removes all the elements from H.

ACTION is called for each element in the hash.  ACTION may, if
appropriate, deallocate the memory used by the hash element.
However, modifying hash table H while hash_clear() is running,
using any of the functions hash_clear(), hash_destroy(),
hash_insert(), hash_replace(), or hash_delete(), yields
undefined behavior, whether done in ACTION or elsewhere.

(C permits a null ACTION, in which case it is not called; pass
[`hash_no_action`] for that behavior.) */
pub unsafe fn hash_clear(h: *mut Hash, action: HashActionFunc) {
    unsafe {
        for i in 0..(*h).bucket_cnt {
            let bucket = (*h).buckets.add(i);

            while !list_empty(bucket) {
                let list_elem = list_pop_front(bucket);
                action(list_elem_to_hash_elem(list_elem), (*h).aux);
            }

            list_init(bucket);
        }

        (*h).elem_cnt = 0;
    }
}

/* Destroys hash table H.

ACTION is first called for each element in the hash.  ACTION
may, if appropriate, deallocate the memory used by the hash
element.  However, modifying hash table H while hash_clear() is
running, using any of the functions hash_clear(),
hash_destroy(), hash_insert(), hash_replace(), or
hash_delete(), yields undefined behavior, whether done in
ACTION or elsewhere.

(C permits a null ACTION, in which case the table is not
cleared at all; pass [`hash_no_action`] to iterate without
acting.) */
pub unsafe fn hash_destroy(h: *mut Hash, action: HashActionFunc) {
    unsafe {
        hash_clear(h, action);
        free((*h).buckets as *mut u8);
    }
}

/* Inserts NEW into hash table H and returns a null pointer, if
no equal element is already in the table.
If an equal element is already in the table, returns it
without inserting NEW. */
pub unsafe fn hash_insert(h: *mut Hash, e: *mut HashElem) -> *mut HashElem {
    unsafe {
        let bucket = find_bucket(h, e);
        let old = find_elem(h, bucket, e);

        if old.is_null() {
            insert_elem(h, bucket, e);
        }

        rehash(h);

        old
    }
}

/* Inserts NEW into hash table H, replacing any equal element
already in the table, which is returned. */
pub unsafe fn hash_replace(h: *mut Hash, e: *mut HashElem) -> *mut HashElem {
    unsafe {
        let bucket = find_bucket(h, e);
        let old = find_elem(h, bucket, e);

        if !old.is_null() {
            remove_elem(h, old);
        }
        insert_elem(h, bucket, e);

        rehash(h);

        old
    }
}

/* Finds and returns an element equal to E in hash table H, or a
null pointer if no equal element exists in the table. */
pub unsafe fn hash_find(h: *mut Hash, e: *const HashElem) -> *mut HashElem {
    unsafe { find_elem(h, find_bucket(h, e), e) }
}

/* Finds, removes, and returns an element equal to E in hash
table H.  Returns a null pointer if no equal element existed
in the table.

If the elements of the hash table are dynamically allocated,
or own resources that are, then it is the caller's
responsibility to deallocate them. */
pub unsafe fn hash_delete(h: *mut Hash, e: *mut HashElem) -> *mut HashElem {
    unsafe {
        let found = find_elem(h, find_bucket(h, e), e);
        if !found.is_null() {
            remove_elem(h, found);
            rehash(h);
        }
        found
    }
}

/* Calls ACTION for each element in hash table H in arbitrary
order.
Modifying hash table H while hash_apply() is running, using
any of the functions hash_clear(), hash_destroy(),
hash_insert(), hash_replace(), or hash_delete(), yields
undefined behavior, whether done from ACTION or elsewhere. */
pub unsafe fn hash_apply(h: *mut Hash, action: HashActionFunc) {
    unsafe {
        for i in 0..(*h).bucket_cnt {
            let bucket = (*h).buckets.add(i);

            let mut elem = list_begin(bucket);
            while elem != list_end(bucket) {
                let next = list_next(elem);
                action(list_elem_to_hash_elem(elem), (*h).aux);
                elem = next;
            }
        }
    }
}

/* Initializes I for iterating hash table H.  Returns a null
pointer (C's hash_first() returns void; the return value only
satisfies the crate-wide signature and should be ignored).

Iteration idiom:

let mut i = HashIterator { hash: core::ptr::null_mut(), bucket: 0,
                           elem: core::ptr::null_mut() };
unsafe {
    hash_first(&mut i, h);
    while !hash_next(&mut i).is_null() {
        let f = hash_entry::<Foo>(hash_cur(&i), core::mem::offset_of!(Foo, elem));
        // ...do something with f...
    }
}

Modifying hash table H during iteration, using any of the
functions hash_clear(), hash_destroy(), hash_insert(),
hash_replace(), or hash_delete(), invalidates all
iterators. */
pub unsafe fn hash_first(i: *mut HashIterator, h: *mut Hash) -> *mut HashElem {
    crate::ASSERT!(!i.is_null());
    crate::ASSERT!(!h.is_null());

    unsafe {
        (*i).hash = h;
        (*i).bucket = 0; /* C: i->bucket = i->hash->buckets. */
        (*i).elem = list_elem_to_hash_elem(list_head((*h).buckets));

        core::ptr::null_mut()
    }
}

/* Advances I to the next element in the hash table and returns
it.  Returns a null pointer if no elements are left.  Elements
are returned in arbitrary order.

Modifying a hash table H during iteration, using any of the
functions hash_clear(), hash_destroy(), hash_insert(),
hash_replace(), or hash_delete(), invalidates all
iterators. */
pub unsafe fn hash_next(i: *mut HashIterator) -> *mut HashElem {
    unsafe {
        (*i).elem =
            list_elem_to_hash_elem(list_next(core::ptr::addr_of_mut!((*(*i).elem).list_elem)));
        while (*i).elem as *mut ListElem == list_end((*(*i).hash).buckets.add((*i).bucket)) {
            (*i).bucket += 1;
            if (*i).bucket >= (*(*i).hash).bucket_cnt {
                (*i).elem = core::ptr::null_mut();
                break;
            }
            (*i).elem = list_elem_to_hash_elem(list_begin((*(*i).hash).buckets.add((*i).bucket)));
        }

        (*i).elem
    }
}

/* Returns the current element in the hash table iteration, or a
null pointer at the end of the table.  Undefined behavior
after calling hash_first() but before hash_next(). */
pub unsafe fn hash_cur(i: *const HashIterator) -> *mut HashElem {
    unsafe { (*i).elem }
}

/* Returns the number of elements in H. */
pub unsafe fn hash_size(h: *const Hash) -> usize {
    unsafe { (*h).elem_cnt }
}

/* Returns true if H contains no elements, false otherwise. */
pub unsafe fn hash_empty(h: *const Hash) -> bool {
    unsafe { (*h).elem_cnt == 0 }
}

/* Fowler-Noll-Vo hash constants, for 64-bit word sizes. */
const FNV_64_PRIME: u64 = 0x0000_0100_0000_01B3;
const FNV_64_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/* Returns a hash of the SIZE bytes in BUF. */
pub unsafe fn hash_bytes(b: *const u8, size: usize) -> u32 {
    /* Fowler-Noll-Vo 32-bit hash, for bytes.  (The C original
    computes a 64-bit FNV hash; the crate-wide hash function type
    is u32, so the 64-bit result is truncated at return.) */
    crate::ASSERT!(!b.is_null());

    unsafe {
        let mut buf = b;
        let mut hash = FNV_64_BASIS;
        let mut n = size;
        while n > 0 {
            n -= 1;
            hash = hash.wrapping_mul(FNV_64_PRIME) ^ (*buf as u64);
            buf = buf.add(1);
        }

        hash as u32
    }
}

/* Returns a hash of string S (a NUL-terminated C string). */
pub unsafe fn hash_string(s: *const u8) -> u32 {
    crate::ASSERT!(!s.is_null());

    unsafe {
        let mut p = s;
        let mut hash = FNV_64_BASIS;
        while *p != b'\0' {
            hash = hash.wrapping_mul(FNV_64_PRIME) ^ (*p as u64);
            p = p.add(1);
        }

        hash as u32
    }
}

/* Returns a hash of integer I. */
pub unsafe fn hash_int(i: i32) -> u32 {
    unsafe {
        hash_bytes(
            core::ptr::addr_of!(i) as *const u8,
            core::mem::size_of::<i32>(),
        )
    }
}

/* Returns the bucket in H that E belongs in. */
unsafe fn find_bucket(h: *mut Hash, e: *const HashElem) -> *mut List {
    unsafe {
        let bucket_idx = ((*h).hash)(e, (*h).aux) as usize & ((*h).bucket_cnt - 1);
        (*h).buckets.add(bucket_idx)
    }
}

/* Searches BUCKET in H for a hash element equal to E.  Returns
it if found or a null pointer otherwise. */
unsafe fn find_elem(h: *mut Hash, bucket: *mut List, e: *const HashElem) -> *mut HashElem {
    unsafe {
        let mut i = list_begin(bucket);
        while i != list_end(bucket) {
            let hi = list_elem_to_hash_elem(i);
            if !((*h).less)(hi, e, (*h).aux) && !((*h).less)(e, hi, (*h).aux) {
                return hi;
            }
            i = list_next(i);
        }
        core::ptr::null_mut()
    }
}

/* Returns X with its lowest-order bit set to 1 turned off. */
fn turn_off_least_1bit(x: usize) -> usize {
    x & x.wrapping_sub(1)
}

/* Returns true if X is a power of 2, otherwise false. */
fn is_power_of_2(x: usize) -> bool {
    x != 0 && turn_off_least_1bit(x) == 0
}

/* Element per bucket ratios. */
const MIN_ELEMS_PER_BUCKET: usize = 1; /* Elems/bucket < 1: reduce # of buckets. */
const BEST_ELEMS_PER_BUCKET: usize = 2; /* Ideal elems/bucket. */
const MAX_ELEMS_PER_BUCKET: usize = 4; /* Elems/bucket > 4: increase # of buckets. */

/* Changes the number of buckets in hash table H to match the
ideal.  This function can fail because of an out-of-memory
condition, but that'll just make hash accesses less efficient;
we can still continue. */
unsafe fn rehash(h: *mut Hash) {
    crate::ASSERT!(!h.is_null());

    unsafe {
        /* Save old bucket info for later use. */
        let old_buckets = (*h).buckets;
        let old_bucket_cnt = (*h).bucket_cnt;

        /* Calculate the number of buckets to use now.
        We want one bucket for about every BEST_ELEMS_PER_BUCKET.
        We must have at least four buckets, and the number of
        buckets must be a power of 2. */
        let mut new_bucket_cnt = (*h).elem_cnt / BEST_ELEMS_PER_BUCKET;
        if new_bucket_cnt < 4 {
            new_bucket_cnt = 4;
        }
        while !is_power_of_2(new_bucket_cnt) {
            new_bucket_cnt = turn_off_least_1bit(new_bucket_cnt);
        }

        /* Don't do anything if the bucket count wouldn't change. */
        if new_bucket_cnt == old_bucket_cnt {
            return;
        }

        /* Allocate new buckets and initialize them as empty. */
        let new_buckets = malloc(core::mem::size_of::<List>() * new_bucket_cnt) as *mut List;
        if new_buckets.is_null() {
            /* Allocation failed.  This means that use of the hash table will
            be less efficient.  However, it is still usable, so
            there's no reason for it to be an error. */
            return;
        }
        for i in 0..new_bucket_cnt {
            list_init(new_buckets.add(i));
        }

        /* Install new bucket info. */
        (*h).buckets = new_buckets;
        (*h).bucket_cnt = new_bucket_cnt;

        /* Move each old element into the appropriate new bucket. */
        for i in 0..old_bucket_cnt {
            let old_bucket = old_buckets.add(i);

            let mut elem = list_begin(old_bucket);
            while elem != list_end(old_bucket) {
                let new_bucket = find_bucket(h, list_elem_to_hash_elem(elem));
                let next = list_next(elem);
                list_remove(elem);
                list_push_front(new_bucket, elem);
                elem = next;
            }
        }

        free(old_buckets as *mut u8);
    }
}

/* Inserts E into BUCKET (in hash table H). */
unsafe fn insert_elem(h: *mut Hash, bucket: *mut List, e: *mut HashElem) {
    unsafe {
        (*h).elem_cnt += 1;
        list_push_front(bucket, core::ptr::addr_of_mut!((*e).list_elem));
    }
}

/* Removes E from hash table H. */
unsafe fn remove_elem(h: *mut Hash, e: *mut HashElem) {
    unsafe {
        (*h).elem_cnt -= 1;
        list_remove(core::ptr::addr_of_mut!((*e).list_elem));
    }
}
