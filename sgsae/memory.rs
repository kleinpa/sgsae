//! The system bus: a flat 32-bit address space carved into regions.
//!
//! The model is deliberately small. There are two kinds of region — memory,
//! which may be read-only, and a [`Device`] — and one dispatch rule: the top
//! byte of an address selects a 16 MiB *slot*, and each slot names at most one
//! region. Lookup is a single array index, so a load or store costs the same
//! whether the map has two regions or twenty.
//!
//! ```text
//!   address    0xSS_RR_RR_RR
//!               ^^ slot index (0..256), 16 MiB apart
//! ```
//!
//! A region may span several consecutive slots (a 32 MiB cartridge occupies
//! two), but every region must *start* on a slot boundary. That restriction is
//! what keeps the dispatch to one indexing operation, and it matches how
//! cartridge-era consoles laid out their memory anyway.
//!
//! # Memory-mapped I/O
//!
//! Anything that is not plain memory implements [`Device`]. A device thinks in
//! bytes: the bus splits wider accesses into byte accesses for it, and a device
//! that needs a register read atomically at its natural width overrides
//! [`Device::read`] for that one case. Reads take `&mut self`, so a register
//! may have read side effects — draining a FIFO, clearing a latch of button
//! edges.
//!
//! Devices never fault. As on real hardware, a write to a read-only register
//! is ignored and a read from a gap returns zero. The bus enforces the one
//! limit that matters, the device's declared [`Device::size`].
//!
//! Devices are mapped as `Rc<RefCell<T>>`, so the host keeps a handle after
//! handing one to the bus. That is the seam for host integration — a gamepad
//! pushing state *in*, a text sink being drained *out*:
//!
//! ```
//! use std::cell::RefCell;
//! use std::rc::Rc;
//! use sgsae::input::{Button, Input, Pad};
//! use sgsae::memory::{Bus, Width};
//!
//! let input = Rc::new(RefCell::new(Input::default()));
//! let mut bus = Bus::new();
//! bus.map_device(0x0500_0000, input.clone()).unwrap();
//!
//! // …later, from a host input event:
//! input.borrow_mut().set_pad(0, Pad { buttons: Button::A.mask(), connected: true, ..Pad::default() });
//! assert_eq!(bus.read(0x0500_0010, Width::Half).unwrap(), u32::from(Button::A.mask()));
//! ```

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use crate::types::Address;

/// Address bits consumed by the offset within a slot.
pub const SLOT_BITS: u32 = 24;

/// Size of one address-space slot: 16 MiB.
pub const SLOT_SIZE: u32 = 1 << SLOT_BITS;

/// Number of slots in the 32-bit address space.
pub const SLOT_COUNT: usize = 1 << (32 - SLOT_BITS);

/// The width of a single bus access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    Byte,
    Half,
    Word,
}

impl Width {
    /// Number of bytes this access touches.
    pub const fn bytes(self) -> u32 {
        match self {
            Width::Byte => 1,
            Width::Half => 2,
            Width::Word => 4,
        }
    }
}

/// Why a bus access could not be completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// No region is mapped at this address, or the access ran off the end of one.
    Unmapped(Address),
    /// The address is in a read-only region.
    ReadOnly(Address),
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Fault::Unmapped(a) => write!(f, "unmapped address {a:#010x}"),
            Fault::ReadOnly(a) => write!(f, "write to read-only address {a:#010x}"),
        }
    }
}

/// Why a region could not be mapped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapError {
    /// `base` is not a multiple of [`SLOT_SIZE`].
    UnalignedBase(Address),
    /// The region is empty, or would run off the end of the address space.
    BadSize(Address, u64),
    /// Another region already claims one of the slots this one needs.
    Occupied(Address),
    /// More than 255 regions were mapped.
    TooManyRegions,
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            MapError::UnalignedBase(a) => {
                write!(f, "base {a:#010x} is not aligned to {SLOT_SIZE:#x}")
            }
            MapError::BadSize(a, n) => write!(f, "bad size {n} for region at {a:#010x}"),
            MapError::Occupied(a) => write!(f, "a region is already mapped over {a:#010x}"),
            MapError::TooManyRegions => write!(f, "too many regions (max 255)"),
        }
    }
}

