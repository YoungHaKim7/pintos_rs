//! Port of `filesys/file.c` — open-file handles.

use crate::filesys::inode::{
    inode_allow_write, inode_close, inode_deny_write, inode_length, inode_read_at, inode_reopen,
    inode_write_at,
};
use crate::threads::malloc::{calloc, free};

/* An open file. */
#[repr(C)]
pub struct File {
    pub inode: *mut crate::filesys::inode::Inode, /* File's inode. */
    pub pos: i32,                                 /* Current position. */
    pub deny_write: bool, /* Has file_deny_write() been called? */
}

/* Opens a file for the given INODE, of which it takes ownership,
and returns the new file.  Returns a null pointer if an
allocation fails or if INODE is null. */
pub unsafe fn file_open(inode: *mut crate::filesys::inode::Inode) -> *mut File {
    unsafe {
        let file = calloc(1, core::mem::size_of::<File>()) as *mut File;
        if !inode.is_null() && !file.is_null() {
            (*file).inode = inode;
            (*file).pos = 0;
            (*file).deny_write = false;
            file
        } else {
            inode_close(inode);
            free(file as *mut u8);
            core::ptr::null_mut()
        }
    }
}

/* Opens and returns a new file for the same inode as FILE.
Returns a null pointer if unsuccessful. */
pub unsafe fn file_reopen(file: *mut File) -> *mut File {
    unsafe { file_open(inode_reopen((*file).inode)) }
}

/* Duplicate the file object including attributes and returns a
new file for the same inode as FILE.  Returns a null pointer if
unsuccessful. */
pub unsafe fn file_duplicate(file: *mut File) -> *mut File {
    unsafe {
        let nfile = file_open(inode_reopen((*file).inode));
        if !nfile.is_null() {
            (*nfile).pos = (*file).pos;
            if (*file).deny_write {
                file_deny_write(nfile);
            }
        }
        nfile
    }
}

/* Closes FILE. */
pub unsafe fn file_close(file: *mut File) {
    unsafe {
        if !file.is_null() {
            file_allow_write(file);
            inode_close((*file).inode);
            free(file as *mut u8);
        }
    }
}

/* Returns the inode encapsulated by FILE. */
pub unsafe fn file_get_inode(file: *mut File) -> *mut crate::filesys::inode::Inode {
    unsafe { (*file).inode }
}

/* Reads SIZE bytes from FILE into BUFFER, starting at the file's
current position.
Returns the number of bytes actually read, which may be less
than SIZE if end of file is reached.
Advances FILE's position by the number of bytes read. */
pub unsafe fn file_read(file: *mut File, buffer: *mut u8, size: i32) -> i32 {
    unsafe {
        let bytes_read = inode_read_at((*file).inode, buffer, size, (*file).pos);
        (*file).pos += bytes_read;
        bytes_read
    }
}

/* Reads SIZE bytes from FILE into BUFFER, starting at offset
FILE_OFS in the file.
Returns the number of bytes actually read, which may be less
than SIZE if end of file is reached.
The file's current position is unaffected. */
pub unsafe fn file_read_at(file: *mut File, buffer: *mut u8, size: i32, file_ofs: i32) -> i32 {
    unsafe { inode_read_at((*file).inode, buffer, size, file_ofs) }
}

/* Writes SIZE bytes from BUFFER into FILE, starting at the
file's current position.
Returns the number of bytes actually written, which may be less
than SIZE if end of file is reached.
Advances FILE's position by the number of bytes read. */
pub unsafe fn file_write(file: *mut File, buffer: *const u8, size: i32) -> i32 {
    unsafe {
        let bytes_written = inode_write_at((*file).inode, buffer, size, (*file).pos);
        (*file).pos += bytes_written;
        bytes_written
    }
}

/* Writes SIZE bytes from BUFFER into FILE, starting at offset
FILE_OFS in the file.
Returns the number of bytes actually written, which may be less
than SIZE if end of file is reached.
The file's current position is unaffected. */
pub unsafe fn file_write_at(file: *mut File, buffer: *const u8, size: i32, file_ofs: i32) -> i32 {
    unsafe { inode_write_at((*file).inode, buffer, size, file_ofs) }
}

/* Prevents write operations on FILE's underlying inode until
file_allow_write() is called or FILE is closed. */
pub unsafe fn file_deny_write(file: *mut File) {
    unsafe {
        crate::ASSERT!(!file.is_null());
        if !(*file).deny_write {
            (*file).deny_write = true;
            inode_deny_write((*file).inode);
        }
    }
}

/* Re-enables write operations on FILE's underlying inode.
(Writes might still be denied by some other file that has the
same inode open.) */
pub unsafe fn file_allow_write(file: *mut File) {
    unsafe {
        crate::ASSERT!(!file.is_null());
        if (*file).deny_write {
            (*file).deny_write = false;
            inode_allow_write((*file).inode);
        }
    }
}

/* Returns the size of FILE in bytes. */
pub unsafe fn file_length(file: *mut File) -> i32 {
    unsafe {
        crate::ASSERT!(!file.is_null());
        inode_length((*file).inode)
    }
}

/* Sets the current position in FILE to NEW_POS bytes from the
start of the file. */
pub unsafe fn file_seek(file: *mut File, new_pos: i32) {
    unsafe {
        crate::ASSERT!(!file.is_null());
        crate::ASSERT!(new_pos >= 0);
        (*file).pos = new_pos;
    }
}

/* Returns the current position in FILE as a byte offset from the
start of the file. */
pub unsafe fn file_tell(file: *mut File) -> i32 {
    unsafe {
        crate::ASSERT!(!file.is_null());
        (*file).pos
    }
}
