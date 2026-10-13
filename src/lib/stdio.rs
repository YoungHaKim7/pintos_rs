//! Port of `lib/stdio.c` — the printf formatting engine.
//!
//! C used `va_list`; Rust has no (stable) varargs, so arguments are
//! collected by the `printf!`/`format_args`-style macros into an
//! `&[Arg]` slice.  The format-string grammar, flags, width,
//! precision, and output algorithm are faithful to the C original so
//! console output matches byte-for-byte.

use crate::lib::string;

/// One variadic argument slot, in the spirit of C's varargs.
#[derive(Clone, Copy)]
pub enum Arg<'a> {
    /// Signed integer slot (`%d`, `%i`).
    I(i64),
    /// Unsigned integer slot (`%o`, `%u`, `%x`, `%X`).
    U(u64),
    /// `*` width/precision slot or `%c`'s character.
    C(u8),
    /// Rust string literal passed to `%s`.
    Str(&'a [u8]),
    /// NUL-terminated C string in kernel memory passed to `%s`.
    Raw(*const u8),
    /// Pointer passed to `%p`.
    P(usize),
}

impl From<i8> for Arg<'_> {
    fn from(v: i8) -> Self {
        Arg::I(v as i64)
    }
}
impl From<i16> for Arg<'_> {
    fn from(v: i16) -> Self {
        Arg::I(v as i64)
    }
}
impl From<i32> for Arg<'_> {
    fn from(v: i32) -> Self {
        Arg::I(v as i64)
    }
}
impl From<i64> for Arg<'_> {
    fn from(v: i64) -> Self {
        Arg::I(v)
    }
}
impl From<isize> for Arg<'_> {
    fn from(v: isize) -> Self {
        Arg::I(v as i64)
    }
}
impl From<u8> for Arg<'_> {
    fn from(v: u8) -> Self {
        Arg::U(v as u64)
    }
}
impl From<u16> for Arg<'_> {
    fn from(v: u16) -> Self {
        Arg::U(v as u64)
    }
}
impl From<u32> for Arg<'_> {
    fn from(v: u32) -> Self {
        Arg::U(v as u64)
    }
}
impl From<u64> for Arg<'_> {
    fn from(v: u64) -> Self {
        Arg::U(v)
    }
}
impl From<usize> for Arg<'_> {
    fn from(v: usize) -> Self {
        Arg::U(v as u64)
    }
}
impl From<char> for Arg<'_> {
    fn from(v: char) -> Self {
        Arg::C(v as u8)
    }
}
impl<'a> From<&'a str> for Arg<'a> {
    fn from(v: &'a str) -> Self {
        Arg::Str(v.as_bytes())
    }
}
impl<'a> From<&'a [u8]> for Arg<'a> {
    fn from(v: &'a [u8]) -> Self {
        Arg::Str(v)
    }
}

/// Character-output callback (C's `void (*output)(char, void *)`).
pub type OutputFn = fn(u8, *mut ());

/* A printf conversion. */
#[derive(Default, Clone, Copy)]
struct Conversion {
    /* Flags. */
    minus: bool, /* '-' */
    plus: bool,  /* '+' */
    space: bool, /* ' ' */
    pound: bool, /* '#' */
    zero: bool,  /* '0' */
    group: bool, /* '\'' */

    /* Minimum field width. */
    width: i32,

    /* Numeric precision; -1 indicates none specified. */
    precision: i32,

    /* Type of argument to format. */
    type_: IntType,
}

#[derive(Default, Clone, Copy, PartialEq)]
enum IntType {
    #[default]
    Int, /* (none) */
    Char,     /* hh */
    Short,    /* h */
    Intmax,   /* j */
    Long,     /* l */
    LongLong, /* ll */
    PtrdiffT, /* t */
    SizeT,    /* z */
}

struct IntegerBase {
    base: u64,
    digits: &'static [u8], /* NUL-terminated digit collection */
    x: u8,                 /* `x' character to use, for base 16 only; 0 otherwise */
    group: i32,            /* Number of digits to group with ' flag */
}

