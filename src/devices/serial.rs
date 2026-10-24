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
#[derive(PartialEq, Clone, Copy)]
enum Mode {
    Uninit,
    Poll,
    Queue,
}

static mut MODE: Mode = Mode::Uninit;

/* Data to be transmitted. */
static mut TXQ: crate::devices::intq::Intq = crate::devices::intq::Intq::EMPTY;

use crate::devices::input::{input_full, input_putc};
use crate::devices::intq::{intq_empty, intq_full, intq_getc, intq_init, intq_putc};
use crate::threads::interrupt::intr_register_ext;

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
        intq_init(&raw mut TXQ);
        MODE = Mode::Poll;
    }
}

/* Initializes the serial port device for queued interrupt-driven
I/O.  With interrupt-driven I/O we don't waste CPU time waiting
for the serial device to become ready. */
pub fn serial_init_queue() {
    unsafe {
        if MODE == Mode::Uninit {
            init_poll();
        }
        debug_assert!(MODE == Mode::Poll);

        intr_register_ext(0x20 + 4, serial_interrupt, b"serial\0".as_ptr());
        MODE = Mode::Queue;
        let old_level = intr_disable();
        write_ier();
        intr_set_level(old_level);
    }
}

/* Sends BYTE to the serial port. */
pub fn serial_putc(byte: u8) {
    let old_level = intr_disable();

    unsafe {
        let mode = *core::ptr::addr_of!(MODE);
        if mode != Mode::Queue {
            /* If we're not set up for interrupt-driven I/O yet,
            use dumb polling to transmit a byte. */
            if mode == Mode::Uninit {
                init_poll();
            }
            putc_poll(byte);
        } else {
            /* Otherwise, queue a byte and update the interrupt
            enable register. */
            if old_level == IntrLevel::Off && intq_full(&raw const TXQ) {
                /* Interrupts are off and the transmit queue is
                full.  If we wanted to wait for the queue to empty,
                we'd have to re-enable interrupts.  That's impolite,
                so we'll send a character via polling instead. */
                putc_poll(intq_getc(&raw mut TXQ));
            }

            intq_putc(&raw mut TXQ, byte);
            write_ier();
        }
    }

    intr_set_level(old_level);
}

/* Flushes anything in the serial buffer out the port in polling
mode. */
pub fn serial_flush() {
    let old_level = intr_disable();
    unsafe {
        while !intq_empty(&raw const TXQ) {
            putc_poll(intq_getc(&raw mut TXQ));
        }
    }
    intr_set_level(old_level);
}

/* The fullness of the input buffer may have changed.  Reassess
whether we should block receive interrupts.
Called by the input buffer routines when characters are added
to or removed from the buffer. */
pub fn serial_notify() {
    unsafe {
        debug_assert!(intr_get_level() == IntrLevel::Off);
        if MODE == Mode::Queue {
            write_ier();
        }
    }
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

/* Update interrupt enable register. */
unsafe fn write_ier() {
    unsafe {
        let mut ier: u8 = 0;

        debug_assert!(intr_get_level() == IntrLevel::Off);

        /* Enable transmit interrupt if we have any characters to
        transmit. */
        if !intq_empty(&raw const TXQ) {
            ier |= IER_XMIT;
        }

        /* Enable receive interrupt if we have room to store any
        characters we receive. */
        if !input_full() {
            ier |= IER_RECV;
        }

        outb(IER_REG, ier);
    }
}

/* Serial interrupt handler. */
extern "C" fn serial_interrupt(_f: *mut crate::threads::interrupt::IntrFrame) {
    unsafe {
        /* Inquire about interrupt in UART.  Without this, we can
        occasionally miss an interrupt running under QEMU. */
        inb(IIR_REG);

        /* As long as we have room to receive a byte, and the
        hardware has a byte for us, receive a byte. */
        while !input_full() && inb(LSR_REG) & LSR_DR != 0 {
            input_putc(inb(RBR_REG));
        }

        /* As long as we have a byte to transmit, and the hardware
        is ready to accept a byte for transmission, transmit a
        byte. */
        while !intq_empty(&raw const TXQ) && inb(LSR_REG) & LSR_THRE != 0 {
            outb(THR_REG, intq_getc(&raw mut TXQ));
        }

        /* Update interrupt enable register based on queue status. */
        write_ier();
    }
}
