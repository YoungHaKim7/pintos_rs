//! Port of `threads/interrupt.c` — interrupt level control.
//!
//! M0 subset: only the intr_disable/intr_set_level/intr_get_level
//! primitives and intr_context() are present.  The IDT/PIC/stub
//! machinery arrives with the threads milestone.

use crate::intrinsic::{cli, read_eflags, sti};

/* Interrupts on/off state (C's `enum intr_level`). */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntrLevel {
    Off,
    On,
}

/* EFLAGS bit for the interrupt flag. */
const FLAG_IF: u64 = 1 << 9;

/// Disables interrupts and returns the previous interrupt state.
#[inline]
pub fn intr_disable() -> IntrLevel {
    let old_level = intr_get_level();
    if old_level == IntrLevel::On {
        unsafe { cli() };
    }
    old_level
}

/// Enables interrupts and returns the previous interrupt state. */
#[inline]
pub fn intr_enable() -> IntrLevel {
    let old_level = intr_get_level();
    if old_level == IntrLevel::Off {
        unsafe { sti() };
    }
    old_level
}

/// Restores the interrupt state to LEVEL. */
#[inline]
pub fn intr_set_level(level: IntrLevel) -> IntrLevel {
    if level == IntrLevel::On {
        intr_enable()
    } else {
        intr_disable()
    }
}

/// Returns the current interrupt state. */
#[inline]
pub fn intr_get_level() -> IntrLevel {
    if unsafe { read_eflags() } & FLAG_IF != 0 {
        IntrLevel::On
    } else {
        IntrLevel::Off
    }
}

/// True if we are running in an external interrupt handler (M0: never).
#[inline]
pub fn intr_context() -> bool {
    // M0: the IDT is not installed, so this cannot happen yet.
    false
}
