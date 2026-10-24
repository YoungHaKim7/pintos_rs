//! Port of `filesys/fat.c` — FAT filesystem cluster management.
//!
//! The C skeleton left `fat_fs_init`, `fat_create_chain`,
//! `fat_remove_chain`, `fat_put`, `fat_get` and
//! `cluster_to_sector` as student TODOs; they are implemented
//! here per the project spec (KAIST EFILESYS).

use crate::devices::disk::{
    DISK_SECTOR_SIZE, DiskSector, disk_read, disk_size, disk_write,
};
use crate::filesys::FILESYS_DISK;
use crate::lib::string::memcpy;
use crate::threads::malloc::{calloc, free, malloc};
use crate::threads::synch::{Lock, lock_acquire, lock_init, lock_release};

/* Index of a cluster within the FAT. */
pub type ClusterT = u32;

pub const FAT_MAGIC: u32 = 0xEB3C_9000; /* MAGIC string to identify FAT disk */
pub const EO_CHAIN: u32 = 0x0FFF_FFFF; /* End of cluster chain */

/* Sectors of FAT information. */
pub const SECTORS_PER_CLUSTER: u32 = 1; /* Number of sectors per cluster */
pub const FAT_BOOT_SECTOR: u32 = 0; /* FAT boot sector. */
pub const ROOT_DIR_CLUSTER: u32 = 1; /* Cluster for the root directory */

/* Should be less than DISK_SECTOR_SIZE. */
#[repr(C)]
pub struct FatBoot {
    pub magic: u32,
    pub sectors_per_cluster: u32, /* Fixed to 1 */
    pub total_sectors: u32,
    pub fat_start: u32,
    pub fat_sectors: u32, /* Size of FAT in sectors. */
    pub root_dir_cluster: u32,
}

/* FAT FS. */
#[repr(C)]
pub struct FatFs {
    pub bs: FatBoot,
    pub fat: *mut ClusterT,
    pub fat_length: u32,
    pub data_start: DiskSector,
    pub last_clst: ClusterT,
    pub write_lock: Lock,
}

impl FatFs {
    /// All-zero FAT state (call `fat_init` before use).
    pub const EMPTY: FatFs = FatFs {
        bs: FatBoot {
            magic: 0,
            sectors_per_cluster: 0,
            total_sectors: 0,
            fat_start: 0,
            fat_sectors: 0,
            root_dir_cluster: 0,
        },
        fat: core::ptr::null_mut(),
        fat_length: 0,
        data_start: 0,
        last_clst: 0,
        write_lock: Lock::EMPTY,
    };
}

static mut FAT_FS: *mut FatFs = core::ptr::null_mut();

/* The single FAT state, for the rest of the filesys layer. */
#[inline]
pub unsafe fn fat_fs() -> *mut FatFs {
    unsafe { FAT_FS }
}

pub unsafe fn fat_init() {
    unsafe {
        let fat_fs = calloc(1, core::mem::size_of::<FatFs>()) as *mut FatFs;
        if fat_fs.is_null() {
            crate::PANIC!("FAT init failed");
        }
        FAT_FS = fat_fs;

        /* Read boot sector from the disk. */
        let bounce = malloc(DISK_SECTOR_SIZE);
        if bounce.is_null() {
            crate::PANIC!("FAT init failed");
        }
        disk_read(FILESYS_DISK, FAT_BOOT_SECTOR as DiskSector, bounce);
        memcpy(
            &raw mut (*fat_fs).bs as *mut u8,
            bounce,
            core::mem::size_of::<FatBoot>(),
        );
        free(bounce);

        /* Extract FAT info. */
        if (*fat_fs).bs.magic != FAT_MAGIC {
            fat_boot_create();
        }
        fat_fs_init();
    }
}

