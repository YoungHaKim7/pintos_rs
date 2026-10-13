//! Port of `lib/kernel/list.c` + `include/lib/kernel/list.h` — the
//! intrusive doubly linked list used throughout the kernel.
//!
//! This implementation of a doubly linked list does not require use
//! of dynamically allocated memory.  Instead, each structure that is
//! a potential list element must embed a [`ListElem`] member.  All of
//! the list functions operate on these `ListElem`s.  [`list_entry`]
//! allows conversion from a `ListElem` back to the structure object
//! that contains it.
//!
//! For example, suppose there is a need for a list of `struct foo`.
//! `foo` should contain a `ListElem` member, like so:
//!
//! ```ignore
//! #[repr(C)]
//! struct Foo {
//!     elem: ListElem,
//!     bar: i32,
//!     // ...other members...
//! }
//! ```
//!
//! Then a list of `Foo` can be declared and initialized like so:
//!
//! ```ignore
//! let mut foo_list: List = unsafe { core::mem::zeroed() };
//! unsafe { list_init(&mut foo_list) };
//! ```
//!
//! Iteration is a typical situation where it is necessary to convert
//! from a `ListElem` back to its enclosing structure.  Here's an
//! example using `foo_list`:
//!
//! ```ignore
//! let mut e = unsafe { list_begin(&foo_list) };
//! while e != unsafe { list_end(&foo_list) } {
//!     let f = unsafe { list_entry::<Foo>(e, core::mem::offset_of!(Foo, elem)) };
//!     // ...do something with f...
//!     e = unsafe { list_next(e) };
//! }
//! ```
//!
//! Glossary of list terms:
//!
//! - "front": The first element in a list.  Undefined in an empty
//!   list.  Returned by `list_front`.
//!
//! - "back": The last element in a list.  Undefined in an empty
//!   list.  Returned by `list_back`.
//!
//! - "tail": The element figuratively just after the last element of
//!   a list.  Well defined even in an empty list.  Returned by
//!   `list_end`.  Used as the end sentinel for an iteration from
//!   front to back.
//!
//! - "beginning": In a non-empty list, the front.  In an empty list,
//!   the tail.  Returned by `list_begin`.  Used as the starting
//!   point for an iteration from front to back.
//!
//! - "head": The element figuratively just before the first element
//!   of a list.  Well defined even in an empty list.  Returned by
//!   `list_rend`.  Used as the end sentinel for an iteration from
//!   back to front.
//!
//! - "reverse beginning": In a non-empty list, the back.  In an
//!   empty list, the head.  Returned by `list_rbegin`.  Used as the
//!   starting point for an iteration from back to front.
//!
//! - "interior element": An element that is not the head or tail,
//!   that is, a real list element.  An empty list does not have any
//!   interior elements.

/* Our doubly linked lists have two header elements: the "head"
just before the first element and the "tail" just after the
last element.  The `prev' link of the front header is null, as
is the `next' link of the back header.  Their other two links
point toward each other via the interior elements of the list.

An empty list looks like this:

+------+     +------+
<---| head |<--->| tail |--->
+------+     +------+

A list with two elements in it looks like this:

+------+     +-------+     +-------+     +------+
<---| head |<--->|   1   |<--->|   2   |<--->| tail |<--->
+------+     +-------+     +-------+     +------+

The symmetry of this arrangement eliminates lots of special
cases in list processing.  For example, take a look at
list_remove(): it takes only two pointer assignments and no
conditionals.  That's a lot simpler than the code would be
without header elements.

(Because only one of the pointers in each header element is used,
we could in fact combine them into a single header element
without sacrificing this simplicity.  But using two separate
elements allows us to do a little bit of checking on some
operations, which can be valuable.) */

/* List element. */
#[repr(C)]
pub struct ListElem {
    pub prev: *mut ListElem, /* Previous list element. */
    pub next: *mut ListElem, /* Next list element. */
}

/* List. */
#[repr(C)]
pub struct List {
    pub head: ListElem, /* List head. */
    pub tail: ListElem, /* List tail. */
}

/* Compares the value of two list elements A and B, given auxiliary
data AUX.  Returns true if A is less than B, or false if A is
greater than or equal to B. */
pub type ListLessFunc = unsafe fn(a: *const ListElem, b: *const ListElem, aux: *mut ()) -> bool;