const BASE_D: IntegerBase = IntegerBase {
    base: 10,
    digits: b"0123456789\0",
    x: 0,
    group: 3,
};
const BASE_O: IntegerBase = IntegerBase {
    base: 8,
    digits: b"01234567\0",
    x: 0,
    group: 3,
};
const BASE_X: IntegerBase = IntegerBase {
    base: 16,
    digits: b"0123456789abcdef\0",
    x: b'x',
    group: 4,
};
const BASE_UPPER_X: IntegerBase = IntegerBase {
    base: 16,
    digits: b"0123456789ABCDEF\0",
    x: b'X',
    group: 4,
};

/// Iterator over the argument slots (stands in for `va_list`).
struct Args<'a> {
    slots: &'a [Arg<'a>],
    pos: usize,
}

impl<'a> Args<'a> {
    fn next(&mut self) -> Arg<'a> {
        let a = if self.pos < self.slots.len() {
            self.slots[self.pos]
        } else {
            Arg::U(0) // reading past the end: C would be UB; we return 0.
        };
        self.pos += 1;
        a
    }
}

/// Core formatter (port of `__vprintf`).
pub fn __vprintf(format: &str, slots: &[Arg], output: OutputFn, aux: *mut ()) {
    let mut args = Args { slots, pos: 0 };
    let fmt = format.as_bytes();
    let mut i = 0usize;

    while i < fmt.len() {
        let ch = fmt[i];
        i += 1;

        /* Literally copy non-conversions to output. */
        if ch != b'%' {
            output(ch, aux);
            continue;
        }

        /* %% => %. */
        if i < fmt.len() && fmt[i] == b'%' {
            output(b'%', aux);
            i += 1;
            continue;
        }

        /* Parse conversion specifiers. */
        let c = parse_conversion(fmt, &mut i, &mut args);

        /* Do conversion. */
        match fmt.get(i).copied().unwrap_or(0) {
            b'd' | b'i' => {
                /* Signed integer conversion. */
                let value = match args.next() {
                    Arg::I(v) => v,
                    Arg::U(v) => v as i64,
                    Arg::C(c) => c as i64,
                    Arg::P(v) => v as i64,
                    _ => 0,
                };
                let value = match c.type_ {
                    IntType::Char => value as i8 as i64,
                    IntType::Short => value as i16 as i64,
                    IntType::Int => value as i32 as i64,
                    _ => value,
                };
                format_integer(
                    value.unsigned_abs(),
                    true,
                    value < 0,
                    &BASE_D,
                    &c,
                    output,
                    aux,
                );
            }

            b'o' | b'u' | b'x' | b'X' => {
                /* Unsigned integer conversion. */
                let value = match args.next() {
                    Arg::U(v) => v,
                    Arg::I(v) => v as u64,
                    Arg::C(c) => c as u64,
                    Arg::P(v) => v as u64,
                    _ => 0,
                };
                let value = match c.type_ {
                    IntType::Char => value as u8 as u64,
                    IntType::Short => value as u16 as u64,
                    IntType::Int => value as u32 as u64,
                    _ => value,
                };
                let b = match fmt[i] {
                    b'o' => &BASE_O,
                    b'u' => &BASE_D,
                    b'x' => &BASE_X,
                    b'X' => &BASE_UPPER_X,
                    _ => unreachable!(),
                };
                format_integer(value, false, false, b, &c, output, aux);
            }

            b'c' => {
                /* Treat character as single-character string. */
                let ch = match args.next() {
                    Arg::C(c) => c,
                    Arg::I(v) => v as u8,
                    Arg::U(v) => v as u8,
                    Arg::P(v) => v as u8,
                    _ => b'?',
                };
                let ch_arr = [ch];
                format_string(&ch_arr, &c, output, aux);
            }

            b's' => {
                /* String conversion. */
                match args.next() {
                    Arg::Str(s) => {
                        let maxlen = if c.precision < 0 {
                            s.len()
                        } else {
                            s.len().min(c.precision as usize)
                        };
                        format_string(&s[..maxlen], &c, output, aux);
                    }
                    Arg::Raw(p) => {
                        let s = if p.is_null() {
                            b"(null)"
                        } else {
                            let maxlen = if c.precision < 0 {
                                usize::MAX
                            } else {
                                c.precision as usize
                            };
                            let len = unsafe { string::strnlen(p, maxlen) };
                            unsafe { core::slice::from_raw_parts(p, len) }
                        };
                        format_string(s, &c, output, aux);
                    }
                    _ => format_string(b"(null)", &c, output, aux),
                }
            }

            b'p' => {
                /* Pointer conversion: format pointers as %#x. */
                let p = match args.next() {
                    Arg::P(v) => v,
                    Arg::U(v) => v as usize,
                    Arg::I(v) => v as usize,
                    _ => 0,
                };
                let mut c2 = c;
                c2.pound = true;
                format_integer(p as u64, false, false, &BASE_X, &c2, output, aux);
            }

            b'f' | b'e' | b'E' | b'g' | b'G' | b'n' => {
                /* We don't support floating-point arithmetic,
                and %n can be part of a security hole. */
                let msg = core::format_args!("<<no %{} in kernel>>", fmt[i] as char);
                emit_str(&msg, output, aux);
            }

            other => {
                let msg = core::format_args!("<<no %{} conversion>>", other as char);
                emit_str(&msg, output, aux);
            }
        }
        i += 1; /* consume the conversion character */
    }
}

