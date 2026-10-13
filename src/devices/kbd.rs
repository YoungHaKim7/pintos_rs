//! Port of `devices/kbd.c` — PS/2 keyboard driver.

use crate::intrinsic::inb;
use crate::lib::string::strlen;
use crate::printf;
use crate::threads::interrupt::{IntrFrame, intr_register_ext};

/* Keyboard data register port. */
const DATA_REG: u16 = 0x60;

/* Current state of shift keys.
True if depressed, false otherwise. */
static mut LEFT_SHIFT: bool = false; /* Left Shift key. */
static mut RIGHT_SHIFT: bool = false; /* Right Shift key. */
static mut LEFT_ALT: bool = false; /* Left Alt key. */
static mut RIGHT_ALT: bool = false; /* Right Alt key. */
static mut LEFT_CTRL: bool = false; /* Left Ctl key. */
static mut RIGHT_CTRL: bool = false; /* Right Ctl key. */

/* Status of Caps Lock.
True when on, false when off. */
static mut CAPS_LOCK: bool = false;

/* Number of keys pressed. */
static mut KEY_CNT: i64 = 0;

/* Initializes the keyboard. */
pub fn kbd_init() {
    unsafe { intr_register_ext(0x21, keyboard_interrupt, b"8042 Keyboard\0".as_ptr()) };
}

/* Prints keyboard statistics. */
pub fn kbd_print_stats() {
    printf!("Keyboard: %lld keys pressed\n", unsafe {
        *core::ptr::addr_of!(KEY_CNT)
    });
}

/* Maps a set of contiguous scancodes into characters. */
#[repr(C)]
struct Keymap {
    first_scancode: u8, /* First scancode. */
    chars: *const u8,   /* chars[0] has scancode first_scancode,
                        chars[1] has scancode first_scancode + 1,
                        and so on to the end of the string. */
}

/* Raw pointers are not `Sync`, but the keymaps are immutable
after initialization. */
unsafe impl Sync for Keymap {}

/* Keys that produce the same characters regardless of whether
the Shift keys are down.  Case of letters is an exception
that we handle elsewhere.  */
static INVARIANT_KEYMAP: [Keymap; 9] = [
    Keymap {
        first_scancode: 0x01,
        chars: b"\x1b\0".as_ptr(),
    },
    Keymap {
        first_scancode: 0x0e,
        chars: b"\x08\0".as_ptr(),
    },
    Keymap {
        first_scancode: 0x0f,
        chars: b"\tQWERTYUIOP\0".as_ptr(),
    },
    Keymap {
        first_scancode: 0x1c,
        chars: b"\r\0".as_ptr(),
    },
    Keymap {
        first_scancode: 0x1e,
        chars: b"ASDFGHJKL\0".as_ptr(),
    },
    Keymap {
        first_scancode: 0x2c,
        chars: b"ZXCVBNM\0".as_ptr(),
    },
    Keymap {
        first_scancode: 0x37,
        chars: b"*\0".as_ptr(),
    },
    Keymap {
        first_scancode: 0x39,
        chars: b" \0".as_ptr(),
    },
    Keymap {
        first_scancode: 0x00,
        chars: core::ptr::null(),
    },
];

/* Characters for keys pressed without Shift, for those keys
where it matters. */
static UNSHIFTED_KEYMAP: [Keymap; 6] = [
    Keymap {
        first_scancode: 0x02,
        chars: b"1234567890-=\0".as_ptr(),
    },
    Keymap {
        first_scancode: 0x1a,
        chars: b"[]\0".as_ptr(),
    },
    Keymap {
        first_scancode: 0x27,
        chars: b";'`\0".as_ptr(),
    },
    Keymap {
        first_scancode: 0x2b,
        chars: b"\\\0".as_ptr(),
    },
    Keymap {
        first_scancode: 0x33,
        chars: b",./\0".as_ptr(),
    },
    Keymap {
        first_scancode: 0x00,
        chars: core::ptr::null(),
    },
];