pub unsafe fn fat_open() {
    unsafe {
        let fat_fs = FAT_FS;
        (*fat_fs).fat = calloc((*fat_fs).fat_length as usize, core::mem::size_of::<ClusterT>())
            as *mut ClusterT;
        if (*fat_fs).fat.is_null() {
            crate::PANIC!("FAT load failed");
        }

        /* Load FAT directly from the disk. */
        let buffer = (*fat_fs).fat as *mut u8;
        let mut bytes_read: usize = 0;
        let fat_size_in_bytes =
            (*fat_fs).fat_length as usize * core::mem::size_of::<ClusterT>();
        for i in 0..(*fat_fs).bs.fat_sectors {
            let bytes_left = fat_size_in_bytes.saturating_sub(bytes_read);
            if bytes_left >= DISK_SECTOR_SIZE {
                disk_read(
                    FILESYS_DISK,
                    ((*fat_fs).bs.fat_start + i) as DiskSector,
                    buffer.add(bytes_read),
                );
                bytes_read += DISK_SECTOR_SIZE;
            } else {
                let bounce = malloc(DISK_SECTOR_SIZE);
                if bounce.is_null() {
                    crate::PANIC!("FAT load failed");
                }
                disk_read(FILESYS_DISK, ((*fat_fs).bs.fat_start + i) as DiskSector, bounce);
                memcpy(buffer.add(bytes_read), bounce, bytes_left);
                bytes_read += bytes_left;
                free(bounce);
            }
        }
    }
}

pub unsafe fn fat_close() {
    unsafe {
        let fat_fs = FAT_FS;

        /* Write FAT boot sector. */
        let bounce = calloc(1, DISK_SECTOR_SIZE);
        if bounce.is_null() {
            crate::PANIC!("FAT close failed");
        }
        memcpy(bounce, &raw const (*fat_fs).bs as *const u8, core::mem::size_of::<FatBoot>());
        disk_write(FILESYS_DISK, FAT_BOOT_SECTOR as DiskSector, bounce);
        free(bounce);

        /* Write FAT directly to the disk. */
        let buffer = (*fat_fs).fat as *mut u8;
        let mut bytes_wrote: usize = 0;
        let fat_size_in_bytes =
            (*fat_fs).fat_length as usize * core::mem::size_of::<ClusterT>();
        for i in 0..(*fat_fs).bs.fat_sectors {
            let bytes_left = fat_size_in_bytes.saturating_sub(bytes_wrote);
            if bytes_left >= DISK_SECTOR_SIZE {
                disk_write(
                    FILESYS_DISK,
                    ((*fat_fs).bs.fat_start + i) as DiskSector,
                    buffer.add(bytes_wrote),
                );
                bytes_wrote += DISK_SECTOR_SIZE;
            } else {
                let bounce = calloc(1, DISK_SECTOR_SIZE);
                if bounce.is_null() {
                    crate::PANIC!("FAT close failed");
                }
                memcpy(bounce, buffer.add(bytes_wrote), bytes_left);
                disk_write(FILESYS_DISK, ((*fat_fs).bs.fat_start + i) as DiskSector, bounce);
                bytes_wrote += bytes_left;
                free(bounce);
            }
        }
    }
}

pub unsafe fn fat_create() {
    unsafe {
        let fat_fs = FAT_FS;

        /* Create FAT boot. */
        fat_boot_create();
        fat_fs_init();

        /* Create FAT table. */
        (*fat_fs).fat =
            calloc((*fat_fs).fat_length as usize, core::mem::size_of::<ClusterT>()) as *mut ClusterT;
        if (*fat_fs).fat.is_null() {
            crate::PANIC!("FAT creation failed");
        }

        /* Set up ROOT_DIR_CLST. */
        fat_put(ROOT_DIR_CLUSTER, EO_CHAIN);

        /* Fill up ROOT_DIR_CLUSTER region with 0. */
        let buf = calloc(1, DISK_SECTOR_SIZE);
        if buf.is_null() {
            crate::PANIC!("FAT create failed due to OOM");
        }
        disk_write(
            FILESYS_DISK,
            cluster_to_sector(ROOT_DIR_CLUSTER),
            buf,
        );
        free(buf);
    }
}

pub unsafe fn fat_boot_create() {
    unsafe {
        let fat_fs = FAT_FS;
        let disk_sectors = disk_size(FILESYS_DISK);
        let fat_sectors = (disk_sectors - 1)
            / (DISK_SECTOR_SIZE as u32 / core::mem::size_of::<ClusterT>() as u32
                * SECTORS_PER_CLUSTER
                + 1)
            + 1;
        (*fat_fs).bs = FatBoot {
            magic: FAT_MAGIC,
            sectors_per_cluster: SECTORS_PER_CLUSTER,
            total_sectors: disk_sectors,
            fat_start: 1,
            fat_sectors,
            root_dir_cluster: ROOT_DIR_CLUSTER,
        };
    }
}

