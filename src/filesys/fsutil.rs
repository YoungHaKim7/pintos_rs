//! Port of `filesys/fsutil.c` — file-system utilities.
//!
//! `put`/`get` implement the pintos -p/-a file-transfer flow:
//! files are staged on the "scratch" disk (hd1:0) with a
//! "PUT\0"/"GET\0" + u32-size header sector followed by data
//! sectors; the kernel copies between the scratch disk and the
//! FAT file system.

use crate::devices::disk::{DISK_SECTOR_SIZE, disk_get, disk_read, disk_size, disk_write};
use crate::filesys::directory::{NAME_MAX, dir_open_root, dir_readdir};
use crate::filesys::file::{file_close, file_length, file_read, file_tell, file_write};
use crate::filesys::{filesys_create, filesys_open, filesys_remove};
use crate::lib::stdio::Arg;
use crate::printf;
use crate::threads::malloc::{free, malloc};
use crate::threads::palloc::{palloc_free_page, palloc_get_page, PAL_ASSERT};

/* List files in the root directory. */
pub unsafe fn fsutil_ls(argv: *mut *mut u8) {
    unsafe {
        let _ = argv;
        let mut name = [0u8; NAME_MAX + 1];

        printf!("Files in the root directory:\n");
        let dir = dir_open_root();
        if dir.is_null() {
            crate::PANIC!("root dir open failed");
        }
        while dir_readdir(dir, name.as_mut_ptr()) {
            printf!("%s\n", Arg::Raw(name.as_ptr()));
        }
        printf!("End of listing.\n");
    }
}

/* Prints the contents of file ARGV[1] to the system console as
hex and ASCII. */
pub unsafe fn fsutil_cat(argv: *mut *mut u8) {
    unsafe {
        let file_name = *argv.add(1);

        printf!("Printing '%s' to the console...\n", Arg::Raw(file_name));
        let file = filesys_open(file_name);
        if file.is_null() {
            crate::PANIC!("%s: open failed", Arg::Raw(file_name));
        }
        let buffer = palloc_get_page(PAL_ASSERT) as *mut u8;
        loop {
            let pos = file_tell(file);
            let n = file_read(file, buffer, crate::threads::vaddr::PGSIZE as i32);
            if n == 0 {
                break;
            }

            crate::lib::stdio::hex_dump(pos as usize, buffer, n as usize, true);
        }
        palloc_free_page(buffer);
        file_close(file);
    }
}

/* Deletes file ARGV[1]. */
pub unsafe fn fsutil_rm(argv: *mut *mut u8) {
    unsafe {
        let file_name = *argv.add(1);

        printf!("Deleting '%s'...\n", Arg::Raw(file_name));
        if !filesys_remove(file_name) {
            crate::PANIC!("%s: delete failed\n", Arg::Raw(file_name));
        }
    }
}

