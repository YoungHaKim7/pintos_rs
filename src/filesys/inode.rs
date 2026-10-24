//! Port of `filesys/inode.c` — inode layer over the FAT.
//!
//! The C base kept the original pintos contiguous-allocation
//! scheme; under EFILESYS the data blocks live in FAT cluster
//! chains (one cluster per sector, SECTORS_PER_CLUSTER = 1), so
//! byte_to_sector walks the chain and inode create/close
//! allocate/release chains instead of free-map runs.

use crate::devices::disk::{DISK_SECTOR_SIZE, DiskSector, disk_read, disk_write};
use crate::filesys::FILESYS_DISK;
use crate::filesys::fat::{
    ClusterT, EO_CHAIN, cluster_to_sector, fat_create_chain, fat_get, fat_remove_chain,
    sector_to_cluster,
};
use crate::lib::kernel::list::{
    List, list_begin, list_end, list_entry, list_init, list_next, list_push_front, list_remove,
};
use crate::lib::string::{memcpy, memset};
use crate::threads::malloc::{calloc, free, malloc};

/* Identifies an inode. */
pub const INODE_MAGIC: u32 = 0x494E_4F44;

/* On-disk inode.
Must be exactly DISK_SECTOR_SIZE bytes long. */
#[repr(C)]
pub struct InodeDisk {
    pub start: DiskSector, /* First data sector. */
    pub length: i32,       /* File size in bytes. */
    pub magic: u32,        /* Magic number. */
    pub unused: [u32; 125], /* Not used. */
}

const _: () = assert!(core::mem::size_of::<InodeDisk>() == DISK_SECTOR_SIZE);

/* Returns the number of sectors to allocate for an inode SIZE
bytes long. */
#[inline]
fn bytes_to_sectors(size: i32) -> usize {
    ((size as usize) + DISK_SECTOR_SIZE - 1) / DISK_SECTOR_SIZE
}

/* In-memory inode. */
#[repr(C)]
pub struct Inode {
    pub elem: ListElem,    /* Element in inode list. */
    pub sector: DiskSector, /* Sector number of disk location. */
    pub open_cnt: i32,     /* Number of openers. */
    pub removed: bool,     /* True if deleted, false otherwise. */
    pub deny_write_cnt: i32, /* 0: writes ok, >0: deny writes. */
    pub data: InodeDisk,   /* Inode content. */
}

pub const INODE_ELEM_OFFSET: usize = core::mem::offset_of!(Inode, elem);

/* List of open inodes, so that opening a single inode twice
returns the same `struct inode'. */
static mut OPEN_INODES: List = List::EMPTY;

/* Initializes the inode module. */
pub unsafe fn inode_init() {
    unsafe {
        list_init(&raw mut OPEN_INODES);
    }
}

/* Returns the disk sector that contains byte offset POS within
INODE.
Returns -1 (as an all-ones sector number) if INODE does not
contain data for a byte at offset POS. */
unsafe fn byte_to_sector(inode: *const Inode, pos: i32) -> DiskSector {
    unsafe {
        crate::ASSERT!(!inode.is_null());
        if pos < (*inode).data.length {
            /* Walk the FAT chain: data sectors are cluster-linked,
            not numerically contiguous. */
            let index = pos as usize / DISK_SECTOR_SIZE;
            let mut cluster = sector_to_cluster((*inode).data.start);
            for _ in 0..index {
                cluster = fat_get(cluster);
            }
            cluster_to_sector(cluster)
        } else {
            !0 /* (disk_sector_t) -1 */
        }
    }
}

/* Initializes an inode with LENGTH bytes of data and writes the
new inode to sector SECTOR on the file system disk.
Returns true if successful.
Returns false if memory or disk allocation fails. */
pub unsafe fn inode_create(sector: DiskSector, length: i32) -> bool {
    unsafe {
        let mut success = false;

        crate::ASSERT!(length >= 0);

        let disk_inode = calloc(1, core::mem::size_of::<InodeDisk>()) as *mut InodeDisk;
        if !disk_inode.is_null() {
            let sectors = bytes_to_sectors(length);
            (*disk_inode).length = length;
            (*disk_inode).magic = INODE_MAGIC;

            /* Allocate the data chain cluster by cluster
            (SECTORS_PER_CLUSTER = 1). */
            let mut chain_ok = true;
            let mut prev_clst: ClusterT = 0;
            let mut first_sector: DiskSector = 0;
            for _ in 0..sectors {
                let clst = fat_create_chain(prev_clst);
                if clst == 0 {
                    chain_ok = false;
                    break;
                }
                if prev_clst == 0 {
                    first_sector = cluster_to_sector(clst);
                }
                prev_clst = clst;
            }

            if chain_ok {
                (*disk_inode).start = first_sector;
                disk_write(FILESYS_DISK, sector, disk_inode as *const u8);
                if sectors > 0 {
                    let zeros = calloc(1, DISK_SECTOR_SIZE);
                    if zeros.is_null() {
                        chain_ok = false;
                    } else {
                        /* Zero every data sector of the chain. */
                        let mut c = sector_to_cluster(first_sector);
                        for _ in 0..sectors {
                            disk_write(FILESYS_DISK, cluster_to_sector(c), zeros);
                            c = fat_get(c);
                        }
                        free(zeros);
                    }
                }
                success = chain_ok;
            }

            free(disk_inode as *mut u8);
        }
        success
    }
}

