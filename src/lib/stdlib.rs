//! Port of `lib/stdlib.c` (kernel-used subset).

/// Converts a string representation of a signed decimal integer in S
/// into an `i32`.
pub unsafe fn atoi(s_: *const u8) -> i32 {
    unsafe {
        let mut s = s_;

        /* Skip white space. */
        while matches!(*s, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
            s = s.add(1);
        }

        /* Parse sign. */
        let mut negative = false;
        if *s == b'+' {
            s = s.add(1);
        } else if *s == b'-' {
            negative = true;
            s = s.add(1);
        }

        /* Parse digits. */
        let mut value: i32 = 0;
        while (*s as char).is_ascii_digit() {
            value = value * 10 + (*s - b'0') as i32;
            s = s.add(1);
        }
        if negative { -value } else { value }
    }
}
