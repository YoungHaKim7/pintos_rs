//! Port of `devices/disk.c` — ATA (IDE) controller driver.
//!
//! The code in this file is an interface to an ATA (IDE)
//! controller.  It attempts to comply to [ATA-3].

use crate::ASSERT;
use crate::devices::timer::{timer_msleep, timer_nsleep, timer_usleep};
use crate::intrinsic::{inb, insw, outb, outsw};
use crate::lib::debug::debug_panic;
use crate::lib::kernel::console::vprintf;
use crate::lib::stdio::Arg;
use crate::printf;
use crate::threads::interrupt::{
    IntrFrame, IntrLevel, intr_get_level, intr_register_ext, intr_register_int,
};
use crate::threads::synch::{
    Lock, Semaphore, lock_acquire, lock_init, lock_release, sema_down, sema_init, sema_up,
};

/* Size of a disk sector in bytes. */
pub const DISK_SECTOR_SIZE: usize = 512;

/* Index of a disk sector within a disk.
Good enough for disks up to 2 TB. */
pub type DiskSector = u32;

/* ATA command block port addresses. */
unsafe fn reg_data(c: *const Channel) -> u16 {
    /* Data. */
    unsafe { (*c).reg_base + 0 }
}
unsafe fn reg_error(c: *const Channel) -> u16 {
    /* Error. */
    unsafe { (*c).reg_base + 1 }
}
unsafe fn reg_nsect(c: *const Channel) -> u16 {
    /* Sector Count. */
    unsafe { (*c).reg_base + 2 }
}
unsafe fn reg_lbal(c: *const Channel) -> u16 {
    /* LBA 0:7. */
    unsafe { (*c).reg_base + 3 }
}
unsafe fn reg_lbam(c: *const Channel) -> u16 {
    /* LBA 15:8. */
    unsafe { (*c).reg_base + 4 }
}
unsafe fn reg_lbah(c: *const Channel) -> u16 {
    /* LBA 23:16. */
    unsafe { (*c).reg_base + 5 }
}
unsafe fn reg_device(c: *const Channel) -> u16 {
    /* Device/LBA 27:24. */
    unsafe { (*c).reg_base + 6 }
}
unsafe fn reg_status(c: *const Channel) -> u16 {
    /* Status (r/o). */
    unsafe { (*c).reg_base + 7 }
}
/* reg_command is reg_status (Command, w/o). */

/* ATA control block port addresses.
(If we supported non-legacy ATA controllers this would not be
flexible enough, but it's fine for what we do.) */
unsafe fn reg_ctl(c: *const Channel) -> u16 {
    /* Control (w/o). */
    unsafe { (*c).reg_base + 0x206 }
}
/* reg_alt_status is reg_ctl (Alt Status, r/o). */

/* Alternate Status Register bits. */
const STA_BSY: u8 = 0x80; /* Busy. */
const STA_DRDY: u8 = 0x40; /* Device Ready. */
const STA_DRQ: u8 = 0x08; /* Data Request. */

/* Control Register bits. */
const CTL_SRST: u8 = 0x04; /* Software Reset. */

/* Device Register bits. */
const DEV_MBS: u8 = 0xa0; /* Must be set. */
const DEV_LBA: u8 = 0x40; /* Linear based addressing. */
const DEV_DEV: u8 = 0x10; /* Select device: 0=master, 1=slave. */

/* Commands.
Many more are defined but this is the small subset that we
use. */
const CMD_IDENTIFY_DEVICE: u8 = 0xec; /* IDENTIFY DEVICE. */
const CMD_READ_SECTOR_RETRY: u8 = 0x20; /* READ SECTOR with retries. */
const CMD_WRITE_SECTOR_RETRY: u8 = 0x30; /* WRITE SECTOR with retries. */

/* An ATA device. */
#[repr(C)]
pub struct Disk {
    pub name: [u8; 8],         /* Name, e.g. "hd0:1". */
    pub channel: *mut Channel, /* Channel disk is on. */
    pub dev_no: i32,           /* Device 0 or 1 for master or slave. */

