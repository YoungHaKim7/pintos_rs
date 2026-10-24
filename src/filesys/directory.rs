//! Port of `filesys/directory.c` — directory entries over inodes.

use crate::devices::disk::DiskSector;
use crate::filesys::fat::{ROOT_DIR_CLUSTER, cluster_to_sector};
use crate::filesys::inode::{
    Inode, inode_close, inode_create, inode_open, inode_read_at, inode_remove, inode_reopen,
    inode_write_at,
};
use crate::lib::string::{strcmp, strlcpy, strlen};
use crate::threads::malloc::{calloc, free};

/* Maximum length of a file name component.
This is the traditional UNIX limit; it may be extended in the
extended filesys. */
pub const NAME_MAX: usize = 14;

/* A directory. */
#[repr(C)]
pub struct Dir {
    pub inode: *mut Inode, /* Backing store. */
    pub pos: i32,          /* Current position. */
}

/* A single directory entry. */
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DirEntry {
    pub inode_sector: DiskSector, /* Sector number of header. */
    pub name: [u8; NAME_MAX + 1], /* Null terminated file name. */
    pub in_use: bool,             /* In use or free? */
}

impl DirEntry {
    pub const EMPTY: DirEntry = DirEntry {
        inode_sector: 0,
        name: [0; NAME_MAX + 1],
        in_use: false,
    };
}

/* Creates a directory with space for ENTRY_CNT entries in the
given SECTOR.  Returns true if successful, false on failure. */
pub unsafe fn dir_create(sector: DiskSector, entry_cnt: usize) -> bool {
    unsafe {
        inode_create(
            sector,
            (entry_cnt * core::mem::size_of::<DirEntry>()) as i32,
        )
    }
}

/* Opens and returns the directory for the given INODE, of which
it takes ownership.  Returns a null pointer on failure. */
pub unsafe fn dir_open(inode: *mut Inode) -> *mut Dir {
    unsafe {
        let dir = calloc(1, core::mem::size_of::<Dir>()) as *mut Dir;
        if !inode.is_null() && !dir.is_null() {
            (*dir).inode = inode;
            (*dir).pos = 0;
            dir
        } else {
            inode_close(inode);
            free(dir as *mut u8);
            core::ptr::null_mut()
        }
    }
}

/* Opens the root directory and returns a directory for it.
Return true if successful, false on failure. */
pub unsafe fn dir_open_root() -> *mut Dir {
    unsafe {
        /* EFILESYS: the root directory's inode lives in the root
        cluster, not at a fixed sector. */
        dir_open(inode_open(cluster_to_sector(ROOT_DIR_CLUSTER)))
    }
}

/* Opens and returns a new directory for the same inode as DIR.
Returns a null pointer on failure. */
pub unsafe fn dir_reopen(dir: *mut Dir) -> *mut Dir {
    unsafe { dir_open(inode_reopen((*dir).inode)) }
}

/* Destroys DIR and frees associated resources. */
pub unsafe fn dir_close(dir: *mut Dir) {
    unsafe {
        if !dir.is_null() {
            inode_close((*dir).inode);
            free(dir as *mut u8);
        }
    }
}

/* Returns the inode encapsulated by DIR. */
pub unsafe fn dir_get_inode(dir: *mut Dir) -> *mut Inode {
    unsafe { (*dir).inode }
}

/* Searches DIR for a file with the given NAME.
If successful, returns true, sets *EP to the directory entry if
EP is non-null, and sets *OFSP to the byte offset of the
directory entry if OFSP is non-null.
otherwise, returns false and ignores EP and OFSP. */
unsafe fn lookup(
    dir: *const Dir,
    name: *const u8,
    ep: *mut DirEntry,
    ofsp: *mut i32,
) -> bool {
    unsafe {
        let mut e = DirEntry::EMPTY;
        let entry_size = core::mem::size_of::<DirEntry>() as i32;

        crate::ASSERT!(!dir.is_null());
        crate::ASSERT!(!name.is_null());

        let mut ofs: i32 = 0;
        while inode_read_at((*dir).inode, &raw mut e as *mut DirEntry as *mut u8, entry_size, ofs)
            == entry_size
        {
            if e.in_use && strcmp(name, e.name.as_ptr()) == 0 {
                if !ep.is_null() {
                    *ep = e;
                }
                if !ofsp.is_null() {
                    *ofsp = ofs;
                }
                return true;
            }
            ofs += entry_size;
        }
        false
    }
}