/* Converts pointer to list element ELEM into a pointer to the
structure that ELEM is embedded inside.  Pass the byte offset of
the `ListElem` member within the outer structure, i.e.
`core::mem::offset_of!(STRUCT, MEMBER)`.  This is C's
list_entry(LIST_ELEM, STRUCT, MEMBER) container_of. */
pub unsafe fn list_entry<T>(e: *mut ListElem, offset: usize) -> *mut T {
    unsafe { (e as *mut u8).sub(offset) as *mut T }
}

/* Returns true if ELEM is a head, false otherwise. */
unsafe fn is_head(elem: *const ListElem) -> bool {
    !elem.is_null() && unsafe { (*elem).prev.is_null() && !(*elem).next.is_null() }
}

/* Returns true if ELEM is an interior element, false otherwise. */
unsafe fn is_interior(elem: *const ListElem) -> bool {
    !elem.is_null() && unsafe { !(*elem).prev.is_null() && !(*elem).next.is_null() }
}

/* Returns true if ELEM is a tail, false otherwise. */
unsafe fn is_tail(elem: *const ListElem) -> bool {
    !elem.is_null() && unsafe { !(*elem).prev.is_null() && (*elem).next.is_null() }
}

/* Initializes LIST as an empty list. */
pub unsafe fn list_init(l: *mut List) {
    crate::ASSERT!(!l.is_null());
    unsafe {
        (*l).head.prev = core::ptr::null_mut();
        (*l).head.next = core::ptr::addr_of_mut!((*l).tail);
        (*l).tail.prev = core::ptr::addr_of_mut!((*l).head);
        (*l).tail.next = core::ptr::null_mut();
    }
}

/* Returns the beginning of LIST. */
pub unsafe fn list_begin(l: *const List) -> *mut ListElem {
    crate::ASSERT!(!l.is_null());
    unsafe { (*l).head.next }
}

/* Returns the element after ELEM in its list.  If ELEM is the
last element in its list, returns the list tail.  Results are
undefined if ELEM is itself a list tail. */
pub unsafe fn list_next(e: *mut ListElem) -> *mut ListElem {
    crate::ASSERT!(unsafe { is_head(e) || is_interior(e) });
    unsafe { (*e).next }
}

/* Returns LIST's tail.

list_end() is often used in iterating through a list from
front to back.  See the big comment at the top of this file for
an example. */
pub unsafe fn list_end(l: *const List) -> *mut ListElem {
    crate::ASSERT!(!l.is_null());
    unsafe { core::ptr::addr_of!((*l).tail) as *mut ListElem }
}

/* Returns the LIST's reverse beginning, for iterating through
LIST in reverse order, from back to front. */
pub unsafe fn list_rbegin(l: *const List) -> *mut ListElem {
    crate::ASSERT!(!l.is_null());
    unsafe { (*l).tail.prev }
}

/* Returns the element before ELEM in its list.  If ELEM is the
first element in its list, returns the list head.  Results are
undefined if ELEM is itself a list head. */
pub unsafe fn list_prev(e: *mut ListElem) -> *mut ListElem {
    crate::ASSERT!(unsafe { is_interior(e) || is_tail(e) });
    unsafe { (*e).prev }
}

/* Returns LIST's head.

list_rend() is often used in iterating through a list in
reverse order, from back to front.  Here's typical usage,
following the example from the top of this file:

let mut e = unsafe { list_rbegin(&foo_list) };
while e != unsafe { list_rend(&foo_list) } {
    let f = unsafe { list_entry::<Foo>(e, core::mem::offset_of!(Foo, elem)) };
    // ...do something with f...
    e = unsafe { list_prev(e) };
} */
pub unsafe fn list_rend(l: *const List) -> *mut ListElem {
    crate::ASSERT!(!l.is_null());
    unsafe { core::ptr::addr_of!((*l).head) as *mut ListElem }
}

/* Returns LIST's head.

list_head() can be used for an alternate style of iterating
through a list, e.g.:

let mut e = unsafe { list_head(&list) };
loop {
    e = unsafe { list_next(e) };
    if e == unsafe { list_end(&list) } {
        break;
    }
    // ...
} */
pub unsafe fn list_head(l: *const List) -> *mut ListElem {
    crate::ASSERT!(!l.is_null());
    unsafe { core::ptr::addr_of!((*l).head) as *mut ListElem }
}