    pub is_ata: bool,         /* 1=This device is an ATA disk. */
    pub capacity: DiskSector, /* Capacity in sectors (if is_ata). */

    pub read_cnt: i64,  /* Number of sectors read. */
    pub write_cnt: i64, /* Number of sectors written. */
}

/* An ATA channel (aka controller).
Each channel can control up to two disks. */
#[repr(C)]
pub struct Channel {
    pub name: [u8; 8], /* Name, e.g. "hd0". */
    pub reg_base: u16, /* Base I/O port. */
    pub irq: u8,       /* Interrupt in use. */

    pub lock: Lock, /* Must acquire to access the controller. */
    pub expecting_interrupt: bool, /* True if an interrupt is expected, false if
                    any interrupt would be spurious. */
    pub completion_wait: Semaphore, /* Up'd by interrupt handler. */

    pub devices: [Disk; 2], /* The devices on this channel. */
}

/* We support the two "legacy" ATA channels found in a standard PC. */
const CHANNEL_CNT: usize = 2;
static mut CHANNELS: [Channel; CHANNEL_CNT] = unsafe { core::mem::zeroed() };

/* C: snprintf (c->name, sizeof c->name, "hd%zu", chan_no). */
unsafe fn set_channel_name(c: *mut Channel, chan_no: usize) {
    unsafe {
        (*c).name = [0; 8];
        (*c).name[0] = b'h';
        (*c).name[1] = b'd';
        (*c).name[2] = b'0' + chan_no as u8;
    }
}

/* C: snprintf (d->name, sizeof d->name, "%s:%d", c->name, dev_no). */
unsafe fn set_disk_name(d: *mut Disk, c: *const Channel, dev_no: usize) {
    unsafe {
        (*d).name = [0; 8];
        let mut i = 0;
        while i < 7 && (*c).name[i] != 0 {
            (*d).name[i] = (*c).name[i];
            i += 1;
        }
        (*d).name[i] = b':';
        (*d).name[i + 1] = b'0' + dev_no as u8;
    }
}

/* Initialize the disk subsystem and detect disks. */
pub fn disk_init() {
    unsafe {
        for chan_no in 0..CHANNEL_CNT {
            let c = core::ptr::addr_of_mut!(CHANNELS[chan_no]);

            /* Initialize channel. */
            set_channel_name(c, chan_no);
            match chan_no {
                0 => {
                    (*c).reg_base = 0x1f0;
                    (*c).irq = 14 + 0x20;
                }
                1 => {
                    (*c).reg_base = 0x170;
                    (*c).irq = 15 + 0x20;
                }
                _ => {
                    debug_panic(file!(), line!(), "disk_init: NOT REACHED", &[]);
                }
            }
            lock_init(core::ptr::addr_of_mut!((*c).lock));
            (*c).expecting_interrupt = false;
            sema_init(core::ptr::addr_of_mut!((*c).completion_wait), 0);

            /* Initialize devices. */
            for dev_no in 0..2 {
                let d = core::ptr::addr_of_mut!((*c).devices[dev_no]);
                set_disk_name(d, c, dev_no);
                (*d).channel = c;
                (*d).dev_no = dev_no as i32;

                (*d).is_ata = false;
                (*d).capacity = 0;

                (*d).read_cnt = 0;
                (*d).write_cnt = 0;
            }

            /* Register interrupt handler. */
            intr_register_ext((*c).irq, interrupt_handler, (*c).name.as_ptr());

            /* Reset hardware. */
            reset_channel(c);

            /* Distinguish ATA hard disks from other devices. */
            if check_device_type(core::ptr::addr_of_mut!((*c).devices[0])) {
                check_device_type(core::ptr::addr_of_mut!((*c).devices[1]));
            }

            /* Read hard disk identity information. */
            for dev_no in 0..2 {
                if (*core::ptr::addr_of!((*c).devices[dev_no])).is_ata {
                    identify_ata_device(core::ptr::addr_of_mut!((*c).devices[dev_no]));
                }
            }
        }

        /* DO NOT MODIFY BELOW LINES. */
        register_disk_inspect_intr();
    }
}