/// Formats a `core::format_args!` result through the output callback.
/// (Only used for internal error messages, never user conversions.)
fn emit_str(args: &core::fmt::Arguments, output: OutputFn, aux: *mut ()) {
    use core::fmt::Write;

    struct Sink<'a> {
        output: OutputFn,
        aux: *mut (),
        _marker: core::marker::PhantomData<&'a ()>,
    }

    impl Write for Sink<'_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            for &b in s.as_bytes() {
                (self.output)(b, self.aux);
            }
            Ok(())
        }
    }

    let _ = Sink {
        output,
        aux,
        _marker: core::marker::PhantomData,
    }
    .write_fmt(*args);
}

/// Parses conversion option characters starting at FMT[I], updating I to
/// index the conversion character itself (e.g. the `d` in `%d`).
/// Consumes `*` width/precision slots from ARGS.
fn parse_conversion(fmt: &[u8], i: &mut usize, args: &mut Args) -> Conversion {
    let mut c = Conversion {
        precision: -1,
        ..Default::default()
    };

    /* Parse flag characters. */
    loop {
        match fmt.get(*i).copied() {
            Some(b'-') => c.minus = true,
            Some(b'+') => c.plus = true,
            Some(b' ') => c.space = true,
            Some(b'#') => c.pound = true,
            Some(b'0') => c.zero = true,
            Some(b'\'') => c.group = true,
            _ => break,
        }
        *i += 1;
    }
    if c.minus {
        c.zero = false;
    }
    if c.plus {
        c.space = false;
    }

    /* Parse field width. */
    c.width = 0;
    if fmt.get(*i) == Some(&b'*') {
        *i += 1;
        c.width = match args.next() {
            Arg::I(v) => v as i32,
            Arg::U(v) => v as i32,
            Arg::C(ch) => ch as i32,
            Arg::P(v) => v as i32,
            _ => 0,
        };
    } else {
        while let Some(&d) = fmt.get(*i) {
            if !d.is_ascii_digit() {
                break;
            }
            c.width = c.width * 10 + (d - b'0') as i32;
            *i += 1;
        }
    }
    if c.width < 0 {
        c.width = -c.width;
        c.minus = true;
    }

    /* Parse precision. */
    c.precision = -1;
    if fmt.get(*i) == Some(&b'.') {
        *i += 1;
        if fmt.get(*i) == Some(&b'*') {
            *i += 1;
            c.precision = match args.next() {
                Arg::I(v) => v as i32,
                Arg::U(v) => v as i32,
                Arg::C(ch) => ch as i32,
                Arg::P(v) => v as i32,
                _ => 0,
            };
        } else {
            c.precision = 0;
            while let Some(&d) = fmt.get(*i) {
                if !d.is_ascii_digit() {
                    break;
                }
                c.precision = c.precision * 10 + (d - b'0') as i32;
                *i += 1;
            }
        }
        if c.precision < 0 {
            c.precision = -1;
        }
    }
    if c.precision >= 0 {
        c.zero = false;
    }

    /* Parse type. */
    c.type_ = IntType::Int;
    match fmt.get(*i).copied() {
        Some(b'h') => {
            *i += 1;
            if fmt.get(*i) == Some(&b'h') {
                *i += 1;
                c.type_ = IntType::Char;
            } else {
                c.type_ = IntType::Short;
            }
        }
        Some(b'j') => c.type_ = IntType::Intmax,
        Some(b'l') => {
            *i += 1;
            if fmt.get(*i) == Some(&b'l') {
                *i += 1;
                c.type_ = IntType::LongLong;
            } else {
                c.type_ = IntType::Long;
            }
        }
        Some(b't') => c.type_ = IntType::PtrdiffT,
        Some(b'z') => c.type_ = IntType::SizeT,
        _ => {}
    }

    c
}

