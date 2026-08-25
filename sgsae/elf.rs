//! A loader for 32-bit RISC-V ELF executables.
//!
//! Just enough to get a statically linked, non-relocatable cartridge into
//! memory: validate the header, walk the program headers, and copy each
//! `PT_LOAD` segment to its virtual address. There is no dynamic linking, no
//! relocation processing, and no section parsing — a bare-metal game needs
//! none of it.
//!
//! The input is untrusted, so every field is bounds-checked and nothing here
//! panics on a malformed file.
//!
//! Note that the loader does not police the instruction set. The `riscv32imc`
//! toolchain marks every file as compressed-capable in `e_flags` whether or not
//! it emitted any compressed instructions, so that flag cannot be used to
//! reject a build. A cartridge that really does contain them traps on the first
//! one with the address and encoding in hand.
//!
//! ```no_run
//! use sgsae::elf;
//! use sgsae::machine::Machine;
//! use sgsae::memory::Bus;
//!
//! let cartridge = std::fs::read("game.elf")?;
//! let mut bus = Bus::new();
//! # bus.map_ram(0, 4096).unwrap();
//! let entry = elf::load(&mut bus, &cartridge)?;
//!
//! let mut machine = Machine::new(bus);
//! machine.set_pc(entry);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::fmt;

use crate::memory::{Bus, Fault};
use crate::types::Address;

/// `\x7fELF`.
const MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
/// `EI_CLASS` value for 32-bit objects.
const ELFCLASS32: u8 = 1;
/// `EI_DATA` value for little-endian objects.
const ELFDATA2LSB: u8 = 1;
/// `e_type` value for a fixed-address executable.
const ET_EXEC: u16 = 2;
/// `e_machine` value for RISC-V.
const EM_RISCV: u16 = 243;
/// `p_type` value for a segment that gets loaded into memory.
const PT_LOAD: u32 = 1;

const EHDR_SIZE: usize = 52;
const PHDR_SIZE: usize = 32;

/// Why a file could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElfError {
    /// The file is shorter than the structure it claims to contain.
    Truncated,
    /// The magic number is not `\x7fELF`.
    NotElf,
    /// Not a 32-bit little-endian object.
    WrongClass,
    /// Not a fixed-address executable — a relocatable object or shared library.
    NotExecutable,
    /// Built for some architecture other than RISC-V.
    WrongMachine(u16),
    /// `e_phentsize` is not the size of an ELF32 program header.
    BadProgramHeaderSize(u16),
    /// A segment declares more bytes in the file than in memory.
    SegmentLargerInFileThanMemory { vaddr: u32 },
    /// A segment would wrap past the end of the address space.
    SegmentOutOfRange { vaddr: u32, memsz: u32 },
    /// A segment could not be written to the bus.
    Unmapped { vaddr: u32, cause: Fault },
    /// There were no loadable segments.
    Empty,
}

impl fmt::Display for ElfError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ElfError::Truncated => write!(f, "file is truncated"),
            ElfError::NotElf => write!(f, "not an ELF file"),
            ElfError::WrongClass => write!(f, "not a 32-bit little-endian ELF"),
            ElfError::NotExecutable => {
                write!(f, "not a fixed-address executable; link it with a linker script")
            }
            ElfError::WrongMachine(m) => write!(f, "built for machine {m}, not RISC-V (243)"),
            ElfError::BadProgramHeaderSize(n) => {
                write!(f, "program header size {n}, expected {PHDR_SIZE}")
            }
            ElfError::SegmentLargerInFileThanMemory { vaddr } => {
                write!(f, "segment at {vaddr:#010x} has filesz > memsz")
            }
            ElfError::SegmentOutOfRange { vaddr, memsz } => {
                write!(f, "segment at {vaddr:#010x} + {memsz:#x} leaves the address space")
            }
            ElfError::Unmapped { vaddr, cause } => {
                write!(f, "segment at {vaddr:#010x}: {cause}")
            }
            ElfError::Empty => write!(f, "no loadable segments"),
        }
    }
}

impl std::error::Error for ElfError {}

/// One `PT_LOAD` segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment<'a> {
    /// Where it goes in the guest address space.
    pub vaddr: Address,
    /// Bytes present in the file. Shorter than `memsz` for `.bss`.
    pub bytes: &'a [u8],
    /// Bytes it occupies in memory; the tail past `bytes` is zero.
    pub memsz: u32,
}

