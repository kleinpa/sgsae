//! An RV32IMC emulator core and a small fantasy console.
//!
//! - [`instructions`] decodes and encodes RV32IM instruction words, and
//!   [`compressed`] expands the RV32C encodings into them.
//! - [`machine`] interprets them against a bus, in cycle-budgeted slices.
//! - [`memory`] is the bus: a slot-indexed address space of ROM, RAM, and
//!   memory-mapped [`memory::Device`]s.
//! - [`elf`] loads a linked cartridge into the bus.
//! - [`devices`], [`input`], [`loader`], and [`midi`] are the memory-mapped
//!   peripherals; [`ppu`] is a paletted bitmap display and [`apu`] the sound
//!   chip.
//! - [`console`] wires all of it into one machine with a fixed memory map,
//!   shared by the CLI and the web build.

pub mod apu;
pub mod compressed;
pub mod console;
pub mod devices;
pub mod elf;
pub mod input;
pub mod instructions;
pub mod loader;
pub mod machine;
pub mod memory;
pub mod midi;
pub mod ppu;
pub mod types;