/* Prints disk statistics. */
pub fn disk_print_stats() {
    unsafe {
        for chan_no in 0..CHANNEL_CNT {
            for dev_no in 0..2 {
                let d = disk_get(chan_no as i32, dev_no as i32);
                if !d.is_null() && (*d).is_ata {
                    vprintf(
                        "%s: %lld reads, %lld writes\n",
                        &[
                            Arg::Raw((*d).name.as_ptr()),
                            Arg::I((*d).read_cnt),
                            Arg::I((*d).write_cnt),
                        ],
                    );
                }
            }
        }
    }
}

/* Returns the disk numbered DEV_NO--either 0 or 1 for master or
slave, respectively--within the channel numbered CHAN_NO.

Pintos uses disks this way:
0:0 - boot loader, command line args, and operating system kernel
0:1 - file system
1:0 - scratch
1:1 - swap
*/
pub unsafe fn disk_get(chan_no: i32, dev_no: i32) -> *mut Disk {
    unsafe {
        ASSERT!(dev_no == 0 || dev_no == 1);

        if chan_no < CHANNEL_CNT as i32 {
            let d = core::ptr::addr_of_mut!(CHANNELS[chan_no as usize].devices[dev_no as usize]);
            if (*d).is_ata {
                return d;
            }
        }
        core::ptr::null_mut()
    }
}

/* Returns the size of disk D, measured in DISK_SECTOR_SIZE-byte
sectors. */
pub unsafe fn disk_size(d: *mut Disk) -> DiskSector {
    unsafe {
        ASSERT!(!d.is_null());

        (*d).capacity
    }
}

/* Reads sector SEC_NO from disk D into BUFFER, which must have
room for DISK_SECTOR_SIZE bytes.
Internally synchronizes accesses to disks, so external
per-disk locking is unneeded. */
pub unsafe fn disk_read(d: *mut Disk, sec_no: DiskSector, buffer: *mut u8) {
    unsafe {
        ASSERT!(!d.is_null());
        ASSERT!(!buffer.is_null());

        let c = (*d).channel;
        lock_acquire(core::ptr::addr_of_mut!((*c).lock));
        select_sector(d, sec_no);
        issue_pio_command(c, CMD_READ_SECTOR_RETRY);
        sema_down(core::ptr::addr_of_mut!((*c).completion_wait));
        if !wait_while_busy(d) {
            debug_panic(
                file!(),
                line!(),
                "%s: disk read failed, sector=%u",
                &[Arg::Raw((*d).name.as_ptr()), Arg::U(sec_no as u64)],
            );
        }
        input_sector(c, buffer);
        (*d).read_cnt += 1;
        lock_release(core::ptr::addr_of_mut!((*c).lock));
    }
}

/* Write sector SEC_NO to disk D from BUFFER, which must contain
DISK_SECTOR_SIZE bytes.  Returns after the disk has
acknowledged receiving the data.
Internally synchronizes accesses to disks, so external
per-disk locking is unneeded. */
pub unsafe fn disk_write(d: *mut Disk, sec_no: DiskSector, buffer: *const u8) {
    unsafe {
        ASSERT!(!d.is_null());
        ASSERT!(!buffer.is_null());

        let c = (*d).channel;
        lock_acquire(core::ptr::addr_of_mut!((*c).lock));
        select_sector(d, sec_no);
        issue_pio_command(c, CMD_WRITE_SECTOR_RETRY);
        if !wait_while_busy(d) {
            debug_panic(
                file!(),
                line!(),
                "%s: disk write failed, sector=%u",
                &[Arg::Raw((*d).name.as_ptr()), Arg::U(sec_no as u64)],
            );
        }
        output_sector(c, buffer);
        sema_down(core::ptr::addr_of_mut!((*c).completion_wait));
        (*d).write_cnt += 1;
        lock_release(core::ptr::addr_of_mut!((*c).lock));
    }
}