/* Characters for keys pressed with Shift, for those keys where
it matters. */
static SHIFTED_KEYMAP: [Keymap; 6] = [
    Keymap {
        first_scancode: 0x02,
        chars: b"!@#$%^&*()_+\0".as_ptr(),
    },
    Keymap {
        first_scancode: 0x1a,
        chars: b"{}\0".as_ptr(),
    },
    Keymap {
        first_scancode: 0x27,
        chars: b":\"~\0".as_ptr(),
    },
    Keymap {
        first_scancode: 0x2b,
        chars: b"|\0".as_ptr(),
    },
    Keymap {
        first_scancode: 0x33,
        chars: b"<>?\0".as_ptr(),
    },
    Keymap {
        first_scancode: 0x00,
        chars: core::ptr::null(),
    },
];

extern "C" fn keyboard_interrupt(_args: *mut IntrFrame) {
    unsafe {
        /* Status of shift keys. */
        let shift = *core::ptr::addr_of!(LEFT_SHIFT) || *core::ptr::addr_of!(RIGHT_SHIFT);
        let alt = *core::ptr::addr_of!(LEFT_ALT) || *core::ptr::addr_of!(RIGHT_ALT);
        let ctrl = *core::ptr::addr_of!(LEFT_CTRL) || *core::ptr::addr_of!(RIGHT_CTRL);

        /* Keyboard scancode. */
        let mut code: u32;

        /* False if key pressed, true if key released. */
        let release: bool;

        /* Character that corresponds to `code'. */
        let mut c: u8 = 0;

        /* Read scancode, including second byte if prefix code. */
        code = inb(DATA_REG) as u32;
        if code == 0xe0 {
            code = (code << 8) | inb(DATA_REG) as u32;
        }

        /* Bit 0x80 distinguishes key press from key release
        (even if there's a prefix). */
        release = (code & 0x80) != 0;
        code &= !0x80u32;

        /* Interpret key. */
        if code == 0x3a {
            /* Caps Lock. */
            if !release {
                let caps_lock = core::ptr::addr_of_mut!(CAPS_LOCK);
                *caps_lock = !*caps_lock;
            }
        } else if map_key(&INVARIANT_KEYMAP, code, &mut c)
            || (!shift && map_key(&UNSHIFTED_KEYMAP, code, &mut c))
            || (shift && map_key(&SHIFTED_KEYMAP, code, &mut c))
        {
            /* Ordinary character. */
            if !release {
                /* Handle Ctrl, Shift.
                Note that Ctrl overrides Shift. */
                if ctrl && c >= 0x40 && c < 0x60 {
                    /* A is 0x41, Ctrl+A is 0x01, etc. */
                    c -= 0x40;
                } else if shift == *core::ptr::addr_of!(CAPS_LOCK) {
                    c = c.to_ascii_lowercase();
                }

                /* Handle Alt by setting the high bit.
                This 0x80 is unrelated to the one used to
                distinguish key press from key release. */
                if alt {
                    c = c.wrapping_add(0x80);
                }

                /* Append to keyboard buffer. */
                if !crate::devices::input::input_full() {
                    *core::ptr::addr_of_mut!(KEY_CNT) += 1;
                    crate::devices::input::input_putc(c);
                }
            }
        } else {
            /* Maps a keycode into a shift state variable. */
            type ShiftKey = (u32, *mut bool);

            /* Table of shift keys. */
            let shift_keys: [ShiftKey; 6] = [
                (0x2a, core::ptr::addr_of_mut!(LEFT_SHIFT)),
                (0x36, core::ptr::addr_of_mut!(RIGHT_SHIFT)),
                (0x38, core::ptr::addr_of_mut!(LEFT_ALT)),
                (0xe038, core::ptr::addr_of_mut!(RIGHT_ALT)),
                (0x1d, core::ptr::addr_of_mut!(LEFT_CTRL)),
                (0xe01d, core::ptr::addr_of_mut!(RIGHT_CTRL)),
            ];

            /* Scan the table. */
            for key in shift_keys {
                if key.0 == code {
                    *key.1 = !release;
                    break;
                }
            }
        }
    }
}

/* Scans the array of keymaps K for SCANCODE.
If found, sets *C to the corresponding character and returns
true.
If not found, returns false and C is ignored. */
fn map_key(k: &[Keymap], scancode: u32, c: &mut u8) -> bool {
    for km in k {
        if km.first_scancode == 0 {
            break;
        }
        let first = km.first_scancode as u32;
        if scancode >= first && scancode < first + unsafe { strlen(km.chars) } as u32 {
            *c = unsafe { *km.chars.add((scancode - first) as usize) };
            return true;
        }
    }

    false
}