impl std::error::Error for Fault {}
impl std::error::Error for MapError {}

/// Assemble a little-endian value `width` bytes wide from `byte_at(i)`.
pub fn read_le(width: Width, mut byte_at: impl FnMut(u32) -> u8) -> u32 {
    let mut value = 0;
    for i in 0..width.bytes() {
        value |= u32::from(byte_at(i)) << (8 * i);
    }
    value
}

/// Something that behaves like memory but isn't.
///
/// Offsets are relative to the device's base address. Values are passed and
/// returned zero-extended in a `u32`; the CPU applies sign extension for the
/// signed load instructions, so a device never has to think about it.
pub trait Device {
    /// Bytes of address space the device occupies. The bus refuses anything
    /// past it, so nothing inside the device needs to.
    fn size(&self) -> u32;

    /// Read one byte. Takes `&mut self` so reads may latch, pop, or otherwise
    /// have side effects.
    fn read_byte(&mut self, offset: u32) -> u8;

    /// Write one byte. Read-only registers ignore it.
    fn write_byte(&mut self, offset: u32, value: u8);

    /// Read `width` bytes at `offset`. Byte-wise by default; override for a
    /// register that must be taken whole.
    fn read(&mut self, offset: u32, width: Width) -> u32 {
        read_le(width, |i| self.read_byte(offset + i))
    }

    /// Write the low `width` bytes of `value` at `offset`.
    fn write(&mut self, offset: u32, width: Width, value: u32) {
        for i in 0..width.bytes() {
            self.write_byte(offset + i, (value >> (8 * i)) as u8);
        }
    }
}

enum Region {
    Memory { base: Address, data: Box<[u8]>, writable: bool },
    Device { base: Address, size: u32, device: Rc<RefCell<dyn Device>> },
}

/// The system bus.
pub struct Bus {
    /// `slots[addr >> SLOT_BITS]` indexes into `regions`.
    slots: [Option<u8>; SLOT_COUNT],
    regions: Vec<Region>,
}

impl Bus {
    /// An empty address space: every access faults until something is mapped.
    pub fn new() -> Bus {
        Bus { slots: [None; SLOT_COUNT], regions: Vec::new() }
    }

    /// Map read-only memory. Writes to it fault with [`Fault::ReadOnly`].
    pub fn map_rom(&mut self, base: Address, data: impl Into<Box<[u8]>>) -> Result<(), MapError> {
        self.map_memory(base, data.into(), false)
    }

    /// Map `size` bytes of zeroed read-write memory.
    pub fn map_ram(&mut self, base: Address, size: usize) -> Result<(), MapError> {
        self.map_memory(base, vec![0; size].into_boxed_slice(), true)
    }

    fn map_memory(&mut self, base: Address, data: Box<[u8]>, writable: bool) -> Result<(), MapError> {
        self.claim(base, data.len() as u64)?;
        self.regions.push(Region::Memory { base, data, writable });
        Ok(())
    }

    /// Map a [`Device`] over as many bytes as it says it occupies.
    pub fn map_device(
        &mut self,
        base: Address,
        device: Rc<RefCell<dyn Device>>,
    ) -> Result<(), MapError> {
        let size = device.borrow().size();
        self.claim(base, u64::from(size))?;
        self.regions.push(Region::Device { base, size, device });
        Ok(())
    }

    /// Validate a placement and reserve its slots for the region that is about
    /// to be pushed.
    fn claim(&mut self, base: Address, size: u64) -> Result<(), MapError> {
        if base % SLOT_SIZE != 0 {
            return Err(MapError::UnalignedBase(base));
        }
        if size == 0 || u64::from(base) + size > 1u64 << 32 {
            return Err(MapError::BadSize(base, size));
        }
        let index = u8::try_from(self.regions.len()).map_err(|_| MapError::TooManyRegions)?;

        let first = (base >> SLOT_BITS) as usize;
        let slots = size.div_ceil(u64::from(SLOT_SIZE)) as usize;
        for slot in first..first + slots {
            if self.slots[slot].is_some() {
                return Err(MapError::Occupied((slot as u32) << SLOT_BITS));
            }
        }
        for slot in first..first + slots {
            self.slots[slot] = Some(index);
        }
        Ok(())
    }