pub unsafe fn fat_fs_init() {
    unsafe {
        let fat_fs = FAT_FS;
        /* Clusters 1..=fat_length-1 cover the data region after
        the boot sector and the FAT itself; cluster 1 is the root
        directory. */
        (*fat_fs).fat_length = ((*fat_fs).bs.total_sectors
            - ((*fat_fs).bs.fat_start + (*fat_fs).bs.fat_sectors))
            / SECTORS_PER_CLUSTER
            + 1;
        (*fat_fs).data_start = (*fat_fs).bs.fat_start + (*fat_fs).bs.fat_sectors;
        /* Nothing allocated yet: scan starts after the root. */
        (*fat_fs).last_clst = ROOT_DIR_CLUSTER;
        lock_init(&raw mut (*fat_fs).write_lock);
    }
}

/* ------------------------------------------------------------------------ */
/* FAT handling                                                              */
/* ------------------------------------------------------------------------ */

/* Add a cluster to the chain.
If CLST is 0, start a new chain.
Returns 0 if fails to allocate a new cluster. */
pub unsafe fn fat_create_chain(clst: ClusterT) -> ClusterT {
    unsafe {
        let fat_fs = FAT_FS;

        lock_acquire(&raw mut (*fat_fs).write_lock);

        /* Find a free cluster, scanning forward from the last
        allocation (with wraparound past the root). */
        let n = (*fat_fs).fat_length;
        let mut c = (*fat_fs).last_clst + 1;
        let mut new_clst: ClusterT = 0;
        let mut tried: u32 = 0;
        while tried < n {
            if c >= n {
                c = ROOT_DIR_CLUSTER + 1;
            }
            if c == EO_CHAIN {
                break;
            }
            if fat_get(c) == 0 {
                new_clst = c;
                break;
            }
            c += 1;
            tried += 1;
        }

        if new_clst != 0 {
            /* Set new value. */
            fat_put(new_clst, EO_CHAIN);
            if clst != 0 {
                fat_put(clst, new_clst);
            }
            (*fat_fs).last_clst = new_clst;
        }

        lock_release(&raw mut (*fat_fs).write_lock);
        new_clst
    }
}

/* Remove the chain of clusters starting from CLST.
If PCLST is 0, assume CLST as the start of the chain. */
pub unsafe fn fat_remove_chain(clst: ClusterT, pclst: ClusterT) {
    unsafe {
        let fat_fs = FAT_FS;

        lock_acquire(&raw mut (*fat_fs).write_lock);

        /* Zero every cluster in the chain. */
        let mut c = clst;
        while c != EO_CHAIN {
            let next = fat_get(c);
            fat_put(c, 0);
            c = next;
        }

        /* The previous cluster, if any, ends the chain now. */
        if pclst != 0 {
            fat_put(pclst, EO_CHAIN);
        }

        lock_release(&raw mut (*fat_fs).write_lock);
    }
}

/* Update a value in the FAT table. */
pub unsafe fn fat_put(clst: ClusterT, val: ClusterT) {
    unsafe {
        *(*FAT_FS).fat.add(clst as usize) = val;
    }
}

/* Fetch a value in the FAT table. */
pub unsafe fn fat_get(clst: ClusterT) -> ClusterT {
    unsafe { *(*FAT_FS).fat.add(clst as usize) }
}

/* Convert a cluster # to a sector number. */
pub unsafe fn cluster_to_sector(clst: ClusterT) -> DiskSector {
    unsafe {
        (*FAT_FS).data_start + (clst - 1) * SECTORS_PER_CLUSTER
    }
}

/* Convert a sector number back to its cluster # (inverse of
cluster_to_sector; used by the inode layer). */
pub unsafe fn sector_to_cluster(sector: DiskSector) -> ClusterT {
    unsafe { (sector - (*FAT_FS).data_start) / SECTORS_PER_CLUSTER + 1 }
}