/* Disk detection and identification. */

/* Resets an ATA channel and waits for any devices present on it
to finish the reset. */
unsafe fn reset_channel(c: *mut Channel) {
    unsafe {
        let mut present = [false; 2];

        /* The ATA reset sequence depends on which devices are present,
        so we start by detecting device presence. */
        for dev_no in 0..2 {
            let d = core::ptr::addr_of_mut!((*c).devices[dev_no]);

            select_device(d);

            outb(reg_nsect(c), 0x55);
            outb(reg_lbal(c), 0xaa);

            outb(reg_nsect(c), 0xaa);
            outb(reg_lbal(c), 0x55);

            outb(reg_nsect(c), 0x55);
            outb(reg_lbal(c), 0xaa);

            present[dev_no] = inb(reg_nsect(c)) == 0x55 && inb(reg_lbal(c)) == 0xaa;
        }

        /* Issue soft reset sequence, which selects device 0 as a side effect.
        Also enable interrupts. */
        outb(reg_ctl(c), 0);
        timer_usleep(10);
        outb(reg_ctl(c), CTL_SRST);
        timer_usleep(10);
        outb(reg_ctl(c), 0);

        timer_msleep(150);

        /* Wait for device 0 to clear BSY. */
        if present[0] {
            select_device(core::ptr::addr_of_mut!((*c).devices[0]));
            wait_while_busy(core::ptr::addr_of_mut!((*c).devices[0]));
        }

        /* Wait for device 1 to clear BSY. */
        if present[1] {
            select_device(core::ptr::addr_of_mut!((*c).devices[1]));
            for _i in 0..3000 {
                if inb(reg_nsect(c)) == 1 && inb(reg_lbal(c)) == 1 {
                    break;
                }
                timer_msleep(10);
            }
            wait_while_busy(core::ptr::addr_of_mut!((*c).devices[1]));
        }
    }
}

/* Checks whether device D is an ATA disk and sets D's is_ata
member appropriately.  If D is device 0 (master), returns true
if it's possible that a slave (device 1) exists on this
channel.  If D is device 1 (slave), the return value is not
meaningful. */
unsafe fn check_device_type(d: *mut Disk) -> bool {
    unsafe {
        let c = (*d).channel;

        select_device(d);

        let error = inb(reg_error(c));
        let lbam = inb(reg_lbam(c));
        let lbah = inb(reg_lbah(c));
        let status = inb(reg_status(c));

        if (error != 1 && (error != 0x81 || (*d).dev_no == 1))
            || (status & STA_DRDY) == 0
            || (status & STA_BSY) != 0
        {
            (*d).is_ata = false;
            error != 0x81
        } else {
            (*d).is_ata = (lbam == 0 && lbah == 0) || (lbam == 0x3c && lbah == 0xc3);
            true
        }
    }
}