    fn region_of(&self, addr: Address) -> Option<usize> {
        self.slots[(addr >> SLOT_BITS) as usize].map(usize::from)
    }

    /// Read `width` bytes at `addr`, zero-extended into a `u32`.
    ///
    /// Unaligned accesses to memory are permitted and are assembled
    /// byte-by-byte; a device sees the offset and width and does as it likes.
    pub fn read(&self, addr: Address, width: Width) -> Result<u32, Fault> {
        let Some(index) = self.region_of(addr) else {
            return Err(Fault::Unmapped(addr));
        };
        match &self.regions[index] {
            Region::Memory { base, data, .. } => {
                let bytes = slice_at(data, addr - base, width).ok_or(Fault::Unmapped(addr))?;
                Ok(read_le(width, |i| bytes[i as usize]))
            }
            Region::Device { base, size, device } => {
                let offset = addr - base;
                if offset + width.bytes() > *size {
                    return Err(Fault::Unmapped(addr));
                }
                Ok(device.borrow_mut().read(offset, width))
            }
        }
    }

    /// Write the low `width` bytes of `value` at `addr`.
    pub fn write(&mut self, addr: Address, width: Width, value: u32) -> Result<(), Fault> {
        let Some(index) = self.region_of(addr) else {
            return Err(Fault::Unmapped(addr));
        };
        match &mut self.regions[index] {
            Region::Memory { writable: false, .. } => Err(Fault::ReadOnly(addr)),
            Region::Memory { base, data, .. } => {
                let n = width.bytes() as usize;
                let start = (addr - *base) as usize;
                let bytes = start
                    .checked_add(n)
                    .and_then(|end| data.get_mut(start..end))
                    .ok_or(Fault::Unmapped(addr))?;
                bytes.copy_from_slice(&value.to_le_bytes()[..n]);
                Ok(())
            }
            Region::Device { base, size, device } => {
                let offset = addr - *base;
                if offset + width.bytes() > *size {
                    return Err(Fault::Unmapped(addr));
                }
                device.borrow_mut().write(offset, width, value);
                Ok(())
            }
        }
    }

    /// Borrow memory from `addr` to the end of its region.
    ///
    /// This is the zero-copy path: instruction fetch reads through it, and so
    /// does the display. Returns `None` for unmapped addresses and for device
    /// regions.
    pub fn bytes_at(&self, addr: Address) -> Option<&[u8]> {
        match self.regions.get(self.region_of(addr)?)? {
            Region::Memory { base, data, .. } => data.get((addr - base) as usize..),
            Region::Device { .. } => None,
        }
    }

    /// Writable view of `len` bytes of memory at `addr`, ignoring write
    /// protection. Only the loader gets this.
    fn image_mut(&mut self, addr: Address, len: usize) -> Result<&mut [u8], Fault> {
        let index = self.region_of(addr).ok_or(Fault::Unmapped(addr))?;
        let Region::Memory { base, data, .. } = &mut self.regions[index] else {
            return Err(Fault::Unmapped(addr));
        };
        let start = (addr - *base) as usize;
        start
            .checked_add(len)
            .and_then(|end| data.get_mut(start..end))
            .ok_or(Fault::Unmapped(addr))
    }

    /// Copy `data` into memory at `addr` as a loader would, ignoring write
    /// protection.
    ///
    /// A cartridge's code belongs in ROM, and something has to put it there.
    /// This is that something — it bypasses the read-only check that guest
    /// stores obey. Device regions are not memory and refuse it.
    pub fn load(&mut self, addr: Address, data: &[u8]) -> Result<(), Fault> {
        self.image_mut(addr, data.len())?.copy_from_slice(data);
        Ok(())
    }

    /// Set `len` bytes of memory at `addr` to `byte`, ignoring write
    /// protection. How a loader clears `.bss`.
    pub fn fill(&mut self, addr: Address, len: u32, byte: u8) -> Result<(), Fault> {
        self.image_mut(addr, len as usize)?.fill(byte);
        Ok(())
    }
}

