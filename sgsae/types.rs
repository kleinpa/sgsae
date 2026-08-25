//! Core scalar types for the machine.
//!
//! This is an RV32 machine: the machine word, the register width, and the
//! address width are all `u32`.

/// A register index in `0..32`. Register 0 is hardwired to zero.
pub type Register = u8;

/// A guest physical address.
pub type Address = u32;

/// The machine word.
pub type Word = u32;