/* Reads an inode from SECTOR and returns a `struct inode' that
contains it.
Returns a null pointer if memory allocation fails. */
pub unsafe fn inode_open(sector: DiskSector) -> *mut Inode {
    unsafe {
        /* Check whether this inode is already open. */
        let mut e = list_begin(&raw const OPEN_INODES);
        while e != list_end(&raw const OPEN_INODES) {
            let inode = list_entry::<Inode>(e, INODE_ELEM_OFFSET);
            if (*inode).sector == sector {
                inode_reopen(inode);
                return inode;
            }
            e = list_next(e);
        }

        /* Allocate memory. */
        let inode = malloc(core::mem::size_of::<Inode>()) as *mut Inode;
        if inode.is_null() {
            return core::ptr::null_mut();
        }

        /* Initialize. */
        list_push_front(&raw mut OPEN_INODES, &raw mut (*inode).elem);
        (*inode).sector = sector;
        (*inode).open_cnt = 1;
        (*inode).deny_write_cnt = 0;
        (*inode).removed = false;
        disk_read(
            FILESYS_DISK,
            (*inode).sector,
            &raw mut (*inode).data as *mut InodeDisk as *mut u8,
        );
        inode
    }
}

/* Reopens and returns INODE. */
pub unsafe fn inode_reopen(inode: *mut Inode) -> *mut Inode {
    unsafe {
        if !inode.is_null() {
            (*inode).open_cnt += 1;
        }
        inode
    }
}

/* Returns INODE's inode number. */
pub unsafe fn inode_get_inumber(inode: *const Inode) -> DiskSector {
    unsafe { (*inode).sector }
}

/* Closes INODE and writes it to disk.
If this was the last reference to INODE, frees its memory.
If INODE was also a removed inode, frees its blocks. */
pub unsafe fn inode_close(inode: *mut Inode) {
    unsafe {
        /* Ignore null pointer. */
        if inode.is_null() {
            return;
        }

        /* Release resources if this was the last opener. */
        (*inode).open_cnt -= 1;
        if (*inode).open_cnt == 0 {
            /* Remove from inode list. */
            list_remove(&raw mut (*inode).elem);

            /* Deallocate clusters if removed: the inode's own
            sector and its data chain. */
            if (*inode).removed {
                fat_remove_chain(sector_to_cluster((*inode).sector), 0);
                if (*inode).data.start != 0 {
                    fat_remove_chain(sector_to_cluster((*inode).data.start), 0);
                }
            }

            free(inode as *mut u8);
        }
    }
}

/* Marks INODE to be deleted when it is closed by the last caller
who has it open. */
pub unsafe fn inode_remove(inode: *mut Inode) {
    unsafe {
        crate::ASSERT!(!inode.is_null());
        (*inode).removed = true;
    }
}

