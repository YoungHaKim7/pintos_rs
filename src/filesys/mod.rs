//! Port of `filesys/filesys.c` — filesystem module glue.

pub mod directory;
pub mod fat;
pub mod file;
pub mod fsutil;
pub mod inode;

use crate::devices::disk::{Disk, disk_get};

/* The disk that contains the file system. */
pub static mut FILESYS_DISK: *mut Disk = core::ptr::null_mut();

/* Initializes the file system module.
If FORMAT is true, reformats the file system. */
pub unsafe fn filesys_init(format: bool) {
    unsafe {
        FILESYS_DISK = disk_get(0, 1);
        if FILESYS_DISK.is_null() {
            crate::PANIC!("hd0:1 (hdb) not present, file system initialization failed");
        }

        inode::inode_init();

        fat::fat_init();

        if format {
            do_format();
        }

        fat::fat_open();
    }
}

/* Shuts down the file system module, writing any unwritten data
to disk. */
pub unsafe fn filesys_done() {
    unsafe {
        fat::fat_close();
    }
}

/* Creates a file named NAME with the given INITIAL_SIZE.
Returns true if successful, false otherwise.
Fails if a file named NAME already exists, or if internal memory
allocation fails. */
pub unsafe fn filesys_create(name: *const u8, initial_size: i32) -> bool {
    unsafe {
        /* EFILESYS: the inode's sector comes from a fresh FAT
        chain. */
        let inode_clst = fat::fat_create_chain(0);
        let inode_sector = if inode_clst != 0 {
            fat::cluster_to_sector(inode_clst)
        } else {
            0
        };

        let dir = directory::dir_open_root();
        let success = inode_sector != 0
            && !dir.is_null()
            && inode::inode_create(inode_sector, initial_size)
            && directory::dir_add(dir, name, inode_sector);
        if !success && inode_sector != 0 {
            fat::fat_remove_chain(inode_clst, 0);
        }
        directory::dir_close(dir);

        success
    }
}

/* Opens the file with the given NAME.
Returns the new file if successful or a null pointer otherwise.
Fails if no file named NAME exists, or if an internal memory
allocation fails. */
pub unsafe fn filesys_open(name: *const u8) -> *mut file::File {
    unsafe {
        let dir = directory::dir_open_root();
        let mut inode: *mut inode::Inode = core::ptr::null_mut();

        if !dir.is_null() {
            directory::dir_lookup(dir, name, &raw mut inode);
        }
        directory::dir_close(dir);

        file::file_open(inode)
    }
}

/* Deletes the file named NAME.
Returns true if successful, false on failure.
Fails if no file named NAME exists, or if an internal memory
allocation fails. */
pub unsafe fn filesys_remove(name: *const u8) -> bool {
    unsafe {
        let dir = directory::dir_open_root();
        let success = !dir.is_null() && directory::dir_remove(dir, name);
        directory::dir_close(dir);

        success
    }
}

/* Formats the file system. */
unsafe fn do_format() {
    unsafe {
        crate::printf!("Formatting file system...");

        /* Create FAT and save it to the disk. */
        fat::fat_create();

        /* Create the root directory (its inode lives in the root
        cluster). */
        directory::dir_create(fat::cluster_to_sector(fat::ROOT_DIR_CLUSTER), 16);

        fat::fat_close();

        crate::printf!("done.\n");
    }
}
