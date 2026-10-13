//! Port of `lib/random.c` — RC4-based pseudo-random number generator.

/* RC4 state. */
static mut S: [u8; 256] = [0; 256]; /* S[]. */
static mut S_I: u8 = 0; /* i. */
static mut S_J: u8 = 0; /* j. */

/* Already initialized? */
static mut INITED: bool = false;

/* Swaps the bytes pointed to by A and B. */
unsafe fn swap_byte(a: *mut u8, b: *mut u8) {
    unsafe {
        let t = *a;
        *a = *b;
        *b = t;
    }
}

/// Initializes or reinitializes the PRNG with the given SEED.
pub fn random_init(seed: u32) {
    unsafe {
        let seedp = seed.to_ne_bytes();

        for (i, s) in S.iter_mut().enumerate() {
            *s = i as u8;
        }
        let mut j: u8 = 0;
        for i in 0..256 {
            j = j
                .wrapping_add(S[i])
                .wrapping_add(seedp[i % core::mem::size_of::<u32>()]);
            swap_byte(&raw mut S[i], &raw mut S[j as usize]);
        }

        S_I = 0;
        S_J = 0;
        INITED = true;
    }
}

/// Writes SIZE random bytes into BUF.
pub unsafe fn random_bytes(buf_: *mut u8, mut size: usize) {
    unsafe {
        if !INITED {
            random_init(0);
        }

        let mut buf = buf_;
        while size > 0 {
            size -= 1;

            S_I = S_I.wrapping_add(1);
            S_J = S_J.wrapping_add(S[S_I as usize]);
            swap_byte(&raw mut S[S_I as usize], &raw mut S[S_J as usize]);

            let s_k = S[S_I as usize].wrapping_add(S[S_J as usize]);
            *buf = S[s_k as usize];
            buf = buf.add(1);
        }
    }
}

/// Returns a pseudo-random unsigned long.
pub fn random_ulong() -> u64 {
    let mut ul: u64 = 0;
    unsafe {
        random_bytes(&raw mut ul as *mut u8, core::mem::size_of::<u64>());
    }
    ul
}
