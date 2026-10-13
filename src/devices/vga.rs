//! Port of `devices/vga.c` — VGA text screen support.
//! See [FREEVGA] for more information.

use crate::intrinsic::{inb, outb, outw};
use crate::lib::string::memmove;
use crate::threads::interrupt::{intr_disable, intr_set_level};
use crate::threads::vaddr::ptov;

/* Number of columns and rows on the text display. */
const COL_CNT: usize = 80;
const ROW_CNT: usize = 25;

/* Attribute value for gray text on a black background. */
const GRAY_ON_BLACK: u8 = 0x07;

/* Current cursor position.  (0,0) is in the upper left corner of
the display. */
static mut CX: usize = 0;
static mut CY: usize = 0;

/* Already initialized? */
static mut INITED: bool = false;

/* Framebuffer: the character at (x,y) is fb[y][x][0], the
attribute is fb[y][x][1]. */
static mut FB: *mut [[u8; 2]; COL_CNT] = core::ptr::null_mut();

/* Initializes the VGA text display. */
unsafe fn init() {
    unsafe {
        if !INITED {
            FB = ptov(0xb8000) as *mut [[u8; 2]; COL_CNT];
            let (x, y) = find_cursor();
            CX = x;
            CY = y;
            INITED = true;
        }
    }
}

/* Writes C to the VGA text display, interpreting control
characters in the conventional ways. */
pub fn vga_putc(c: u8) {
    /* Disable interrupts to lock out interrupt handlers
    that might write to the console. */
    let old_level = intr_disable();

    unsafe {
        init();

        match c {
            b'\n' => newline(),

            0x0c /* \f */ => cls(),

            0x08 /* \b */ => {
                if CX > 0 {
                    CX -= 1;
                }
            }

            b'\r' => CX = 0,

            b'\t' => {
                CX = (CX + 1 + 7) & !7; /* ROUND_UP(cx + 1, 8) */
                if CX >= COL_CNT {
                    newline();
                }
            }

            _ => {
                (*FB.add(CY))[CX][0] = c;
                (*FB.add(CY))[CX][1] = GRAY_ON_BLACK;
                CX += 1;
                if CX >= COL_CNT {
                    newline();
                }
            }
        }

        /* Update cursor position. */
        move_cursor();
    }

    intr_set_level(old_level);
}

/* Clears the screen and moves the cursor to the upper left. */
unsafe fn cls() {
    unsafe {
        for y in 0..ROW_CNT {
            clear_row(y);
        }
        CX = 0;
        CY = 0;
        move_cursor();
    }
}

/* Clears row Y to spaces. */
unsafe fn clear_row(y: usize) {
    unsafe {
        for x in 0..COL_CNT {
            (*FB.add(y))[x][0] = b' ';
            (*FB.add(y))[x][1] = GRAY_ON_BLACK;
        }
    }
}

/* Advances the cursor to the first column in the next line on
the screen.  If the cursor is already on the last line on
the screen, scrolls the screen upward one line. */
unsafe fn newline() {
    unsafe {
        CX = 0;
        CY += 1;
        if CY >= ROW_CNT {
            CY = ROW_CNT - 1;
            memmove(
                FB as *mut u8,
                FB.add(1) as *const u8,
                core::mem::size_of::<[[u8; 2]; COL_CNT]>() * (ROW_CNT - 1),
            );
            clear_row(ROW_CNT - 1);
        }
    }
}

/* Moves the hardware cursor to (cx,cy). */
unsafe fn move_cursor() {
    unsafe {
        /* See [FREEVGA] under "Manipulating the Text-mode Cursor". */
        let cp = (CX + COL_CNT * CY) as u16;
        outw(0x3d4, 0x0e | (cp & 0xff00));
        outw(0x3d4, 0x0f | (cp << 8));
    }
}

/* Reads the current hardware cursor position into (X,Y). */
unsafe fn find_cursor() -> (usize, usize) {
    unsafe {
        /* See [FREEVGA] under "Manipulating the Text-mode Cursor". */
        let mut cp: u16;

        outb(0x3d4, 0x0e);
        cp = (inb(0x3d5) as u16) << 8;

        outb(0x3d4, 0x0f);
        cp |= inb(0x3d5) as u16;

        (cp as usize % COL_CNT, cp as usize / COL_CNT)
    }
}