/* Performs an integer conversion, writing output to OUTPUT with
auxiliary data AUX.  The integer converted has absolute value
VALUE.  If IS_SIGNED is true, does a signed conversion with
NEGATIVE indicating a negative value; otherwise does an
unsigned conversion and ignores NEGATIVE. */
fn format_integer(
    mut value: u64,
    is_signed: bool,
    negative: bool,
    b: &IntegerBase,
    c: &Conversion,
    output: OutputFn,
    aux: *mut (),
) {
    let mut buf = [0u8; 64];
    let mut cp = 0usize; /* current position in buf */

    /* Determine sign character, if any. */
    let mut sign = 0u8;
    if is_signed {
        if c.plus {
            sign = if negative { b'-' } else { b'+' };
        } else if c.space {
            sign = if negative { b'-' } else { b' ' };
        } else if negative {
            sign = b'-';
        }
    }

    /* Determine whether to include `0x' or `0X'. */
    let x = if c.pound && value != 0 { b.x } else { 0 };

    /* Accumulate digits into buffer, in reverse order. */
    let mut digit_cnt = 0;
    while value > 0 {
        if c.group && digit_cnt > 0 && digit_cnt % b.group == 0 {
            buf[cp] = b',';
            cp += 1;
        }
        buf[cp] = b.digits[(value % b.base) as usize];
        cp += 1;
        value /= b.base;
        digit_cnt += 1;
    }

    /* Append enough zeros to match precision. */
    let precision = if c.precision < 0 {
        1
    } else {
        c.precision as usize
    };
    while cp < precision && cp < buf.len() - 1 {
        buf[cp] = b'0';
        cp += 1;
    }
    if c.pound && b.base == 8 && (cp == 0 || buf[cp - 1] != b'0') {
        buf[cp] = b'0';
        cp += 1;
    }

    /* Calculate number of pad characters to fill field width. */
    let mut pad_cnt = c.width as i64 - cp as i64 - if x != 0 { 2 } else { 0 } - (sign != 0) as i64;
    if pad_cnt < 0 {
        pad_cnt = 0;
    }

    /* Do output. */
    if !c.minus && !c.zero {
        output_dup(b' ', pad_cnt as usize, output, aux);
    }
    if sign != 0 {
        output(sign, aux);
    }
    if x != 0 {
        output(b'0', aux);
        output(x, aux);
    }
    if c.zero {
        output_dup(b'0', pad_cnt as usize, output, aux);
    }
    while cp > 0 {
        cp -= 1;
        output(buf[cp], aux);
    }
    if c.minus {
        output_dup(b' ', pad_cnt as usize, output, aux);
    }
}