/* Returns LIST's tail. */
pub unsafe fn list_tail(l: *const List) -> *mut ListElem {
    crate::ASSERT!(!l.is_null());
    unsafe { core::ptr::addr_of!((*l).tail) as *mut ListElem }
}

/* Inserts ELEM just before BEFORE, which may be either an
interior element or a tail.  The latter case is equivalent to
list_push_back().  (C: list_insert(BEFORE, ELEM).) */
pub unsafe fn list_insert_before(before: *mut ListElem, e: *mut ListElem) {
    crate::ASSERT!(unsafe { is_interior(before) || is_tail(before) });
    crate::ASSERT!(!e.is_null());

    unsafe {
        (*e).prev = (*before).prev;
        (*e).next = before;
        (*(*before).prev).next = e;
        (*before).prev = e;
    }
}

/* Removes elements FIRST through LAST (exclusive) from their
current list, then inserts them just before BEFORE, which may be
either an interior element or a tail. */
pub unsafe fn list_splice(before: *mut ListElem, first: *mut ListElem, last: *mut ListElem) {
    crate::ASSERT!(unsafe { is_interior(before) || is_tail(before) });
    unsafe {
        if first == last {
            return;
        }
        let last = list_prev(last);

        crate::ASSERT!(is_interior(first));
        crate::ASSERT!(is_interior(last));

        /* Cleanly remove FIRST...LAST from its current list. */
        (*(*first).prev).next = (*last).next;
        (*(*last).next).prev = (*first).prev;

        /* Splice FIRST...LAST into new list. */
        (*first).prev = (*before).prev;
        (*last).next = before;
        (*(*before).prev).next = first;
        (*before).prev = last;
    }
}

/* Inserts ELEM at the beginning of LIST, so that it becomes the
front in LIST. */
pub unsafe fn list_push_front(l: *mut List, e: *mut ListElem) {
    unsafe {
        list_insert_before(list_begin(l), e);
    }
}

/* Inserts ELEM at the end of LIST, so that it becomes the
back in LIST. */
pub unsafe fn list_push_back(l: *mut List, e: *mut ListElem) {
    unsafe {
        list_insert_before(list_end(l), e);
    }
}

/* Removes ELEM from its list and returns the element that
followed it.  Undefined behavior if ELEM is not in a list.

It's not safe to treat ELEM as an element in a list after
removing it.  In particular, using list_next() or list_prev()
on ELEM after removal yields undefined behavior.  This means
that a naive loop to remove the elements in a list will fail:

** DON'T DO THIS **
let mut e = unsafe { list_begin(&list) };
while e != unsafe { list_end(&list) } {
    // ...do something with e...
    unsafe { list_remove(e) };
    e = unsafe { list_next(e) };
}
** DON'T DO THIS **

Here is one correct way to iterate and remove elements from a
list:

let mut e = unsafe { list_begin(&list) };
while e != unsafe { list_end(&list) } {
    // ...do something with e...
    e = unsafe { list_remove(e) };
}

If you need to free() elements of the list then you need to be
more conservative.  Here's an alternate strategy that works
even in that case:

while unsafe { !list_empty(&list) } {
    let e = unsafe { list_pop_front(&list) };
    // ...do something with e...
} */
pub unsafe fn list_remove(e: *mut ListElem) -> *mut ListElem {
    crate::ASSERT!(unsafe { is_interior(e) });
    unsafe {
        (*(*e).prev).next = (*e).next;
        (*(*e).next).prev = (*e).prev;
        (*e).next
    }
}

/* Removes the front element from LIST and returns it.
Undefined behavior if LIST is empty before removal. */
pub unsafe fn list_pop_front(l: *mut List) -> *mut ListElem {
    unsafe {
        let front = list_front(l);
        list_remove(front);
        front
    }
}

/* Removes the back element from LIST and returns it.
Undefined behavior if LIST is empty before removal. */
pub unsafe fn list_pop_back(l: *mut List) -> *mut ListElem {
    unsafe {
        let back = list_back(l);
        list_remove(back);
        back
    }
}

