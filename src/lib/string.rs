//! Port of `lib/string.c` — byte-string helpers over raw pointers.
//!
//! These keep the C semantics (NUL-terminated strings, pointer in/out)
//! because kernel data structures store C strings.  They are *not*
//! `#[no_mangle]`: Rust's compiler-provided `memcpy`-alikes in
//! compiler_builtins are unaffected by these same-named Rust functions.

/* Returns a pointer to the first occurrence of C in the SIZE bytes
starting at BUFFER, or a null pointer if C does not occur.  */
pub unsafe fn memchr(buffer: *const u8, c: u8, size: usize) -> *mut u8 {
    unsafe {
        let mut p = buffer;
        let end = buffer.wrapping_add(size);
        while p < end {
            if *p == c {
                return p as *mut u8;
            }
            p = p.add(1);
        }
        core::ptr::null_mut()
    }
}

/* Copies SIZE bytes from SRC to DST, which must not overlap.
Returns DST. */
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(dst_: *mut u8, src_: *const u8, size: usize) -> *mut u8 {
    unsafe {
        // Rust codegen may lower struct copies to compiler_builtins'
        // memcpy, not this one, so a straightforward byte loop is safe.
        let mut dst = dst_;
        let mut src = src_;
        let mut n = size;
        while n > 0 {
            *dst = *src;
            dst = dst.add(1);
            src = src.add(1);
            n -= 1;
        }
        dst_
    }
}

/* Copies SIZE bytes from SRC to DST, which are allowed to
overlap.  Returns DST. */
pub unsafe fn memmove(dst_: *mut u8, src_: *const u8, size: usize) -> *mut u8 {
    unsafe {
        let mut dst = dst_;
        let mut src = src_;
        let mut n = size;
        if (dst as usize) < (src as usize) {
            while n > 0 {
                *dst = *src;
                dst = dst.add(1);
                src = src.add(1);
                n -= 1;
            }
        } else {
            dst = dst.add(n);
            src = src.add(n);
            while n > 0 {
                dst = dst.sub(1);
                src = src.sub(1);
                *dst = *src;
                n -= 1;
            }
        }
        dst_
    }
}

/* Compares A and B, treating each as an array of SIZE unsigned
bytes.  Returns a value less than, equal to, or greater than
zero based on the first differing bytes. */
/* Exported under the C symbol name: core's slice comparison (and any
compiler_builtins lowering) expects to link against `memcmp`. */
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(a_: *const u8, b_: *const u8, size: usize) -> i32 {
    unsafe {
        let mut a = a_;
        let mut b = b_;
        let mut n = size;
        while n > 0 {
            let (x, y) = (*a, *b);
            if x != y {
                return x as i32 - y as i32;
            }
            a = a.add(1);
            b = b.add(1);
            n -= 1;
        }
        0
    }
}

/* Sets the SIZE bytes starting at P to C.  Returns P. */
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memset(p_: *mut u8, c: u8, size: usize) -> *mut u8 {
    unsafe {
        let mut p = p_;
        let mut n = size;
        while n > 0 {
            *p = c;
            p = p.add(1);
            n -= 1;
        }
        p_
    }
}

/* Returns the length of STRING. */
pub unsafe fn strlen(string: *const u8) -> usize {
    unsafe {
        let mut p = string;
        while *p != 0 {
            p = p.add(1);
        }
        p.offset_from(string) as usize
    }
}

/* Returns the length of STRING, but scan at most MAXLEN bytes.
If no NUL terminator is found within that many characters,
returns MAXLEN. */
pub unsafe fn strnlen(string: *const u8, maxlen: usize) -> usize {
    unsafe {
        /* Index-based like C: pointer arithmetic would wrap when
        maxlen is usize::MAX (the no-precision %s case). */
        let mut n = 0;
        while n < maxlen && *string.add(n) != 0 {
            n += 1;
        }
        n
    }
}

/* Compares two NUL-terminated strings. */
pub unsafe fn strcmp(a_: *const u8, b_: *const u8) -> i32 {
    unsafe {
        let mut a = a_;
        let mut b = b_;
        loop {
            let (x, y) = (*a, *b);
            if x != y {
                return x as i32 - y as i32;
            }
            if x == 0 {
                return 0;
            }
            a = a.add(1);
            b = b.add(1);
        }
    }
}

/* Finds the first occurrence of C in STRING.  Returns a pointer
to the first occurrence, or a null pointer if C does not occur
in STRING. */
pub unsafe fn strchr(string: *const u8, c_: u8) -> *mut u8 {
    unsafe {
        let mut p = string;
        loop {
            if *p == c_ {
                return p as *mut u8;
            }
            if *p == 0 {
                return core::ptr::null_mut();
            }
            p = p.add(1);
        }
    }
}

