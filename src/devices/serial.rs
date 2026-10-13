//! Port of `devices/serial.c` — 16550A UART driver.
//!
//! M0 subset: polling-mode transmission (`init_poll`, `putc_poll`,
//! `set_serial`) plus the `serial_putc` polling path.  Queue mode
//! (interrupt-driven TX/RX through `intq`) arrives with the
//! interrupts/threads milestone.

use crate::intrinsic::{inb, outb};
use crate::threads::interrupt::{IntrLevel, intr_disable, intr_get_level, intr_set_level};

/* Register definitions for the 16550A UART used in PCs.
The 16550A has a lot more going on than shown here, but this
is all we need.  Refer to [PC16650D] for hardware information. */

/* I/O port base address for the first serial port. */
const IO_BASE: u16 = 0x3f8;

/* DLAB=0 registers. */
const RBR_REG: u16 = IO_BASE + 0; /* Receiver Buffer Reg. (read-only). */
const THR_REG: u16 = IO_BASE + 0; /* Transmitter Holding Reg. (write-only). */
const IER_REG: u16 = IO_BASE + 1; /* Interrupt Enable Reg.. */

/* DLAB=1 registers. */
const LS_REG: u16 = IO_BASE + 0; /* Divisor Latch (LSB). */
const MS_REG: u16 = IO_BASE + 1; /* Divisor Latch (MSB). */

/* DLAB-insensitive registers. */
const IIR_REG: u16 = IO_BASE + 2; /* Interrupt Identification Reg. (read-only) */
const FCR_REG: u16 = IO_BASE + 2; /* FIFO Control Reg. (write-only). */
const LCR_REG: u16 = IO_BASE + 3; /* Line Control Register. */
const MCR_REG: u16 = IO_BASE + 4; /* MODEM Control Register. */
const LSR_REG: u16 = IO_BASE + 5; /* Line Status Register (read-only). */

/* Interrupt Enable Register bits. */
const IER_RECV: u8 = 0x01; /* Interrupt when data received. */
const IER_XMIT: u8 = 0x02; /* Interrupt when transmit finishes. */

/* Line Control Register bits. */
const LCR_N81: u8 = 0x03; /* No parity, 8 data bits, 1 stop bit. */
const LCR_DLAB: u8 = 0x80; /* Divisor Latch Access Bit (DLAB). */

/* MODEM Control Register. */
const MCR_OUT2: u8 = 0x08; /* Output line 2. */

/* Line Status Register. */
const LSR_DR: u8 = 0x01; /* Data Ready: received data byte is in RBR. */
const LSR_THRE: u8 = 0x20; /* THR Empty. */

/* Transmission mode. */
#[derive(PartialEq)]
enum Mode {
    Uninit,
    Poll,
    Queue,
}

static mut MODE: Mode = Mode::Uninit;

/* Initializes the serial port device for polling mode.
Polling mode busy-waits for the serial port to become free
before writing to it.  It's slow, but until interrupts have
been initialized it's all we can do. */
unsafe fn init_poll() {
    unsafe {
        debug_assert!(MODE == Mode::Uninit);
        outb(IER_REG, 0); /* Turn off all interrupts. */
        outb(FCR_REG, 0); /* Disable FIFO. */
        set_serial(115200); /* 115.2 kbps, N-8-1. */
        outb(MCR_REG, MCR_OUT2); /* Required to enable interrupts. */
        MODE = Mode::Poll;
    }
}

/* Initializes the serial port device for queued interrupt-driven
I/O.  (M1: registers the IRQ4 handler and switches to QUEUE.) */
pub fn serial_init_queue() {
    todo!("serial_init_queue: requires intr_register_ext + intq (M1)")
}

/* Sends BYTE to the serial port. */
pub fn serial_putc(byte: u8) {
    let old_level = intr_disable();

    unsafe {
        let mode = MODE;
        if mode != Mode::Queue {
            /* If we're not set up for interrupt-driven I/O yet,
            use dumb polling to transmit a byte. */
            if mode == Mode::Uninit {
                init_poll();
            }
            putc_poll(byte);
        } else {
            /* Queue a byte and update the interrupt enable register. */
            // M1: intq-based transmit path.
            putc_poll(byte);
        }
    }

    intr_set_level(old_level);
}

/* Flushes anything in the serial buffer out the port in polling
mode. */
pub fn serial_flush() {
    let old_level = intr_disable();
    // M1: drain txq via polling; nothing queued in polling mode.
    intr_set_level(old_level);
}

/* The fullness of the input buffer may have changed.  Reassess
whether we should block receive interrupts. */
pub fn serial_notify() {
    debug_assert!(intr_get_level() == IntrLevel::Off);
    /* M1: write_ier() when in QUEUE mode. */
}

/* Configures the serial port for BPS bits per second. */
unsafe fn set_serial(bps: i32) {
    unsafe {
        let base_rate: i32 = 1843200 / 16; /* Base rate of 16550A, in Hz. */
        let divisor: u16 = (base_rate / bps) as u16;

        debug_assert!(bps >= 300 && bps <= 115200);

        /* Enable DLAB and set data rate. */
        outb(LCR_REG, LCR_N81 | LCR_DLAB);
        outb(LS_REG, (divisor & 0xff) as u8);
        outb(MS_REG, (divisor >> 8) as u8);

        /* Reset DLAB. */
        outb(LCR_REG, LCR_N81);
    }
}

/* Polls the serial port until it's ready, and then transmits BYTE. */
unsafe fn putc_poll(byte: u8) {
    unsafe {
        debug_assert!(intr_get_level() == IntrLevel::Off);
        while inb(LSR_REG) & LSR_THRE == 0 {
            continue;
        }
        outb(THR_REG, byte);
    }
}

/* Silence unused-constant warnings for registers used only in M1. */
#[allow(dead_code)]
fn _unused() {
    let _ = (RBR_REG, IER_REG, IIR_REG, IER_RECV, IER_XMIT, LSR_DR);
}