/* Returns the front element in LIST.
Undefined behavior if LIST is empty. */
pub unsafe fn list_front(l: *const List) -> *mut ListElem {
    crate::ASSERT!(unsafe { !list_empty(l) });
    unsafe { (*l).head.next }
}

/* Returns the back element in LIST.
Undefined behavior if LIST is empty. */
pub unsafe fn list_back(l: *const List) -> *mut ListElem {
    crate::ASSERT!(unsafe { !list_empty(l) });
    unsafe { (*l).tail.prev }
}

/* Returns the number of elements in LIST.
Runs in O(n) in the number of elements. */
pub unsafe fn list_size(l: *const List) -> usize {
    unsafe {
        let mut cnt: usize = 0;

        let mut e = list_begin(l);
        while e != list_end(l) {
            cnt += 1;
            e = list_next(e);
        }
        cnt
    }
}

/* Returns true if LIST is empty, false otherwise. */
pub unsafe fn list_empty(l: *const List) -> bool {
    unsafe { list_begin(l) == list_end(l) }
}

/* Swaps the `ListElem` pointers that A and B point to. */
unsafe fn swap(a: *mut *mut ListElem, b: *mut *mut ListElem) {
    unsafe {
        let t = *a;
        *a = *b;
        *b = t;
    }
}

/* Reverses the order of LIST. */
pub unsafe fn list_reverse(l: *mut List) {
    unsafe {
        if !list_empty(l) {
            let mut e = list_begin(l);
            while e != list_end(l) {
                swap(
                    core::ptr::addr_of_mut!((*e).prev),
                    core::ptr::addr_of_mut!((*e).next),
                );
                e = (*e).prev;
            }
            swap(
                core::ptr::addr_of_mut!((*l).head.next),
                core::ptr::addr_of_mut!((*l).tail.prev),
            );
            swap(
                core::ptr::addr_of_mut!((*(*l).head.next).prev),
                core::ptr::addr_of_mut!((*(*l).tail.prev).next),
            );
        }
    }
}

/* Returns true only if the list elements A through B (exclusive)
are in order according to LESS given auxiliary data AUX. */
unsafe fn is_sorted(
    mut a: *mut ListElem,
    b: *mut ListElem,
    less: ListLessFunc,
    aux: *mut (),
) -> bool {
    unsafe {
        if a != b {
            loop {
                a = list_next(a);
                if a == b {
                    break;
                }
                if less(a, list_prev(a), aux) {
                    return false;
                }
            }
        }
        true
    }
}

/* Finds a run, starting at A and ending not after B, of list
elements that are in nondecreasing order according to LESS
given auxiliary data AUX.  Returns the (exclusive) end of the
run.
A through B (exclusive) must form a non-empty range. */
unsafe fn find_end_of_run(
    mut a: *mut ListElem,
    b: *mut ListElem,
    less: ListLessFunc,
    aux: *mut (),
) -> *mut ListElem {
    crate::ASSERT!(!a.is_null());
    crate::ASSERT!(!b.is_null());
    crate::ASSERT!(a != b);

    unsafe {
        loop {
            a = list_next(a);
            if a == b || less(a, list_prev(a), aux) {
                break;
            }
        }
        a
    }
}

/* Merges A0 through A1B0 (exclusive) with A1B0 through B1
(exclusive) to form a combined range also ending at B1
(exclusive).  Both input ranges must be nonempty and sorted in
nondecreasing order according to LESS given auxiliary data
AUX.  The output range will be sorted the same way. */
unsafe fn inplace_merge(
    mut a0: *mut ListElem,
    mut a1b0: *mut ListElem,
    b1: *mut ListElem,
    less: ListLessFunc,
    aux: *mut (),
) {
    crate::ASSERT!(!a0.is_null());
    crate::ASSERT!(!a1b0.is_null());
    crate::ASSERT!(!b1.is_null());
    crate::ASSERT!(unsafe { is_sorted(a0, a1b0, less, aux) });
    crate::ASSERT!(unsafe { is_sorted(a1b0, b1, less, aux) });

    unsafe {
        while a0 != a1b0 && a1b0 != b1 {
            if !less(a1b0, a0, aux) {
                a0 = list_next(a0);
            } else {
                a1b0 = list_next(a1b0);
                list_splice(a0, list_prev(a1b0), a1b0);
            }
        }
    }
}