impl Segment<'_> {
    /// Bytes of zero-filled tail — `.bss` and friends.
    pub fn zero_fill(&self) -> u32 {
        self.memsz - self.bytes.len() as u32
    }
}

/// A parsed executable, borrowing the file it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image<'a> {
    /// Address of the first instruction.
    pub entry: Address,
    pub segments: Vec<Segment<'a>>,
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16, ElfError> {
    bytes.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or(ElfError::Truncated)
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, ElfError> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or(ElfError::Truncated)
}

/// Parse an ELF32 RISC-V executable without copying its contents.
pub fn parse(file: &[u8]) -> Result<Image<'_>, ElfError> {
    if file.len() < EHDR_SIZE {
        return Err(ElfError::Truncated);
    }
    if file[..4] != MAGIC {
        return Err(ElfError::NotElf);
    }
    if file[4] != ELFCLASS32 || file[5] != ELFDATA2LSB {
        return Err(ElfError::WrongClass);
    }

    let machine = u16_at(file, 0x12)?;
    if machine != EM_RISCV {
        return Err(ElfError::WrongMachine(machine));
    }
    if u16_at(file, 0x10)? != ET_EXEC {
        return Err(ElfError::NotExecutable);
    }
    let entry = u32_at(file, 0x18)?;
    let phoff = u32_at(file, 0x1C)? as usize;
    let phentsize = u16_at(file, 0x2A)?;
    let phnum = u16_at(file, 0x2C)? as usize;

    if phentsize as usize != PHDR_SIZE {
        return Err(ElfError::BadProgramHeaderSize(phentsize));
    }

    let mut segments = Vec::new();
    for i in 0..phnum {
        let at = phoff.checked_add(i * PHDR_SIZE).ok_or(ElfError::Truncated)?;
        if u32_at(file, at)? != PT_LOAD {
            continue;
        }

        let offset = u32_at(file, at + 0x04)? as usize;
        let vaddr = u32_at(file, at + 0x08)?;
        let filesz = u32_at(file, at + 0x10)?;
        let memsz = u32_at(file, at + 0x14)?;

        if filesz > memsz {
            return Err(ElfError::SegmentLargerInFileThanMemory { vaddr });
        }
        if u64::from(vaddr) + u64::from(memsz) > 1u64 << 32 {
            return Err(ElfError::SegmentOutOfRange { vaddr, memsz });
        }
        let end = offset.checked_add(filesz as usize).ok_or(ElfError::Truncated)?;
        let bytes = file.get(offset..end).ok_or(ElfError::Truncated)?;

        // A zero-length segment is legal but there is nothing to place.
        if memsz > 0 {
            segments.push(Segment { vaddr, bytes, memsz });
        }
    }

    if segments.is_empty() {
        return Err(ElfError::Empty);
    }
    Ok(Image { entry, segments })
}