/* Sends an IDENTIFY DEVICE command to disk D and reads the
response.  Initializes D's capacity member based on the result
and prints a message describing the disk to the console. */
unsafe fn identify_ata_device(d: *mut Disk) {
    unsafe {
        let c = (*d).channel;
        let mut id = [0u16; DISK_SECTOR_SIZE / 2];

        ASSERT!((*d).is_ata);

        /* Send the IDENTIFY DEVICE command, wait for an interrupt
        indicating the device's response is ready, and read the data
        into our buffer. */
        select_device_wait(d);
        issue_pio_command(c, CMD_IDENTIFY_DEVICE);
        sema_down(core::ptr::addr_of_mut!((*c).completion_wait));
        if !wait_while_busy(d) {
            (*d).is_ata = false;
            return;
        }
        input_sector(c, id.as_mut_ptr() as *mut u8);

        /* Calculate capacity. */
        (*d).capacity = id[60] as u32 | ((id[61] as u32) << 16);

        /* Print identification message. */
        vprintf(
            "%s: detected %'u sector (",
            &[Arg::Raw((*d).name.as_ptr()), Arg::U((*d).capacity as u64)],
        );
        let kb_sector = (1024 / DISK_SECTOR_SIZE) as u32; /* Sectors per kB. */
        if (*d).capacity > kb_sector * 1024 * 1024 {
            vprintf(
                "%'u GB",
                &[Arg::U(((*d).capacity / (kb_sector * 1024 * 1024)) as u64)],
            );
        } else if (*d).capacity > kb_sector * 1024 {
            vprintf(
                "%'u MB",
                &[Arg::U(((*d).capacity / (kb_sector * 1024)) as u64)],
            );
        } else if (*d).capacity > kb_sector {
            vprintf("%'u kB", &[Arg::U(((*d).capacity / kb_sector) as u64)]);
        } else {
            vprintf(
                "%'u byte",
                &[Arg::U(((*d).capacity * DISK_SECTOR_SIZE as u32) as u64)],
            );
        }
        printf!(") disk, model \"");
        print_ata_string(core::ptr::addr_of_mut!(id[27]) as *mut u8, 40);
        printf!("\", serial \"");
        print_ata_string(core::ptr::addr_of_mut!(id[10]) as *mut u8, 20);
        printf!("\"\n");
    }
}