/* Finds the last occurrence of C in STRING.  Returns a pointer
to the last occurrence, or a null pointer if C does not occur
in STRING. */
pub unsafe fn strrchr(string: *const u8, c_: u8) -> *mut u8 {
    unsafe {
        let mut last: *mut u8 = core::ptr::null_mut();
        let mut p = string;
        loop {
            if *p == c_ {
                last = p as *mut u8;
            }
            if *p == 0 {
                return last;
            }
            p = p.add(1);
        }
    }
}

/* Returns the length of the initial substring of S that consists
entirely of characters in ACCEPT. */
pub unsafe fn strspn(s: *const u8, accept: *const u8) -> usize {
    unsafe {
        let mut p = s;
        loop {
            if *p == 0 || strchr(accept, *p).is_null() {
                return p.offset_from(s) as usize;
            }
            p = p.add(1);
        }
    }
}

/* Returns the length of the initial substring of S that consists
entirely of characters not in REJECT. */
pub unsafe fn strcspn(s: *const u8, reject: *const u8) -> usize {
    unsafe {
        let mut p = s;
        loop {
            if *p == 0 || !strchr(reject, *p).is_null() {
                return p.offset_from(s) as usize;
            }
            p = p.add(1);
        }
    }
}

/* Finds the first character in S that is also in ACCEPT.
Returns a pointer to the first such occurrence, or a null
pointer if S has no characters in ACCEPT. */
pub unsafe fn strpbrk(s: *const u8, accept: *const u8) -> *mut u8 {
    unsafe {
        let mut p = s;
        loop {
            if *p == 0 {
                return core::ptr::null_mut();
            }
            if !strchr(accept, *p).is_null() {
                return p as *mut u8;
            }
            p = p.add(1);
        }
    }
}

/* Finds the first occurrence of NEEDLE in HAYSTACK, which must be
NUL-terminated.  Returns a pointer to the beginning of the
first occurrence, or a null pointer if there is none. */
pub unsafe fn strstr(haystack: *const u8, needle: *const u8) -> *mut u8 {
    unsafe {
        // Check for the empty needle (C guarantees it matches).
        if *needle == 0 {
            return haystack as *mut u8;
        }
        let mut p = haystack;
        loop {
            if *p == 0 {
                return core::ptr::null_mut();
            }
            if memcmp(p, needle, strlen(needle)) == 0 {
                return p as *mut u8;
            }
            p = p.add(1);
        }
    }
}

/* Breaks a string into tokens separated by DELIMITERS.  The
first call uses S; later calls with a null S continue from the
end of the previous token.  SAVE_PTR tracks the position. */
pub unsafe fn strtok_r(s_: *mut u8, delimiters: *const u8, save_ptr: *mut *mut u8) -> *mut u8 {
    unsafe {
        let mut s = if !s_.is_null() { s_ } else { *save_ptr };
        if s.is_null() {
            return core::ptr::null_mut();
        }

        s = s.add(strspn(s, delimiters));
        if *s == 0 {
            *save_ptr = s;
            return core::ptr::null_mut();
        }

        let token = s;
        s = strpbrk(token, delimiters);
        if s.is_null() {
            *save_ptr = token.add(strlen(token));
        } else {
            *s = 0;
            *save_ptr = s.add(1);
        }
        token
    }
}

/* Copies string SRC to DST, truncating if necessary.  Assumes
DST has room for at least SIZE characters (including the NUL
terminator).  Returns the length of the string the caller
intended to write, i.e. strlen(SRC). */
pub unsafe fn strlcpy(dst: *mut u8, src: *const u8, size: usize) -> usize {
    unsafe {
        let src_len = strlen(src);
        if size > 0 {
            let mut n = size - 1;
            if n > src_len {
                n = src_len;
            }
            memcpy(dst, src, n);
            *dst.add(n) = 0;
        }
        src_len
    }
}

/* Appends string SRC to DST, truncating if necessary.  Assumes
DST has room for at least SIZE characters (including the NUL
terminator).  Returns the length of the string the caller
intended to write, i.e. strlen(DST) + strlen(SRC). */
pub unsafe fn strlcat(dst: *mut u8, src: *const u8, size: usize) -> usize {
    unsafe {
        let dst_len = strnlen(dst, size);
        if dst_len < size {
            let src_len = strlcpy(dst.add(dst_len), src, size - dst_len);
            dst_len + src_len
        } else {
            dst_len + strlen(src)
        }
    }
}