/// Parse `file` and copy its segments into `bus`, returning the entry point.
///
/// Segments are placed with [`Bus::load`], so read-only regions accept them —
/// a cartridge's code belongs in ROM, and only the loader may put it there.
pub fn load(bus: &mut Bus, file: &[u8]) -> Result<Address, ElfError> {
    let image = parse(file)?;
    for segment in &image.segments {
        bus.load(segment.vaddr, segment.bytes)
            .map_err(|cause| ElfError::Unmapped { vaddr: segment.vaddr, cause })?;

        // Zero the tail so `.bss` starts clean even if the region was reused.
        let zeros = segment.zero_fill();
        if zeros > 0 {
            let tail = segment.vaddr.wrapping_add(segment.bytes.len() as u32);
            bus.fill(tail, zeros, 0).map_err(|cause| ElfError::Unmapped { vaddr: tail, cause })?;
        }
    }
    Ok(image.entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Width;

    /// Build a minimal but valid ELF32 RISC-V executable.
    fn elf(entry: u32, segments: &[(u32, &[u8], u32)]) -> Vec<u8> {
        let phoff = EHDR_SIZE;
        let data_at = phoff + segments.len() * PHDR_SIZE;

        let mut header = vec![0u8; EHDR_SIZE];
        header[..4].copy_from_slice(&MAGIC);
        header[4] = ELFCLASS32;
        header[5] = ELFDATA2LSB;
        header[6] = 1; // EI_VERSION
        header[0x10..0x12].copy_from_slice(&ET_EXEC.to_le_bytes());
        header[0x12..0x14].copy_from_slice(&EM_RISCV.to_le_bytes());
        header[0x14..0x18].copy_from_slice(&1u32.to_le_bytes());
        header[0x18..0x1C].copy_from_slice(&entry.to_le_bytes());
        header[0x1C..0x20].copy_from_slice(&(phoff as u32).to_le_bytes());
        header[0x28..0x2A].copy_from_slice(&(EHDR_SIZE as u16).to_le_bytes());
        header[0x2A..0x2C].copy_from_slice(&(PHDR_SIZE as u16).to_le_bytes());
        header[0x2C..0x2E].copy_from_slice(&(segments.len() as u16).to_le_bytes());

        let mut phdrs = Vec::new();
        let mut body = Vec::new();
        for (vaddr, bytes, memsz) in segments {
            let offset = data_at + body.len();
            let mut ph = vec![0u8; PHDR_SIZE];
            ph[0x00..0x04].copy_from_slice(&PT_LOAD.to_le_bytes());
            ph[0x04..0x08].copy_from_slice(&(offset as u32).to_le_bytes());
            ph[0x08..0x0C].copy_from_slice(&vaddr.to_le_bytes());
            ph[0x0C..0x10].copy_from_slice(&vaddr.to_le_bytes());
            ph[0x10..0x14].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
            ph[0x14..0x18].copy_from_slice(&memsz.to_le_bytes());
            ph[0x18..0x1C].copy_from_slice(&5u32.to_le_bytes()); // PF_R | PF_X
            phdrs.extend_from_slice(&ph);
            body.extend_from_slice(bytes);
        }

        let mut out = header;
        out.extend_from_slice(&phdrs);
        out.extend_from_slice(&body);
        out
    }

    fn bus() -> Bus {
        let mut bus = Bus::new();
        bus.map_rom(0x0000_0000, vec![0; 4096]).unwrap();
        bus.map_ram(0x0200_0000, 4096).unwrap();
        bus
    }

    #[test]
    fn parses_entry_point_and_segments() {
        let text: &[u8] = &[1, 2, 3, 4];
        let file = elf(0x40, &[(0, text, 4)]);
        let image = parse(&file).unwrap();

        assert_eq!(image.entry, 0x40);
        assert_eq!(image.segments.len(), 1);
        assert_eq!(image.segments[0].vaddr, 0);
        assert_eq!(image.segments[0].bytes, text);
    }

    #[test]
    fn loads_code_into_read_only_memory() {
        // A cartridge's code lives in ROM; only the loader may put it there.
        let file = elf(0x10, &[(0, &[0xAA, 0xBB, 0xCC, 0xDD], 4)]);
        let mut b = bus();
        assert_eq!(load(&mut b, &file).unwrap(), 0x10);
        assert_eq!(b.read(0, Width::Word).unwrap(), 0xDDCC_BBAA);
        // The guest still cannot write there.
        assert!(b.write(0, Width::Byte, 0).is_err());
    }

    #[test]
    fn zero_fills_the_bss_tail() {
        let mut b = bus();
        // Dirty the RAM first, so the zeroing is observable.
        b.write(0x0200_0000, Width::Word, 0xFFFF_FFFF).unwrap();

        // filesz 2, memsz 8: two bytes of data then six of .bss.
        let file = elf(0, &[(0x0200_0000, &[1, 2], 8)]);
        load(&mut b, &file).unwrap();

        assert_eq!(b.read(0x0200_0000, Width::Byte).unwrap(), 1);
        assert_eq!(b.read(0x0200_0001, Width::Byte).unwrap(), 2);
        for i in 2..8 {
            assert_eq!(b.read(0x0200_0000 + i, Width::Byte).unwrap(), 0, "byte {i}");
        }
    }

    #[test]
    fn places_several_segments_at_their_own_addresses() {
        let file = elf(
            0,
            &[(0x0000_0000, &[0x11; 4], 4), (0x0200_0100, &[0x22; 4], 4)],
        );
        let mut b = bus();
        load(&mut b, &file).unwrap();
        assert_eq!(b.read(0, Width::Byte).unwrap(), 0x11);
        assert_eq!(b.read(0x0200_0100, Width::Byte).unwrap(), 0x22);
    }

    #[test]
    fn non_loadable_segments_are_skipped() {
        let mut file = elf(0, &[(0, &[1, 2, 3, 4], 4)]);
        // Retype the only segment as something other than PT_LOAD.
        file[EHDR_SIZE..EHDR_SIZE + 4].copy_from_slice(&3u32.to_le_bytes()); // PT_INTERP
        assert_eq!(parse(&file), Err(ElfError::Empty));
    }

    // -- rejecting things that are not our executables -----------------------

    #[test]
    fn rejects_files_that_are_not_our_executables() {
        assert_eq!(parse(&[]), Err(ElfError::Truncated));
        assert_eq!(parse(&[0; 64]), Err(ElfError::NotElf));

        let good = elf(0, &[(0, &[1, 2, 3, 4], 4)]);

        let mut wrong_class = good.clone();
        wrong_class[4] = 2; // ELFCLASS64
        assert_eq!(parse(&wrong_class), Err(ElfError::WrongClass));

        let mut big_endian = good.clone();
        big_endian[5] = 2; // ELFDATA2MSB
        assert_eq!(parse(&big_endian), Err(ElfError::WrongClass));

        let mut x86 = good.clone();
        x86[0x12..0x14].copy_from_slice(&62u16.to_le_bytes()); // EM_X86_64
        assert_eq!(parse(&x86), Err(ElfError::WrongMachine(62)));

        let mut relocatable = good.clone();
        relocatable[0x10..0x12].copy_from_slice(&1u16.to_le_bytes()); // ET_REL
        assert_eq!(parse(&relocatable), Err(ElfError::NotExecutable));
    }

    #[test]
    fn the_rvc_header_flag_alone_is_not_a_rejection() {
        // The `riscv32imc` toolchain sets EF_RISCV_RVC in e_flags because the
        // *triple* allows compressed instructions, even when the build emitted
        // none. Rejecting on the flag would turn away perfectly runnable
        // cartridges; a genuinely compressed one traps at the first bad word,
        // which names the address and the encoding.
        let mut file = elf(0, &[(0, &[1, 2, 3, 4], 4)]);
        file[0x24..0x28].copy_from_slice(&1u32.to_le_bytes()); // EF_RISCV_RVC
        assert!(parse(&file).is_ok());
    }

    #[test]
    fn a_truncated_file_does_not_panic() {
        let file = elf(0, &[(0, &[1, 2, 3, 4], 4)]);
        // Every prefix must produce an error rather than an index panic.
        for len in 0..file.len() {
            assert!(parse(&file[..len]).is_err(), "prefix of {len} bytes parsed");
        }
        assert!(parse(&file).is_ok());
    }

    #[test]
    fn a_segment_pointing_outside_the_file_is_rejected() {
        let mut file = elf(0, &[(0, &[1, 2, 3, 4], 4)]);
        // Claim far more file bytes than exist.
        file[EHDR_SIZE + 0x10..EHDR_SIZE + 0x14].copy_from_slice(&0x1000u32.to_le_bytes());
        file[EHDR_SIZE + 0x14..EHDR_SIZE + 0x18].copy_from_slice(&0x1000u32.to_le_bytes());
        assert_eq!(parse(&file), Err(ElfError::Truncated));
    }

    #[test]
    fn a_segment_that_wraps_the_address_space_is_rejected() {
        let file = elf(0, &[(0xFFFF_FF00, &[0; 4], 0x400)]);
        assert_eq!(
            parse(&file),
            Err(ElfError::SegmentOutOfRange { vaddr: 0xFFFF_FF00, memsz: 0x400 })
        );
    }

    #[test]
    fn filesz_larger_than_memsz_is_rejected() {
        let mut file = elf(0, &[(0x100, &[1, 2, 3, 4], 4)]);
        file[EHDR_SIZE + 0x14..EHDR_SIZE + 0x18].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(parse(&file), Err(ElfError::SegmentLargerInFileThanMemory { vaddr: 0x100 }));
    }

    #[test]
    fn loading_somewhere_unmapped_reports_the_address() {
        let file = elf(0, &[(0x0F00_0000, &[1, 2, 3, 4], 4)]);
        let mut b = bus();
        assert!(matches!(load(&mut b, &file), Err(ElfError::Unmapped { vaddr: 0x0F00_0000, .. })));
    }
}
