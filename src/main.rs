//! pintos_rs — KAIST Pintos (x86_64) ported to Rust.
//!
//! Boot chain: `loader.S` (BIOS MBR) loads this kernel at physical
//! 0x200000 and jumps to `_start` (asm/start.S), which switches to
//! 64-bit long mode and calls `main` below (link address 0x8004200000).

#![no_std]
#![no_main]

mod intrinsic;

mod threads {
    pub mod loader;
    pub mod pte;
    pub mod vaddr;
}

use core::panic::PanicInfo;

/// C entry point called by asm/start.S after the 32→64-bit transition.
#[unsafe(no_mangle)]
pub extern "C" fn main() -> ! {
    // Minimal pipeline test: write to COM1 (0x3f8) in polled mode, then
    // power QEMU off through the isa-debug-exit-compatible port 0x604.
    // (Replaced by the full boot sequence as modules land.)
    unsafe {
        for &b in b"pintos_rs: boot pipeline OK\n" {
            while intrinsic::inb(0x3f8 + 5) & 0x20 == 0 {}
            intrinsic::outb(0x3f8, b);
        }
        intrinsic::outw(0x604, 0x2000);
    }
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