/* Searches DIR for a file with the given NAME and returns true
if one exists, false otherwise.
On success, sets *INODE to an inode for the file, otherwise to a
null pointer.  The caller must close *INODE. */
pub unsafe fn dir_lookup(dir: *const Dir, name: *const u8, inode: *mut *mut Inode) -> bool {
    unsafe {
        let mut e = DirEntry::EMPTY;

        crate::ASSERT!(!dir.is_null());
        crate::ASSERT!(!name.is_null());

        if lookup(dir, name, &raw mut e, core::ptr::null_mut()) {
            *inode = inode_open(e.inode_sector);
        } else {
            *inode = core::ptr::null_mut();
        }

        !(*inode).is_null()
    }
}

/* Adds a file named NAME to DIR, which must not already contain
a file by that name.  The file's inode is in sector
INODE_SECTOR.
Returns true if successful, false on failure.
Fails if NAME is invalid (i.e. too long) or a disk or memory
error occurs. */
pub unsafe fn dir_add(dir: *mut Dir, name: *const u8, inode_sector: DiskSector) -> bool {
    unsafe {
        let mut e = DirEntry::EMPTY;
        let mut ofs: i32 = 0;
        let mut success = false;

        let entry_size = core::mem::size_of::<DirEntry>() as i32;

        crate::ASSERT!(!dir.is_null());
        crate::ASSERT!(!name.is_null());

        /* Check NAME for validity. */
        if *name == 0 || strlen(name) > NAME_MAX {
            return false;
        }

        /* Check that NAME is not in use. */
        if lookup(dir, name, core::ptr::null_mut(), core::ptr::null_mut()) {
            return false;
        }

        /* Set OFS to offset of free slot.
        If there are no free slots, then it will be set to the
        current end-of-file.

        inode_read_at() will only return a short read at end of
        file.  Otherwise, we'd need to verify that we didn't get a
        short read due to something intermittent such as low
        memory. */
        while inode_read_at(
            (*dir).inode,
            &raw mut e as *mut DirEntry as *mut u8,
            entry_size,
            ofs,
        ) == entry_size
        {
            if !e.in_use {
                break;
            }
            ofs += entry_size;
        }

        /* Write slot. */
        e.in_use = true;
        strlcpy(e.name.as_mut_ptr(), name, e.name.len());
        e.inode_sector = inode_sector;
        success = inode_write_at(
            (*dir).inode,
            &raw const e as *const DirEntry as *const u8,
            entry_size,
            ofs,
        ) == entry_size;

        success
    }
}

/* Removes any entry for NAME in DIR.
Returns true if successful, false on failure, which occurs only
if there is no file with the given NAME. */
pub unsafe fn dir_remove(dir: *mut Dir, name: *const u8) -> bool {
    unsafe {
        let mut e = DirEntry::EMPTY;
        let mut inode: *mut Inode = core::ptr::null_mut();
        let mut success = false;
        let mut ofs: i32 = 0;

        let entry_size = core::mem::size_of::<DirEntry>() as i32;

        crate::ASSERT!(!dir.is_null());
        crate::ASSERT!(!name.is_null());

        /* Find directory entry. */
        if !lookup(dir, name, &raw mut e, &raw mut ofs) {
            return false;
        }

        /* Open inode. */
        inode = inode_open(e.inode_sector);
        if inode.is_null() {
            return false;
        }

        /* Erase directory entry. */
        e.in_use = false;
        if inode_write_at(
            (*dir).inode,
            &raw const e as *const DirEntry as *const u8,
            entry_size,
            ofs,
        ) != entry_size
        {
            inode_close(inode);
            return false;
        }

        /* Remove inode. */
        inode_remove(inode);
        success = true;

        inode_close(inode);
        success
    }
}

/* Reads the next directory entry in DIR and stores the name in
NAME.  Returns true if successful, false if the directory
contains no more entries. */
pub unsafe fn dir_readdir(dir: *mut Dir, name: *mut u8) -> bool {
    unsafe {
        let mut e = DirEntry::EMPTY;
        let entry_size = core::mem::size_of::<DirEntry>() as i32;

        while inode_read_at(
            (*dir).inode,
            &raw mut e as *mut DirEntry as *mut u8,
            entry_size,
            (*dir).pos,
        ) == entry_size
        {
            (*dir).pos += entry_size;
            if e.in_use {
                strlcpy(name, e.name.as_ptr(), NAME_MAX + 1);
                return true;
            }
        }
        false
    }
}