/// Borrow exactly `width` bytes at `offset`, if they are all in bounds.
fn slice_at(data: &[u8], offset: u32, width: Width) -> Option<&[u8]> {
    let start = offset as usize;
    let end = start.checked_add(width.bytes() as usize)?;
    data.get(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A device that records every access it sees.
    #[derive(Default)]
    struct Recorder {
        log: Vec<(u32, Width, Option<u32>)>,
        next_read: u32,
    }

    impl Device for Recorder {
        fn size(&self) -> u32 {
            0x100
        }
        fn read_byte(&mut self, _: u32) -> u8 {
            unreachable!("reads are taken whole")
        }
        fn write_byte(&mut self, _: u32, _: u8) {
            unreachable!("writes are taken whole")
        }
        fn read(&mut self, offset: u32, width: Width) -> u32 {
            self.log.push((offset, width, None));
            self.next_read += 1;
            self.next_read
        }
        fn write(&mut self, offset: u32, width: Width, value: u32) {
            self.log.push((offset, width, Some(value)));
        }
    }

    fn bus() -> Bus {
        let mut bus = Bus::new();
        bus.map_rom(0x0000_0000, vec![0xA0, 0xA1, 0xA2, 0xA3]).unwrap();
        bus.map_ram(0x0200_0000, 64).unwrap();
        bus
    }

    #[test]
    fn reads_each_width_little_endian() {
        let b = bus();
        assert_eq!(b.read(0, Width::Byte).unwrap(), 0xA0);
        assert_eq!(b.read(0, Width::Half).unwrap(), 0xA1A0);
        assert_eq!(b.read(0, Width::Word).unwrap(), 0xA3A2A1A0);
    }

    #[test]
    fn unaligned_memory_access_is_allowed() {
        let mut b = bus();
        assert_eq!(b.read(1, Width::Half).unwrap(), 0xA2A1);
        b.write(0x0200_0001, Width::Word, 0xDEAD_BEEF).unwrap();
        assert_eq!(b.read(0x0200_0001, Width::Word).unwrap(), 0xDEAD_BEEF);
        // …and did not disturb its neighbours.
        assert_eq!(b.read(0x0200_0000, Width::Byte).unwrap(), 0);
        assert_eq!(b.read(0x0200_0005, Width::Byte).unwrap(), 0);
    }

    #[test]
    fn access_running_past_the_end_of_a_region_faults() {
        let mut b = bus();
        // The ROM is 4 bytes long but its slot is 16 MiB.
        assert_eq!(b.read(2, Width::Word), Err(Fault::Unmapped(2)));
        assert_eq!(b.read(4, Width::Byte), Err(Fault::Unmapped(4)));
        assert_eq!(b.write(0x0200_003E, Width::Word, 0), Err(Fault::Unmapped(0x0200_003E)));
    }

    #[test]
    fn rom_rejects_writes_and_gaps_fault() {
        let mut b = bus();
        assert_eq!(b.write(0, Width::Byte, 1), Err(Fault::ReadOnly(0)));
        assert_eq!(b.read(0x0100_0000, Width::Byte), Err(Fault::Unmapped(0x0100_0000)));
    }

    #[test]
    fn ram_round_trips() {
        let mut b = bus();
        b.write(0x0200_0010, Width::Word, 0x1234_5678).unwrap();
        assert_eq!(b.read(0x0200_0010, Width::Word).unwrap(), 0x1234_5678);
        assert_eq!(b.read(0x0200_0010, Width::Byte).unwrap(), 0x78);
        assert_eq!(b.read(0x0200_0013, Width::Byte).unwrap(), 0x12);
    }

    #[test]
    fn a_region_may_span_several_slots() {
        let mut b = Bus::new();
        b.map_ram(0x0000_0000, (SLOT_SIZE as usize) + 16).unwrap();
        assert!(b.write(0x0100_000F, Width::Byte, 0x5A).is_ok());
        assert_eq!(b.read(0x0100_000F, Width::Byte).unwrap(), 0x5A);
        assert_eq!(b.read(0x0100_0010, Width::Byte), Err(Fault::Unmapped(0x0100_0010)));
        // The second slot is taken, so nothing else may claim it.
        assert_eq!(b.map_ram(0x0100_0000, 16), Err(MapError::Occupied(0x0100_0000)));
    }

    #[test]
    fn mapping_rejects_bad_placements() {
        let mut b = Bus::new();
        assert_eq!(b.map_ram(0x1234, 16), Err(MapError::UnalignedBase(0x1234)));
        assert_eq!(b.map_ram(0, 0), Err(MapError::BadSize(0, 0)));
        assert_eq!(
            b.map_ram(0xFF00_0000, SLOT_SIZE as usize + 1),
            Err(MapError::BadSize(0xFF00_0000, u64::from(SLOT_SIZE) + 1))
        );
        b.map_ram(0x0300_0000, 16).unwrap();
        assert_eq!(b.map_rom(0x0300_0000, vec![0; 4]), Err(MapError::Occupied(0x0300_0000)));
    }

    #[test]
    fn devices_see_offset_and_width_within_their_size() {
        let recorder = Rc::new(RefCell::new(Recorder::default()));
        let mut b = Bus::new();
        b.map_device(0x0400_0000, recorder.clone()).unwrap();

        assert_eq!(b.read(0x0400_0004, Width::Word).unwrap(), 1);
        assert_eq!(b.read(0x0400_0004, Width::Word).unwrap(), 2);
        b.write(0x0400_0008, Width::Byte, 0xFF).unwrap();

        // Past the declared size, even though the slot is 16 MiB.
        assert_eq!(b.read(0x0400_0100, Width::Byte), Err(Fault::Unmapped(0x0400_0100)));
        assert_eq!(b.read(0x0400_00FE, Width::Word), Err(Fault::Unmapped(0x0400_00FE)));

        let log = &recorder.borrow().log;
        assert_eq!(log[0], (4, Width::Word, None));
        assert_eq!(log[2], (8, Width::Byte, Some(0xFF)));
    }

    #[test]
    fn a_byte_device_is_assembled_little_endian() {
        struct Counter;
        impl Device for Counter {
            fn size(&self) -> u32 {
                8
            }
            fn read_byte(&mut self, offset: u32) -> u8 {
                offset as u8
            }
            fn write_byte(&mut self, _: u32, _: u8) {}
        }
        let mut b = Bus::new();
        b.map_device(0x0400_0000, Rc::new(RefCell::new(Counter))).unwrap();
        assert_eq!(b.read(0x0400_0001, Width::Word).unwrap(), 0x0403_0201);
        assert_eq!(b.read(0x0400_0002, Width::Half).unwrap(), 0x0302);
    }

    #[test]
    fn host_can_borrow_memory_from_an_address() {
        let mut b = bus();
        b.write(0x0200_0004, Width::Word, 0x0403_0201).unwrap();
        assert_eq!(&b.bytes_at(0x0200_0004).unwrap()[..4], &[1, 2, 3, 4]);
        assert_eq!(b.bytes_at(0).unwrap(), &[0xA0, 0xA1, 0xA2, 0xA3]);
        assert_eq!(b.bytes_at(2).unwrap(), &[0xA2, 0xA3]);
        assert_eq!(b.bytes_at(4).unwrap(), &[]);
        assert_eq!(b.bytes_at(5), None);
        assert_eq!(b.bytes_at(0x0100_0000), None);
    }

    #[test]
    fn a_loader_may_write_rom_and_clear_memory() {
        let mut b = bus();
        b.load(1, &[0xB1, 0xB2]).unwrap();
        assert_eq!(b.read(0, Width::Word).unwrap(), 0xA3B2_B1A0);
        b.fill(0x0200_0004, 3, 0xEE).unwrap();
        assert_eq!(b.read(0x0200_0004, Width::Word).unwrap(), 0x00EE_EEEE);

        assert_eq!(b.load(2, &[0; 4]), Err(Fault::Unmapped(2)));
        assert_eq!(b.fill(0x0100_0000, 1, 0), Err(Fault::Unmapped(0x0100_0000)));
    }
}