/* Sorts LIST according to LESS given auxiliary data AUX, using a
natural iterative merge sort that runs in O(n lg n) time and
O(1) space in the number of elements in LIST. */
pub unsafe fn list_sort(l: *mut List, less: ListLessFunc, aux: *mut ()) {
    /* Number of runs output in current pass. */
    let mut output_run_cnt: usize;

    crate::ASSERT!(!l.is_null());

    unsafe {
        /* Pass over the list repeatedly, merging adjacent runs of
        nondecreasing elements, until only one run is left. */
        loop {
            output_run_cnt = 0;
            let mut a0 = list_begin(l); /* Start of first run. */
            while a0 != list_end(l) {
                /* Each iteration produces one output run. */
                output_run_cnt += 1;

                /* Locate two adjacent runs of nondecreasing elements
                A0...A1B0 and A1B0...B1. */
                let a1b0 = find_end_of_run(a0, list_end(l), less, aux); /* End of first run, start of second. */
                if a1b0 == list_end(l) {
                    break;
                }
                let b1 = find_end_of_run(a1b0, list_end(l), less, aux); /* End of second run. */

                /* Merge the runs. */
                inplace_merge(a0, a1b0, b1, less, aux);

                a0 = b1;
            }
            if output_run_cnt <= 1 {
                break;
            }
        }

        crate::ASSERT!(is_sorted(list_begin(l), list_end(l), less, aux));
    }
}

/* Inserts ELEM in the proper position in LIST, which must be
sorted according to LESS given auxiliary data AUX.
Runs in O(n) average case in the number of elements in LIST. */
pub unsafe fn list_insert_ordered(
    l: *mut List,
    e: *mut ListElem,
    less: ListLessFunc,
    aux: *mut (),
) {
    crate::ASSERT!(!l.is_null());
    crate::ASSERT!(!e.is_null());

    unsafe {
        let mut e2 = list_begin(l);
        while e2 != list_end(l) {
            if less(e, e2, aux) {
                break;
            }
            e2 = list_next(e2);
        }
        list_insert_before(e2, e);
    }
}

/* Iterates through LIST and removes all but the first in each
set of adjacent elements that are equal according to LESS
given auxiliary data AUX.  If DUPLICATES is non-null, then the
elements from LIST are appended to DUPLICATES. */
pub unsafe fn list_unique(l: *mut List, duplicates: *mut List, less: ListLessFunc, aux: *mut ()) {
    crate::ASSERT!(!l.is_null());

    unsafe {
        if list_empty(l) {
            return;
        }

        let mut elem = list_begin(l);
        loop {
            let next = list_next(elem);
            if next == list_end(l) {
                break;
            }
            if !less(elem, next, aux) && !less(next, elem, aux) {
                list_remove(next);
                if !duplicates.is_null() {
                    list_push_back(duplicates, next);
                }
            } else {
                elem = next;
            }
        }
    }
}

/* Returns the element in LIST with the largest value according
to LESS given auxiliary data AUX.  If there is more than one
maximum, returns the one that appears earlier in the list.  If
the list is empty, returns its tail. */
pub unsafe fn list_max(l: *const List, less: ListLessFunc, aux: *mut ()) -> *mut ListElem {
    unsafe {
        let mut max = list_begin(l);
        if max != list_end(l) {
            let mut e = list_next(max);
            while e != list_end(l) {
                if less(max, e, aux) {
                    max = e;
                }
                e = list_next(e);
            }
        }
        max
    }
}

/* Returns the element in LIST with the smallest value according
to LESS given auxiliary data AUX.  If there is more than one
minimum, returns the one that appears earlier in the list.  If
the list is empty, returns its tail. */
pub unsafe fn list_min(l: *const List, less: ListLessFunc, aux: *mut ()) -> *mut ListElem {
    unsafe {
        let mut min = list_begin(l);
        if min != list_end(l) {
            let mut e = list_next(min);
            while e != list_end(l) {
                if less(e, min, aux) {
                    min = e;
                }
                e = list_next(e);
            }
        }
        min
    }
}