/* Reads SIZE bytes from INODE into BUFFER, starting at position
OFFSET.
Returns the number of bytes actually read, which may be less
than SIZE if an error occurs or end of file is reached. */
pub unsafe fn inode_read_at(
    inode: *mut Inode,
    buffer_: *mut u8,
    size: i32,
    offset: i32,
) -> i32 {
    unsafe {
        let buffer = buffer_;
        let mut bytes_read: i32 = 0;
        let mut size = size;
        let mut offset = offset;
        let mut bounce: *mut u8 = core::ptr::null_mut();

        while size > 0 {
            /* Disk sector to read, starting byte offset within
            sector. */
            let sector_idx = byte_to_sector(inode, offset);
            let sector_ofs = offset as usize % DISK_SECTOR_SIZE;

            /* Bytes left in inode, bytes left in sector, lesser
            of the two. */
            let inode_left = inode_length(inode) - offset;
            let sector_left = (DISK_SECTOR_SIZE - sector_ofs) as i32;
            let min_left = if inode_left < sector_left { inode_left } else { sector_left };

            /* Number of bytes to actually copy out of this
            sector. */
            let chunk_size = if size < min_left { size } else { min_left };
            if chunk_size <= 0 {
                break;
            }

            if sector_ofs == 0 && chunk_size as usize == DISK_SECTOR_SIZE {
                /* Read full sector directly into caller's
                buffer. */
                disk_read(FILESYS_DISK, sector_idx, buffer.add(bytes_read as usize));
            } else {
                /* Read sector into bounce buffer, then partially
                copy into caller's buffer. */
                if bounce.is_null() {
                    bounce = malloc(DISK_SECTOR_SIZE);
                    if bounce.is_null() {
                        break;
                    }
                }
                disk_read(FILESYS_DISK, sector_idx, bounce);
                memcpy(
                    buffer.add(bytes_read as usize),
                    bounce.add(sector_ofs),
                    chunk_size as usize,
                );
            }

            /* Advance. */
            size -= chunk_size;
            offset += chunk_size;
            bytes_read += chunk_size;
        }
        free(bounce);

        bytes_read
    }
}

/* Writes SIZE bytes from BUFFER into INODE, starting at OFFSET.
Returns the number of bytes actually written, which may be less
than SIZE if end of file is reached or an error occurs. */
pub unsafe fn inode_write_at(
    inode: *mut Inode,
    buffer_: *const u8,
    size: i32,
    offset: i32,
) -> i32 {
    unsafe {
        let buffer = buffer_;
        let mut bytes_written: i32 = 0;
        let mut size = size;
        let mut offset = offset;
        let mut bounce: *mut u8 = core::ptr::null_mut();

        if (*inode).deny_write_cnt != 0 {
            return 0;
        }

        while size > 0 {
            /* Sector to write, starting byte offset within
            sector. */
            let sector_idx = byte_to_sector(inode, offset);
            let sector_ofs = offset as usize % DISK_SECTOR_SIZE;

            let inode_left = inode_length(inode) - offset;
            let sector_left = (DISK_SECTOR_SIZE - sector_ofs) as i32;
            let min_left = if inode_left < sector_left { inode_left } else { sector_left };

            let chunk_size = if size < min_left { size } else { min_left };
            if chunk_size <= 0 {
                break;
            }

            if sector_ofs == 0 && chunk_size as usize == DISK_SECTOR_SIZE {
                /* Write full sector directly to disk. */
                disk_write(FILESYS_DISK, sector_idx, buffer.add(bytes_written as usize));
            } else {
                /* We need a bounce buffer. */
                if bounce.is_null() {
                    bounce = malloc(DISK_SECTOR_SIZE);
                    if bounce.is_null() {
                        break;
                    }
                }

                /* If the sector contains data before or after the
                chunk we're writing, then we need to read in the
                sector first.  Otherwise we start with a sector of
                all zeros. */
                if sector_ofs > 0 || (chunk_size as usize) < sector_left as usize {
                    disk_read(FILESYS_DISK, sector_idx, bounce);
                } else {
                    memset(bounce, 0, DISK_SECTOR_SIZE);
                }
                memcpy(
                    bounce.add(sector_ofs),
                    buffer.add(bytes_written as usize),
                    chunk_size as usize,
                );
                disk_write(FILESYS_DISK, sector_idx, bounce);
            }

            /* Advance. */
            size -= chunk_size;
            offset += chunk_size;
            bytes_written += chunk_size;
        }
        free(bounce);

        bytes_written
    }
}

/* Disables writes to INODE.
May be called at most once per inode opener. */
pub unsafe fn inode_deny_write(inode: *mut Inode) {
    unsafe {
        (*inode).deny_write_cnt += 1;
        crate::ASSERT!((*inode).deny_write_cnt <= (*inode).open_cnt);
    }
}

/* Re-enables writes to INODE.
Must be called once by each inode opener who has called
inode_deny_write() on the inode, before closing the inode. */
pub unsafe fn inode_allow_write(inode: *mut Inode) {
    unsafe {
        crate::ASSERT!((*inode).deny_write_cnt > 0);
        crate::ASSERT!((*inode).deny_write_cnt <= (*inode).open_cnt);
        (*inode).deny_write_cnt -= 1;
    }
}

/* Returns the length, in bytes, of INODE's data. */
pub unsafe fn inode_length(inode: *const Inode) -> i32 {
    unsafe { (*inode).data.length }
}