/* C's isspace(). */
fn isspace(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/* Prints STRING, which consists of SIZE bytes in a funky format:
each pair of bytes is in reverse order.  Does not print
trailing whitespace and/or nulls. */
unsafe fn print_ata_string(string: *mut u8, mut size: usize) {
    unsafe {
        /* Find the last non-white, non-null character. */
        while size > 0 {
            let c = *string.add((size - 1) ^ 1);
            if c != b'\0' && !isspace(c) {
                break;
            }
            size -= 1;
        }

        /* Print. */
        for i in 0..size {
            printf!("%c", *string.add(i ^ 1) as char);
        }
    }
}

/* Selects device D, waiting for it to become ready, and then
writes SEC_NO to the disk's sector selection registers.  (We
use LBA mode.) */
unsafe fn select_sector(d: *mut Disk, sec_no: DiskSector) {
    unsafe {
        let c = (*d).channel;

        ASSERT!(sec_no < (*d).capacity);
        ASSERT!(sec_no < (1u32 << 28));

        select_device_wait(d);
        outb(reg_nsect(c), 1);
        outb(reg_lbal(c), sec_no as u8);
        outb(reg_lbam(c), (sec_no >> 8) as u8);
        outb(reg_lbah(c), (sec_no >> 16) as u8);
        outb(
            reg_device(c),
            DEV_MBS | DEV_LBA | (if (*d).dev_no == 1 { DEV_DEV } else { 0 }) | (sec_no >> 24) as u8,
        );
    }
}

/* Writes COMMAND to channel C and prepares for receiving a
completion interrupt. */
unsafe fn issue_pio_command(c: *mut Channel, command: u8) {
    unsafe {
        /* Interrupts must be enabled or our semaphore will never be
        up'd by the completion handler. */
        ASSERT!(intr_get_level() == IntrLevel::On);

        (*c).expecting_interrupt = true;
        outb(reg_status(c), command);
    }
}

/* Reads a sector from channel C's data register in PIO mode into
SECTOR, which must have room for DISK_SECTOR_SIZE bytes. */
unsafe fn input_sector(c: *mut Channel, sector: *mut u8) {
    unsafe {
        insw(reg_data(c), sector as *mut u16, DISK_SECTOR_SIZE / 2);
    }
}

/* Writes SECTOR to channel C's data register in PIO mode.
SECTOR must contain DISK_SECTOR_SIZE bytes. */
unsafe fn output_sector(c: *mut Channel, sector: *const u8) {
    unsafe {
        outsw(reg_data(c), sector as *const u16, DISK_SECTOR_SIZE / 2);
    }
}

/* Low-level ATA primitives. */

/* Wait up to 10 seconds for the controller to become idle, that
is, for the BSY and DRQ bits to clear in the status register.

As a side effect, reading the status register clears any
pending interrupt. */
unsafe fn wait_until_idle(d: *const Disk) {
    unsafe {
        for _i in 0..1000 {
            if (inb(reg_status((*d).channel)) & (STA_BSY | STA_DRQ)) == 0 {
                return;
            }
            timer_usleep(10);
        }

        vprintf("%s: idle timeout\n", &[Arg::Raw((*d).name.as_ptr())]);
    }
}

/* Wait up to 30 seconds for disk D to clear BSY,
and then return the status of the DRQ bit.
The ATA standards say that a disk may take as long as that to
complete its reset. */
unsafe fn wait_while_busy(d: *const Disk) -> bool {
    unsafe {
        let c = (*d).channel;

        for i in 0..3000 {
            if i == 700 {
                vprintf("%s: busy, waiting...", &[Arg::Raw((*d).name.as_ptr())]);
            }
            if (inb(reg_ctl(c)) & STA_BSY) == 0 {
                if i >= 700 {
                    printf!("ok\n");
                }
                return (inb(reg_ctl(c)) & STA_DRQ) != 0;
            }
            timer_msleep(10);
        }

        printf!("failed\n");
        false
    }
}

/* Program D's channel so that D is now the selected disk. */
unsafe fn select_device(d: *const Disk) {
    unsafe {
        let c = (*d).channel;
        let mut dev = DEV_MBS;
        if (*d).dev_no == 1 {
            dev |= DEV_DEV;
        }
        outb(reg_device(c), dev);
        inb(reg_ctl(c));
        timer_nsleep(400);
    }
}

/* Select disk D in its channel, as select_device(), but wait for
the channel to become idle before and after. */
unsafe fn select_device_wait(d: *const Disk) {
    unsafe {
        wait_until_idle(d);
        select_device(d);
        wait_until_idle(d);
    }
}

/* ATA interrupt handler. */
extern "C" fn interrupt_handler(f: *mut IntrFrame) {
    unsafe {
        for chan_no in 0..CHANNEL_CNT {
            let c = core::ptr::addr_of_mut!(CHANNELS[chan_no]);
            if (*f).vec_no as usize == (*c).irq as usize {
                if (*c).expecting_interrupt {
                    inb(reg_status(c)); /* Acknowledge interrupt. */
                    sema_up(core::ptr::addr_of_mut!((*c).completion_wait)); /* Wake up waiter. */
                } else {
                    vprintf(
                        "%s: unexpected interrupt\n",
                        &[Arg::Raw((*c).name.as_ptr())],
                    );
                }
                return;
            }
        }

        /* NOT_REACHED. */
        debug_panic(
            file!(),
            line!(),
            "disk: interrupt_handler: NOT REACHED",
            &[],
        );
    }
}

extern "C" fn inspect_read_cnt(f: *mut IntrFrame) {
    unsafe {
        let d = disk_get((*f).R.rdx as i32, (*f).R.rcx as i32);
        (*f).R.rax = (*d).read_cnt as u64;
    }
}

extern "C" fn inspect_write_cnt(f: *mut IntrFrame) {
    unsafe {
        let d = disk_get((*f).R.rdx as i32, (*f).R.rcx as i32);
        (*f).R.rax = (*d).write_cnt as u64;
    }
}

/* Tool for testing disk r/w cnt. Calling this function via int 0x43 and int 0x44.
 * Input:
 *   @RDX - chan_no of disk to inspect
 *   @RCX - dev_no of disk to inspect
 * Output:
 *   @RAX - Read/Write count of disk. */
pub fn register_disk_inspect_intr() {
    unsafe {
        intr_register_int(
            0x43,
            3,
            IntrLevel::Off,
            inspect_read_cnt,
            b"Inspect Disk Read Count\0".as_ptr(),
        );
        intr_register_int(
            0x44,
            3,
            IntrLevel::Off,
            inspect_write_cnt,
            b"Inspect Disk Write Count\0".as_ptr(),
        );
    }
}