/* Writes CH to OUTPUT with auxiliary data AUX, CNT times. */
fn output_dup(ch: u8, cnt: usize, output: OutputFn, aux: *mut ()) {
    for _ in 0..cnt {
        output(ch, aux);
    }
}

/* Formats the bytes in STRING according to the conversion
specified in C. */
fn format_string(string: &[u8], c: &Conversion, output: OutputFn, aux: *mut ()) {
    let length = string.len();
    if c.width as usize > length && !c.minus {
        output_dup(b' ', c.width as usize - length, output, aux);
    }
    for &ch in string {
        output(ch, aux);
    }
    if c.width as usize > length && c.minus {
        output_dup(b' ', c.width as usize - length, output, aux);
    }
}

/* Auxiliary data for vsnprintf_helper(). */
struct VsnprintfAux {
    p: *mut u8,      /* Current output position. */
    length: i32,     /* Length of output string. */
    max_length: i32, /* Max length of output string. */
}

fn vsnprintf_helper(ch: u8, aux_: *mut ()) {
    let aux = unsafe { &mut *(aux_ as *mut VsnprintfAux) };
    let len = aux.length;
    aux.length += 1;
    if len < aux.max_length {
        unsafe {
            *aux.p = ch;
            aux.p = aux.p.add(1);
        }
    }
}

/// Like `__vprintf`, but output is stored into BUFFER, which must have
/// space for BUF_SIZE bytes.  Writes at most BUF_SIZE - 1 bytes followed
/// by a NUL terminator.  Returns the number of characters that would
/// have been written had there been enough room.
pub unsafe fn vsnprintf(buffer: *mut u8, buf_size: usize, format: &str, slots: &[Arg]) -> i32 {
    let mut aux = VsnprintfAux {
        p: buffer,
        length: 0,
        max_length: if buf_size > 0 { buf_size as i32 - 1 } else { 0 },
    };
    __vprintf(
        format,
        slots,
        vsnprintf_helper,
        &mut aux as *mut VsnprintfAux as *mut (),
    );
    if buf_size > 0 {
        unsafe { *aux.p = 0 };
    }
    aux.length
}

/* Dumps the SIZE bytes in BUF to the console as hex bytes
arranged 16 per line, starting at offset OFS.  If ASCII is true
then the corresponding ASCII characters are rendered too. */
pub unsafe fn hex_dump(mut ofs: usize, mut buf: *const u8, mut size: usize, ascii: bool) {
    unsafe {
        const PER_LINE: usize = 16;

        while size > 0 {
            /* Number of bytes on this line. */
            let start = ofs % PER_LINE;
            let mut end = PER_LINE;
            if end - start > size {
                end = start + size;
            }
            let n = end - start;

            /* Print line. */
            crate::printf!("%016llx  ", (ofs & !(PER_LINE - 1)) as u64);
            let mut i = start;
            while i < start {
                crate::printf!("   ");
                i += 1;
            }
            while i < end {
                crate::printf!(
                    "%02hhx%c",
                    *buf.add(i - start),
                    if i == PER_LINE / 2 - 1 { '-' } else { ' ' }
                );
                i += 1;
            }
            if ascii {
                while i < PER_LINE {
                    crate::printf!("   ");
                    i += 1;
                }
                crate::printf!("|");
                let mut i = start;
                while i < end {
                    let ch = *buf.add(i - start);
                    crate::printf!("%c", if (0x20..0x7f).contains(&ch) { ch } else { b'.' });
                    i += 1;
                }
                while i < PER_LINE {
                    crate::printf!(" ");
                    i += 1;
                }
                crate::printf!("|");
            }
            crate::printf!("\n");

            ofs += n;
            buf = buf.add(n);
            size -= n;
        }
    }
}