/* Copies from the "scratch" disk, hdc or hd1:0, to file ARGV[1]
in the file system.

The current sector on the scratch disk must begin with the
string "PUT\0" followed by a 32-bit little-endian integer
indicating the file size in bytes.  Subsequent sectors hold the
file content.

The first call to this function will read starting at the
beginning of the scratch disk.  Later calls advance across the
disk.  This disk position is independent of that used for
fsutil_get(), so all `put's should precede all `get's. */
pub unsafe fn fsutil_put(argv: *mut *mut u8) {
    unsafe {
        static mut SECTOR: u32 = 0;

        let file_name = *argv.add(1);
        let mut size: i32;

        printf!("Putting '%s' into the file system...\n", Arg::Raw(file_name));

        /* Allocate buffer. */
        let buffer = malloc(DISK_SECTOR_SIZE);
        if buffer.is_null() {
            crate::PANIC!("couldn't allocate buffer");
        }

        /* Open the source disk and read the file size. */
        let src = disk_get(1, 0);
        if src.is_null() {
            crate::PANIC!("couldn't open source disk (hdc or hd1:0)");
        }

        /* Read the file size. */
        disk_read(src, SECTOR, buffer);
        SECTOR += 1;
        if crate::lib::string::memcmp(buffer, b"PUT\0".as_ptr(), 4) != 0 {
            crate::PANIC!("%s: missing PUT signature on scratch disk", Arg::Raw(file_name));
        }
        size = *(buffer.add(4) as *const i32);
        if size < 0 {
            crate::PANIC!("%s: invalid file size %d", Arg::Raw(file_name), Arg::I(size as i64));
        }

        /* Create the destination file. */
        if !filesys_create(file_name, size) {
            crate::PANIC!("%s: create failed", Arg::Raw(file_name));
        }
        let dst = filesys_open(file_name);
        if dst.is_null() {
            crate::PANIC!("%s: open failed", Arg::Raw(file_name));
        }

        /* Do the copy. */
        while size > 0 {
            let chunk_size = if size > DISK_SECTOR_SIZE as i32 {
                DISK_SECTOR_SIZE as i32
            } else {
                size
            };
            disk_read(src, SECTOR, buffer);
            SECTOR += 1;
            if file_write(dst, buffer, chunk_size) != chunk_size {
                crate::PANIC!(
                    "%s: write failed with %d bytes unwritten",
                    Arg::Raw(file_name),
                    Arg::I(size as i64)
                );
            }
            size -= chunk_size;
        }

        /* Finish up. */
        file_close(dst);
        free(buffer);
    }
}

/* Copies file ARGV[1] from the file system to the scratch disk.

The current sector on the scratch disk will receive "GET\0"
followed by the file's size in bytes as a 32-bit little-endian
integer.  Subsequent sectors receive the file's data. */
pub unsafe fn fsutil_get(argv: *mut *mut u8) {
    unsafe {
        static mut SECTOR: u32 = 0;

        let file_name = *argv.add(1);
        let mut size: i32;

        printf!("Getting '%s' from the file system...\n", Arg::Raw(file_name));

        /* Allocate buffer. */
        let buffer = malloc(DISK_SECTOR_SIZE);
        if buffer.is_null() {
            crate::PANIC!("couldn't allocate buffer");
        }

        /* Open the source file. */
        let src = filesys_open(file_name);
        if src.is_null() {
            crate::PANIC!("%s: open failed", Arg::Raw(file_name));
        }
        size = file_length(src);

        /* Open the target disk. */
        let dst = disk_get(1, 0);
        if dst.is_null() {
            crate::PANIC!("couldn't open target disk (hdc or hd1:0)");
        }

        /* Write the size to sector 0. */
        crate::lib::string::memset(buffer, 0, DISK_SECTOR_SIZE);
        crate::lib::string::memcpy(buffer, b"GET\0".as_ptr(), 4);
        *(buffer.add(4) as *mut i32) = size;
        disk_write(dst, SECTOR, buffer);
        SECTOR += 1;

        /* Do the copy. */
        while size > 0 {
            let chunk_size = if size > DISK_SECTOR_SIZE as i32 {
                DISK_SECTOR_SIZE as i32
            } else {
                size
            };
            if SECTOR >= disk_size(dst) {
                crate::PANIC!("%s: out of space on scratch disk", Arg::Raw(file_name));
            }
            if file_read(src, buffer, chunk_size) != chunk_size {
                crate::PANIC!(
                    "%s: read failed with %d bytes unread",
                    Arg::Raw(file_name),
                    Arg::I(size as i64)
                );
            }
            crate::lib::string::memset(
                buffer.add(chunk_size as usize),
                0,
                (DISK_SECTOR_SIZE as i32 - chunk_size) as usize,
            );
            disk_write(dst, SECTOR, buffer);
            SECTOR += 1;
            size -= chunk_size;
        }

        /* Finish up. */
        file_close(src);
        free(buffer);
    }
}
