//! Port of `filesys/` — file system (project 4).
//!
//! M1 stubs: the threads tests run without a filesystem disk; the
//! real FAT port is the filesys milestone.

/// Reads/formats the filesystem.  M1: no-op (no FAT port yet; safe
/// to call with or without an fs disk attached).
pub unsafe fn filesys_init(_format: bool) {
    unsafe {}
}

/// Flushes and closes the filesystem.  M1: no-op.
pub unsafe fn filesys_done() {}
