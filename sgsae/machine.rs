//! The RV32IM interpreter.
//!
//! The machine owns a [`Bus`] and 32 registers, and runs in cycle-budgeted
//! slices so a host can drive it from a frame loop:
//!
//! ```no_run
//! # use sgsae::machine::{Machine, Trap};
//! # use sgsae::memory::Bus;
//! # let mut machine = Machine::new(Bus::new());
//! # const CYCLES_PER_FRAME: u64 = 280_000;
//! loop {
//!     match machine.run(CYCLES_PER_FRAME) {
//!         Ok(()) => break,                             // frame done, yield to the host
//!         Err(Trap::EnvironmentCall { .. }) => {
//!             // …service the call and keep going; the ECALL has retired.
//!         }
//!         Err(other) => panic!("{other}"),
//!     }
//! }
//! ```
//!
//! Nothing in here panics on guest behaviour. A bad address, a misaligned
//! jump, or an unrecognised opcode all come back as a [`Trap`], which is what
//! lets a host show a fault screen instead of taking the page down with it.

use std::fmt;

use crate::compressed;
use crate::instructions::Instruction;
use crate::memory::{Bus, Fault, Width};
use crate::types::{Address, Register, Word};

/// An abnormal condition that stopped execution.
///
/// Every variant carries the address of the instruction responsible. `ECALL`
/// and `EBREAK` are traps too — that is the seam a host uses for syscalls and
/// breakpoints.
///
/// A fault leaves `pc` on the instruction that caused it. `ECALL` is a call,
/// not a fault, so it is the one trap that retires: `pc` has already moved on
/// and the host just calls [`Machine::run`] again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trap {
    /// The word at `pc` is not a valid RV32IM encoding.
    IllegalInstruction { pc: Address, word: Word },
    /// There is no memory at `pc`.
    FetchFault { pc: Address },
    /// `pc` is not 2-byte aligned.
    MisalignedFetch { pc: Address },
    /// A branch or jump targeted a misaligned address.
    MisalignedJump { pc: Address, target: Address },
    /// A load failed.
    LoadFault { pc: Address, addr: Address, cause: Fault },
    /// A store failed.
    StoreFault { pc: Address, addr: Address, cause: Fault },
    /// The guest executed `ECALL`.
    EnvironmentCall { pc: Address },
    /// The guest executed `EBREAK`.
    Breakpoint { pc: Address },
    /// The guest touched a CSR. No control and status registers are
    /// implemented.
    UnsupportedCsr { pc: Address, csr: u16 },
}

impl fmt::Display for Trap {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Trap::IllegalInstruction { pc, word } => {
                write!(f, "illegal instruction {word:#010x} at {pc:#010x}")
            }
            Trap::FetchFault { pc } => write!(f, "fetch from unmapped {pc:#010x}"),
            Trap::MisalignedFetch { pc } => write!(f, "misaligned fetch at {pc:#010x}"),
            Trap::MisalignedJump { pc, target } => {
                write!(f, "jump at {pc:#010x} to misaligned {target:#010x}")
            }
            Trap::LoadFault { pc, cause, .. } => write!(f, "load at {pc:#010x}: {cause}"),
            Trap::StoreFault { pc, cause, .. } => write!(f, "store at {pc:#010x}: {cause}"),
            Trap::EnvironmentCall { pc } => write!(f, "ecall at {pc:#010x}"),
            Trap::Breakpoint { pc } => write!(f, "ebreak at {pc:#010x}"),
            Trap::UnsupportedCsr { pc, csr } => {
                write!(f, "unsupported csr {csr:#05x} at {pc:#010x}")
            }
        }
    }
}

impl std::error::Error for Trap {}

/// An RV32IM hart and the bus it is attached to.
pub struct Machine {
    pc: Address,
    /// Register 0 is never written, so it always reads as zero.
    registers: [Word; 32],
    bus: Bus,
    cycles: u64,
}

impl Machine {
    /// Create a machine attached to `bus`, with `pc` at 0 and all registers zero.
    pub fn new(bus: Bus) -> Machine {
        Machine { pc: 0, registers: [0; 32], bus, cycles: 0 }
    }

    // -- host inspection -----------------------------------------------------

    pub fn pc(&self) -> Address {
        self.pc
    }

    pub fn set_pc(&mut self, pc: Address) {
        self.pc = pc;
    }

    pub fn registers(&self) -> &[Word; 32] {
        &self.registers
    }

    /// Read register `i`. Register 0 always reads as zero.
    pub fn register(&self, i: Register) -> Word {
        self.registers[usize::from(i)]
    }

    /// Write register `i`. Writes to register 0 are discarded.
    pub fn set_register(&mut self, i: Register, value: Word) {
        if i != 0 {
            self.registers[usize::from(i)] = value;
        }
    }

    /// Clear every register, as a reset would.
    ///
    /// A program starting up is entitled to assume nothing about what the last
    /// one left behind.
    pub fn reset_registers(&mut self) {
        self.registers = [0; 32];
    }

    /// Instructions retired since the machine was created.
    pub fn cycles(&self) -> u64 {
        self.cycles
    }

    pub fn bus(&self) -> &Bus {
        &self.bus
    }

    pub fn bus_mut(&mut self) -> &mut Bus {
        &mut self.bus
    }

    // -- execution -----------------------------------------------------------

    /// Run until the machine traps or `budget` instructions have retired.
    ///
    /// `Ok` means the budget was spent; call again to continue.
    pub fn run(&mut self, budget: u64) -> Result<(), Trap> {
        for _ in 0..budget {
            self.step()?;
        }
        Ok(())
    }

    /// Fetch, decode, and execute one instruction.
    pub fn step(&mut self) -> Result<(), Trap> {
        let (instruction, width) = self.fetch()?;
        self.pc = self.execute_width(instruction, width)?;
        self.cycles += 1;
        Ok(())
    }

    /// Read the instruction at `pc`, returning it and how many bytes it took.
    ///
    /// Instructions are 4 bytes unless the first halfword marks them
    /// compressed, in which case they are 2 — so a 2-byte instruction at the
    /// very end of memory does not fault.
    fn fetch(&self) -> Result<(Instruction, u32), Trap> {
        let pc = self.pc;
        if pc % 2 != 0 {
            return Err(Trap::MisalignedFetch { pc });
        }
        let fault = || Trap::FetchFault { pc };
        let bytes = self.bus.bytes_at(pc).ok_or_else(fault)?;
        let &[a, b, ..] = bytes else { return Err(fault()) };
        let low = u16::from_le_bytes([a, b]);
        if compressed::is_compressed(low) {
            return Ok((compressed::decode(low), 2));
        }
        let &[_, _, c, d, ..] = bytes else { return Err(fault()) };
        Ok((Instruction::decode(u32::from_le_bytes([a, b, c, d])), 4))
    }

    /// Address for a load or store: `rs1 + offset`, wrapping.
    fn effective_address(&self, rs1: Register, offset: i32) -> Address {
        self.register(rs1).wrapping_add(offset as u32)
    }

    /// Load through `extend`, which turns the zero-extended bus value into
    /// the register value — the signed loads sign-extend from their own width.
    fn ld(
        &mut self,
        rd: Register,
        rs1: Register,
        offset: i32,
        width: Width,
        extend: impl Fn(u32) -> u32,
    ) -> Result<(), Trap> {
        let addr = self.effective_address(rs1, offset);
        let pc = self.pc;
        let v = self.bus.read(addr, width).map_err(|cause| Trap::LoadFault { pc, addr, cause })?;
        self.set_register(rd, extend(v));
        Ok(())
    }

    fn st(&mut self, rs1: Register, rs2: Register, offset: i32, width: Width) -> Result<(), Trap> {
        let addr = self.effective_address(rs1, offset);
        let pc = self.pc;
        self.bus
            .write(addr, width, self.register(rs2))
            .map_err(|cause| Trap::StoreFault { pc, addr, cause })
    }

    /// `rd = f(rs1, rs2)`.
    fn rr(&mut self, rd: Register, rs1: Register, rs2: Register, f: impl Fn(u32, u32) -> u32) {
        self.set_register(rd, f(self.register(rs1), self.register(rs2)));
    }

    /// `rd = f(rs1, imm)`, with the sign-extended immediate as a `u32`.
    fn ri(&mut self, rd: Register, rs1: Register, imm: i32, f: impl Fn(u32, u32) -> u32) {
        self.set_register(rd, f(self.register(rs1), imm as u32));
    }

    /// Next `pc` after a branch: `pc + offset` if `taken(rs1, rs2)`, else `next`.
    fn br(
        &self,
        next: Address,
        rs1: Register,
        rs2: Register,
        offset: i32,
        taken: impl Fn(u32, u32) -> bool,
    ) -> Result<Address, Trap> {
        if taken(self.register(rs1), self.register(rs2)) {
            self.aligned(self.pc.wrapping_add(offset as u32))
        } else {
            Ok(next)
        }
    }

    /// A control-transfer target, rejecting a misaligned one at the jump
    /// itself. With compressed instructions in play only an odd target is out.
    fn aligned(&self, target: Address) -> Result<Address, Trap> {
        if target % 2 != 0 {
            return Err(Trap::MisalignedJump { pc: self.pc, target });
        }
        Ok(target)
    }

    /// Execute one already-decoded 4-byte instruction against the current
    /// `pc`, without advancing it.
    ///
    /// Call [`Machine::step`] for the normal fetch-execute cycle — this is
    /// exposed for tests and for hosts that want to inject an instruction.
    pub fn execute(&mut self, instruction: Instruction) -> Result<(), Trap> {
        self.execute_width(instruction, 4).map(|_| ())
    }

    /// Execute one instruction of `width` bytes, returning the next `pc`.
    #[rustfmt::skip]
    fn execute_width(&mut self, instruction: Instruction, width: u32) -> Result<Address, Trap> {
        use Instruction as I;
        let pc = self.pc;
        let next = pc.wrapping_add(width);

        match instruction {
            // -- upper immediates and jumps --------------------------------
            I::LUI { rd, imm } => self.set_register(rd, imm as u32),
            I::AUIPC { rd, imm } => self.set_register(rd, pc.wrapping_add(imm as u32)),
            I::JAL { rd, offset } => {
                let target = self.aligned(pc.wrapping_add(offset as u32))?;
                self.set_register(rd, next);
                return Ok(target);
            }
            I::JALR { rd, rs1, offset } => {
                // The spec clears the low bit of the computed target.
                let target = self.aligned(self.effective_address(rs1, offset) & !1)?;
                self.set_register(rd, next);
                return Ok(target);
            }

            // -- branches --------------------------------------------------
            I::BEQ  { rs1, rs2, offset } => return self.br(next, rs1, rs2, offset, |a, b| a == b),
            I::BNE  { rs1, rs2, offset } => return self.br(next, rs1, rs2, offset, |a, b| a != b),
            I::BLT  { rs1, rs2, offset } => return self.br(next, rs1, rs2, offset, |a, b| (a as i32) < (b as i32)),
            I::BGE  { rs1, rs2, offset } => return self.br(next, rs1, rs2, offset, |a, b| (a as i32) >= (b as i32)),
            I::BLTU { rs1, rs2, offset } => return self.br(next, rs1, rs2, offset, |a, b| a < b),
            I::BGEU { rs1, rs2, offset } => return self.br(next, rs1, rs2, offset, |a, b| a >= b),

            // -- loads and stores ------------------------------------------
            I::LB  { rd, rs1, offset } => self.ld(rd, rs1, offset, Width::Byte, |v| v as u8 as i8 as i32 as u32)?,
            I::LH  { rd, rs1, offset } => self.ld(rd, rs1, offset, Width::Half, |v| v as u16 as i16 as i32 as u32)?,
            I::LW  { rd, rs1, offset } => self.ld(rd, rs1, offset, Width::Word, |v| v)?,
            I::LBU { rd, rs1, offset } => self.ld(rd, rs1, offset, Width::Byte, |v| v)?,
            I::LHU { rd, rs1, offset } => self.ld(rd, rs1, offset, Width::Half, |v| v)?,
            I::SB  { rs1, rs2, offset } => self.st(rs1, rs2, offset, Width::Byte)?,
            I::SH  { rs1, rs2, offset } => self.st(rs1, rs2, offset, Width::Half)?,
            I::SW  { rs1, rs2, offset } => self.st(rs1, rs2, offset, Width::Word)?,

            // -- register-immediate ----------------------------------------
            // Every immediate arrives sign-extended from the decoder, so the
            // bitwise ops get the full-width value the spec calls for. Shift
            // amounts use only their low five bits.
            I::ADDI  { rd, rs1, imm } => self.ri(rd, rs1, imm, u32::wrapping_add),
            I::SLTI  { rd, rs1, imm } => self.ri(rd, rs1, imm, |a, b| ((a as i32) < (b as i32)) as u32),
            I::SLTIU { rd, rs1, imm } => self.ri(rd, rs1, imm, |a, b| (a < b) as u32),
            I::XORI  { rd, rs1, imm } => self.ri(rd, rs1, imm, |a, b| a ^ b),
            I::ORI   { rd, rs1, imm } => self.ri(rd, rs1, imm, |a, b| a | b),
            I::ANDI  { rd, rs1, imm } => self.ri(rd, rs1, imm, |a, b| a & b),
            I::SLLI  { rd, rs1, shamt } => self.ri(rd, rs1, shamt as i32, |a, b| a << (b & 31)),
            I::SRLI  { rd, rs1, shamt } => self.ri(rd, rs1, shamt as i32, |a, b| a >> (b & 31)),
            I::SRAI  { rd, rs1, shamt } => self.ri(rd, rs1, shamt as i32, |a, b| ((a as i32) >> (b & 31)) as u32),

            // -- register-register -----------------------------------------
            I::ADD  { rd, rs1, rs2 } => self.rr(rd, rs1, rs2, u32::wrapping_add),
            I::SUB  { rd, rs1, rs2 } => self.rr(rd, rs1, rs2, u32::wrapping_sub),
            I::SLL  { rd, rs1, rs2 } => self.rr(rd, rs1, rs2, |a, b| a << (b & 31)),
            I::SLT  { rd, rs1, rs2 } => self.rr(rd, rs1, rs2, |a, b| ((a as i32) < (b as i32)) as u32),
            I::SLTU { rd, rs1, rs2 } => self.rr(rd, rs1, rs2, |a, b| (a < b) as u32),
            I::XOR  { rd, rs1, rs2 } => self.rr(rd, rs1, rs2, |a, b| a ^ b),
            I::SRL  { rd, rs1, rs2 } => self.rr(rd, rs1, rs2, |a, b| a >> (b & 31)),
            I::SRA  { rd, rs1, rs2 } => self.rr(rd, rs1, rs2, |a, b| ((a as i32) >> (b & 31)) as u32),
            I::OR   { rd, rs1, rs2 } => self.rr(rd, rs1, rs2, |a, b| a | b),
            I::AND  { rd, rs1, rs2 } => self.rr(rd, rs1, rs2, |a, b| a & b),

            // -- M extension -----------------------------------------------
            // Division never traps on RV32M. The spec fixes divide-by-zero to
            // all-ones / the dividend, and the one signed overflow case
            // (MIN / -1) to MIN / 0 — which is what the wrapping ops give.
            I::MUL    { rd, rs1, rs2 } => self.rr(rd, rs1, rs2, u32::wrapping_mul),
            I::MULH   { rd, rs1, rs2 } => self.rr(rd, rs1, rs2, |a, b| ((i64::from(a as i32) * i64::from(b as i32)) >> 32) as u32),
            I::MULHSU { rd, rs1, rs2 } => self.rr(rd, rs1, rs2, |a, b| ((i64::from(a as i32) * i64::from(b)) >> 32) as u32),
            I::MULHU  { rd, rs1, rs2 } => self.rr(rd, rs1, rs2, |a, b| ((u64::from(a) * u64::from(b)) >> 32) as u32),
            I::DIV    { rd, rs1, rs2 } => self.rr(rd, rs1, rs2, |a, b| if b == 0 { u32::MAX } else { (a as i32).wrapping_div(b as i32) as u32 }),
            I::DIVU   { rd, rs1, rs2 } => self.rr(rd, rs1, rs2, |a, b| if b == 0 { u32::MAX } else { a / b }),
            I::REM    { rd, rs1, rs2 } => self.rr(rd, rs1, rs2, |a, b| if b == 0 { a } else { (a as i32).wrapping_rem(b as i32) as u32 }),
            I::REMU   { rd, rs1, rs2 } => self.rr(rd, rs1, rs2, |a, b| if b == 0 { a } else { a % b }),

            // -- ordering and system ---------------------------------------
            // Nothing is reordered or cached, so both fences retire as no-ops.
            I::FENCE { .. } | I::FENCEI => {}

            I::ECALL => {
                self.pc = next;
                return Err(Trap::EnvironmentCall { pc });
            }
            I::EBREAK => return Err(Trap::Breakpoint { pc }),

            I::CSRRW { csr, .. } | I::CSRRS { csr, .. } | I::CSRRC { csr, .. }
            | I::CSRRWI { csr, .. } | I::CSRRSI { csr, .. } | I::CSRRCI { csr, .. } => {
                return Err(Trap::UnsupportedCsr { pc, csr })
            }

            I::UNKNOWN { word } => return Err(Trap::IllegalInstruction { pc, word }),
        }
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::ConsoleOut;
    use crate::input::{Button, Input, Pad};
    use crate::instructions::Instruction as I;
    use std::cell::RefCell;
    use std::rc::Rc;

    const ROM: Address = 0x0000_0000;
    const RAM: Address = 0x0200_0000;

    fn image(program: &[I]) -> Vec<u8> {
        program.iter().flat_map(|i| i.encode().to_le_bytes()).collect()
    }

    /// A machine with the program in ROM at 0 and 1 KiB of RAM.
    fn machine(program: &[I]) -> Machine {
        let mut bus = Bus::new();
        bus.map_rom(ROM, image(program)).unwrap();
        bus.map_ram(RAM, 1024).unwrap();
        Machine::new(bus)
    }

    /// Run to the first trap, which the test programs make an `EBREAK`.
    fn run(program: &[I]) -> Machine {
        let mut m = machine(program);
        match m.run(1000) {
            Err(Trap::Breakpoint { .. }) => m,
            other => panic!("expected EBREAK, got {other:?}"),
        }
    }

    /// Execute one instruction in isolation and read back a register.
    fn exec(setup: &[(Register, u32)], instruction: I, read: Register) -> u32 {
        let mut m = Machine::new(Bus::new());
        for (r, v) in setup {
            m.set_register(*r, *v);
        }
        m.execute(instruction).unwrap();
        m.register(read)
    }

    #[test]
    fn register_zero_stays_zero() {
        let mut m = Machine::new(Bus::new());
        m.execute(I::ADDI { rd: 0, rs1: 0, imm: 1 }).unwrap();
        assert_eq!(m.register(0), 0);
        m.set_register(0, 0xFFFF_FFFF);
        assert_eq!(m.register(0), 0);
    }

    // -- the PC-relative regressions ----------------------------------------

    #[test]
    fn auipc_is_relative_to_its_own_address() {
        // AUIPC at 0x0 with a zero immediate must yield 0x0, not 0x4.
        let m = run(&[I::AUIPC { rd: 10, imm: 0 }, I::EBREAK]);
        assert_eq!(m.register(10), 0);

        // …and at 0x4, 0x4.
        let m = run(&[I::ADDI { rd: 0, rs1: 0, imm: 0 }, I::AUIPC { rd: 10, imm: 0 }, I::EBREAK]);
        assert_eq!(m.register(10), 4);

        // The immediate is a full 32-bit value and wraps, not truncates.
        let m = run(&[I::AUIPC { rd: 10, imm: -0x1000 }, I::EBREAK]);
        assert_eq!(m.register(10), 0xFFFF_F000);
    }

    #[test]
    fn a_forward_branch_lands_on_its_target() {
        // BEQ at 0x4 with offset +8 must reach 0xC, skipping the ADDI at 0x8.
        let m = run(&[
            I::ADDI { rd: 10, rs1: 0, imm: 1 },   // 0x0
            I::BEQ { rs1: 0, rs2: 0, offset: 8 }, // 0x4 -> 0xC
            I::ADDI { rd: 10, rs1: 0, imm: 99 },  // 0x8, skipped
            I::EBREAK,                            // 0xC
        ]);
        assert_eq!(m.register(10), 1);
        assert_eq!(m.pc(), 0xC);
    }

    #[test]
    fn jal_targets_its_own_address_and_links_past_itself() {
        let m = run(&[
            I::JAL { rd: 1, offset: 8 },         // 0x0 -> 0x8
            I::ADDI { rd: 10, rs1: 0, imm: 99 }, // 0x4, skipped
            I::EBREAK,                           // 0x8
        ]);
        assert_eq!(m.pc(), 0x8);
        assert_eq!(m.register(1), 0x4, "link register is the instruction after JAL");
    }

    #[test]
    fn a_backward_branch_closes_a_loop() {
        // Count down from 3; the loop body is two instructions.
        let m = run(&[
            I::ADDI { rd: 10, rs1: 0, imm: 3 },     // 0x0
            I::ADDI { rd: 10, rs1: 10, imm: -1 },   // 0x4  loop:
            I::BNE { rs1: 10, rs2: 0, offset: -4 }, // 0x8  -> 0x4
            I::EBREAK,                              // 0xC
        ]);
        assert_eq!(m.register(10), 0);
        assert_eq!(m.cycles(), 1 + 3 * 2);
    }

    #[test]
    fn jalr_links_and_clears_the_low_bit() {
        let mut m = machine(&[I::JALR { rd: 1, rs1: 5, offset: 0 }, I::EBREAK]);
        m.set_register(5, 0x9); // odd target
        m.step().unwrap();
        assert_eq!(m.pc(), 0x8, "low bit is cleared, not rounded");
        assert_eq!(m.register(1), 0x4);
    }

    #[test]
    fn a_two_byte_aligned_jump_target_is_legal() {
        // Compressed instructions are 2 bytes, so odd multiples of two are
        // perfectly good instruction addresses.
        let mut m = machine(&[I::JALR { rd: 1, rs1: 5, offset: 0 }, I::EBREAK]);
        m.set_register(5, 0xA);
        m.step().unwrap();
        assert_eq!(m.pc(), 0xA);
    }

    #[test]
    fn an_odd_jump_target_traps_at_the_jump() {
        // JALR clears the low bit and encoded displacements are always even,
        // so this is only reachable from a hand-built instruction — but it is
        // the guard that keeps the fetch stream aligned.
        let mut m = machine(&[I::EBREAK]);
        assert_eq!(
            m.execute(I::JAL { rd: 0, offset: 1 }),
            Err(Trap::MisalignedJump { pc: 0, target: 1 })
        );
        assert_eq!(m.pc(), 0, "pc stays on the instruction that trapped");
        assert_eq!(m.register(0), 0, "the link is not written on a failed jump");
    }

    #[test]
    fn compressed_instructions_are_two_bytes_wide() {
        // c.addi a0, 1 ; c.addi a0, 1 ; ebreak (32-bit). The link after a
        // compressed JAL is 2 bytes on, not 4.
        let mut bus = Bus::new();
        let mut image = Vec::new();
        image.extend_from_slice(&0x0505u16.to_le_bytes()); // 0x0 c.addi a0, 1
        image.extend_from_slice(&0x2001u16.to_le_bytes()); // 0x2 c.jal . (offset 0)
        bus.map_rom(ROM, image).unwrap();
        let mut m = Machine::new(bus);
        m.step().unwrap();
        assert_eq!(m.pc(), 2);
        assert_eq!(m.register(10), 1);
        m.step().unwrap();
        assert_eq!(m.pc(), 2, "c.jal . loops to itself");
        assert_eq!(m.register(1), 4, "link is the halfword after the c.jal");
    }

    // -- sign and zero extension --------------------------------------------

    #[test]
    fn signed_loads_sign_extend_to_the_full_word() {
        let mut m = machine(&[I::EBREAK]);
        m.bus_mut().load(RAM, &[0xF0, 0xF1, 0xF2, 0xF3]).unwrap();
        m.set_register(5, RAM);

        m.execute(I::LB { rd: 10, rs1: 5, offset: 0 }).unwrap();
        assert_eq!(m.register(10), 0xFFFF_FFF0);
        m.execute(I::LH { rd: 10, rs1: 5, offset: 0 }).unwrap();
        assert_eq!(m.register(10), 0xFFFF_F1F0);
        m.execute(I::LW { rd: 10, rs1: 5, offset: 0 }).unwrap();
        assert_eq!(m.register(10), 0xF3F2_F1F0);

        m.execute(I::LBU { rd: 10, rs1: 5, offset: 0 }).unwrap();
        assert_eq!(m.register(10), 0x0000_00F0);
        m.execute(I::LHU { rd: 10, rs1: 5, offset: 0 }).unwrap();
        assert_eq!(m.register(10), 0x0000_F1F0);
    }

    #[test]
    fn lui_carries_the_full_value_including_the_sign_bit() {
        assert_eq!(exec(&[], I::LUI { rd: 10, imm: 0xB000 }, 10), 0xB000);
        assert_eq!(exec(&[], I::LUI { rd: 10, imm: -0x8000_0000 }, 10), 0x8000_0000);
        assert_eq!(exec(&[], I::LUI { rd: 10, imm: -0x1000 }, 10), 0xFFFF_F000);
    }

    #[test]
    fn logical_immediates_are_sign_extended() {
        // `andi rd, rs, -1` is the identity, not a mask with 0xFFF.
        assert_eq!(exec(&[(5, 0xDEAD_BEEF)], I::ANDI { rd: 10, rs1: 5, imm: -1 }, 10), 0xDEAD_BEEF);
        assert_eq!(exec(&[(5, 0)], I::ORI { rd: 10, rs1: 5, imm: -1 }, 10), 0xFFFF_FFFF);
        assert_eq!(exec(&[(5, 0)], I::XORI { rd: 10, rs1: 5, imm: -1 }, 10), 0xFFFF_FFFF);
        assert_eq!(exec(&[(5, 0xAA)], I::ANDI { rd: 10, rs1: 5, imm: 0x3C }, 10), 0x28);
    }

    #[test]
    fn sltiu_compares_unsigned_against_a_sign_extended_immediate() {
        // -1 becomes 0xFFFF_FFFF, which almost everything is below.
        assert_eq!(exec(&[(5, 1)], I::SLTIU { rd: 10, rs1: 5, imm: -1 }, 10), 1);
        assert_eq!(exec(&[(5, 0xFFFF_FFFF)], I::SLTIU { rd: 10, rs1: 5, imm: -1 }, 10), 0);
        assert_eq!(exec(&[(5, 1)], I::SLTIU { rd: 10, rs1: 5, imm: 0 }, 10), 0);
        assert_eq!(exec(&[(5, 0xFFFF_FFFF)], I::SLTI { rd: 10, rs1: 5, imm: 0 }, 10), 1);
    }

    #[test]
    fn addi_reaches_both_ends_of_its_range() {
        assert_eq!(exec(&[(5, 0)], I::ADDI { rd: 10, rs1: 5, imm: 2047 }, 10), 2047);
        assert_eq!(exec(&[(5, 0)], I::ADDI { rd: 10, rs1: 5, imm: -2048 }, 10), (-2048i32) as u32);
        assert_eq!(exec(&[(5, 0)], I::ADDI { rd: 10, rs1: 5, imm: 1024 }, 10), 1024);
    }

    // -- shifts --------------------------------------------------------------

    #[test]
    fn shifts_use_only_the_low_five_bits_of_rs2() {
        assert_eq!(exec(&[(5, 1), (6, 33)], I::SLL { rd: 10, rs1: 5, rs2: 6 }, 10), 2);
        assert_eq!(
            exec(&[(5, 0x8000_0000), (6, 32)], I::SRL { rd: 10, rs1: 5, rs2: 6 }, 10),
            0x8000_0000
        );
        assert_eq!(
            exec(&[(5, 0x8000_0000), (6, 31)], I::SRA { rd: 10, rs1: 5, rs2: 6 }, 10),
            0xFFFF_FFFF
        );
        assert_eq!(exec(&[(5, 0x8000_0000), (6, 31)], I::SRL { rd: 10, rs1: 5, rs2: 6 }, 10), 1);
    }

    #[test]
    fn immediate_shifts_behave_the_same() {
        assert_eq!(exec(&[(5, 0x1000)], I::SLLI { rd: 10, rs1: 5, shamt: 4 }, 10), 0x10000);
        assert_eq!(exec(&[(5, 0xA3A2_A1A0)], I::SRLI { rd: 10, rs1: 5, shamt: 24 }, 10), 0xA3);
        assert_eq!(
            exec(&[(5, 0xA3A2_A1A0)], I::SRAI { rd: 10, rs1: 5, shamt: 24 }, 10),
            0xFFFF_FFA3
        );
    }

    // -- M extension ---------------------------------------------------------

    #[test]
    fn multiply_produces_both_halves() {
        assert_eq!(exec(&[(5, 7), (6, 1000)], I::MUL { rd: 10, rs1: 5, rs2: 6 }, 10), 7000);
        // 0x1234_5678 * 0x1000_0000 overflows 32 bits.
        let (a, b) = (0x1234_5678u32, 0x1000_0000u32);
        assert_eq!(
            exec(&[(5, a), (6, b)], I::MUL { rd: 10, rs1: 5, rs2: 6 }, 10),
            a.wrapping_mul(b)
        );
        assert_eq!(
            exec(&[(5, a), (6, b)], I::MULHU { rd: 10, rs1: 5, rs2: 6 }, 10),
            ((u64::from(a) * u64::from(b)) >> 32) as u32
        );
        // -1 * -1 = 1, so the high word is 0 signed but 0xFFFF_FFFE unsigned.
        let neg = 0xFFFF_FFFFu32;
        assert_eq!(exec(&[(5, neg), (6, neg)], I::MULH { rd: 10, rs1: 5, rs2: 6 }, 10), 0);
        assert_eq!(
            exec(&[(5, neg), (6, neg)], I::MULHU { rd: 10, rs1: 5, rs2: 6 }, 10),
            0xFFFF_FFFE
        );
        // MULHSU: rs1 signed (-1), rs2 unsigned (0xFFFF_FFFF).
        assert_eq!(
            exec(&[(5, neg), (6, neg)], I::MULHSU { rd: 10, rs1: 5, rs2: 6 }, 10),
            0xFFFF_FFFF
        );
    }

    #[test]
    fn division_follows_the_spec_instead_of_trapping() {
        let min = 0x8000_0000u32; // i32::MIN
        let neg1 = 0xFFFF_FFFFu32;

        assert_eq!(exec(&[(5, 100), (6, 7)], I::DIV { rd: 10, rs1: 5, rs2: 6 }, 10), 14);
        assert_eq!(exec(&[(5, 100), (6, 7)], I::REM { rd: 10, rs1: 5, rs2: 6 }, 10), 2);
        // Signed division truncates toward zero.
        assert_eq!(
            exec(&[(5, (-7i32) as u32), (6, 2)], I::DIV { rd: 10, rs1: 5, rs2: 6 }, 10),
            (-3i32) as u32
        );
        assert_eq!(
            exec(&[(5, (-7i32) as u32), (6, 2)], I::REM { rd: 10, rs1: 5, rs2: 6 }, 10),
            (-1i32) as u32
        );

        // Divide by zero: all ones for DIV/DIVU, the dividend for REM/REMU.
        assert_eq!(exec(&[(5, 42), (6, 0)], I::DIV { rd: 10, rs1: 5, rs2: 6 }, 10), neg1);
        assert_eq!(exec(&[(5, 42), (6, 0)], I::DIVU { rd: 10, rs1: 5, rs2: 6 }, 10), u32::MAX);
        assert_eq!(exec(&[(5, 42), (6, 0)], I::REM { rd: 10, rs1: 5, rs2: 6 }, 10), 42);
        assert_eq!(exec(&[(5, 42), (6, 0)], I::REMU { rd: 10, rs1: 5, rs2: 6 }, 10), 42);

        // The one signed overflow: i32::MIN / -1 wraps rather than faulting.
        assert_eq!(exec(&[(5, min), (6, neg1)], I::DIV { rd: 10, rs1: 5, rs2: 6 }, 10), min);
        assert_eq!(exec(&[(5, min), (6, neg1)], I::REM { rd: 10, rs1: 5, rs2: 6 }, 10), 0);

        // Unsigned division treats the operands as unsigned.
        assert_eq!(exec(&[(5, neg1), (6, 2)], I::DIVU { rd: 10, rs1: 5, rs2: 6 }, 10), 0x7FFF_FFFF);
        assert_eq!(exec(&[(5, neg1), (6, 2)], I::DIV { rd: 10, rs1: 5, rs2: 6 }, 10), 0);
    }

    // -- traps ---------------------------------------------------------------

    #[test]
    fn guest_faults_are_traps_not_panics() {
        let mut m = machine(&[I::LW { rd: 10, rs1: 0, offset: 0x100 }]);
        assert!(matches!(m.step(), Err(Trap::LoadFault { pc: 0, addr: 0x100, .. })));

        let mut m = machine(&[I::SW { rs1: 0, rs2: 0, offset: 0 }]);
        assert_eq!(
            m.step(),
            Err(Trap::StoreFault { pc: 0, addr: 0, cause: Fault::ReadOnly(0) })
        );

        // Running off the end of ROM is a fetch fault, not a panic.
        let mut m = machine(&[I::ADDI { rd: 0, rs1: 0, imm: 0 }]);
        assert_eq!(m.run(10), Err(Trap::FetchFault { pc: 4 }));

        // So is a 4-byte instruction with only two bytes left.
        let mut bus = Bus::new();
        bus.map_rom(ROM, vec![0x13, 0x05]).unwrap(); // half of `addi a0, ...`
        let mut m = Machine::new(bus);
        assert_eq!(m.step(), Err(Trap::FetchFault { pc: 0 }));

        // An odd pc cannot be fetched at all.
        let mut m = machine(&[I::EBREAK]);
        m.set_pc(1);
        assert_eq!(m.step(), Err(Trap::MisalignedFetch { pc: 1 }));
    }

    #[test]
    fn an_unrecognised_word_is_an_illegal_instruction() {
        let mut bus = Bus::new();
        bus.map_rom(ROM, 0xFFFF_FFFFu32.to_le_bytes().to_vec()).unwrap();
        let mut m = Machine::new(bus);
        assert_eq!(m.step(), Err(Trap::IllegalInstruction { pc: 0, word: 0xFFFF_FFFF }));
    }

    #[test]
    fn ecall_traps_and_retires_so_the_host_can_resume() {
        let mut m = machine(&[
            I::ADDI { rd: 10, rs1: 0, imm: 7 },
            I::ECALL,
            I::ADDI { rd: 11, rs1: 0, imm: 9 },
            I::EBREAK,
        ]);
        assert_eq!(m.run(100), Err(Trap::EnvironmentCall { pc: 4 }));
        assert_eq!(m.register(10), 7);
        assert_eq!(m.pc(), 8, "the ECALL has retired");

        assert_eq!(m.run(100), Err(Trap::Breakpoint { pc: 0xC }));
        assert_eq!(m.register(11), 9);
        assert_eq!(m.pc(), 0xC, "the EBREAK has not");
    }

    #[test]
    fn run_returns_when_the_budget_is_spent_and_resumes_where_it_left_off() {
        // A tight infinite loop: `jal x0, 0`.
        let mut m = machine(&[I::JAL { rd: 0, offset: 0 }]);
        assert_eq!(m.run(50), Ok(()));
        assert_eq!(m.cycles(), 50);
        assert_eq!(m.pc(), 0);
        assert_eq!(m.run(50), Ok(()));
        assert_eq!(m.cycles(), 100);
    }

    #[test]
    fn csr_access_traps_with_the_register_number() {
        let mut m = machine(&[I::CSRRS { rd: 10, rs1: 0, csr: 0xF14 }]);
        assert_eq!(m.step(), Err(Trap::UnsupportedCsr { pc: 0, csr: 0xF14 }));
    }

    // -- end to end ----------------------------------------------------------

    #[test]
    fn a_program_can_talk_to_memory_mapped_devices() {
        const CONSOLE: Address = 0x0400_0000;
        const PAD: Address = 0x0500_0000;

        let console = Rc::new(RefCell::new(ConsoleOut::default()));
        let input = Rc::new(RefCell::new(Input::default()));

        // Print "Hi" if A is held, then multiply and stop.
        let program = [
            I::LUI { rd: 5, imm: PAD as i32 },      // 0x00 t0 = input
            I::LHU { rd: 6, rs1: 5, offset: 0x10 }, // 0x04 t1 = pad 0 buttons
            I::ANDI { rd: 6, rs1: 6, imm: Button::A.mask() as i32 }, // 0x08
            I::BEQ { rs1: 6, rs2: 0, offset: 24 },  // 0x0C -> 0x24
            I::LUI { rd: 7, imm: CONSOLE as i32 },  // 0x10 t2 = console
            I::ADDI { rd: 8, rs1: 0, imm: b'H' as i32 }, // 0x14
            I::SB { rs1: 7, rs2: 8, offset: 0 },    // 0x18
            I::ADDI { rd: 8, rs1: 0, imm: b'i' as i32 }, // 0x1C
            I::SB { rs1: 7, rs2: 8, offset: 0 },    // 0x20
            I::ADDI { rd: 10, rs1: 0, imm: 7 },     // 0x24
            I::ADDI { rd: 11, rs1: 0, imm: 1000 },  // 0x28
            I::MUL { rd: 10, rs1: 10, rs2: 11 },    // 0x2C
            I::EBREAK,                              // 0x30
        ];

        let mut bus = Bus::new();
        bus.map_rom(ROM, image(&program)).unwrap();
        bus.map_ram(RAM, 1024).unwrap();
        bus.map_device(CONSOLE, console.clone()).unwrap();
        bus.map_device(PAD, input.clone()).unwrap();
        let mut m = Machine::new(bus);

        // Host pushes input in before the frame runs.
        input
            .borrow_mut()
            .set_pad(0, Pad { buttons: Button::A.mask(), connected: true, ..Pad::default() });

        assert_eq!(m.run(100), Err(Trap::Breakpoint { pc: 0x30 }));
        assert_eq!(m.register(10), 7000);
        assert_eq!(console.borrow_mut().take_string(), "Hi");

        // With A released the guest takes the branch and prints nothing.
        input.borrow_mut().set_pad(0, Pad { connected: true, ..Pad::default() });
        m.set_pc(0);
        assert_eq!(m.run(100), Err(Trap::Breakpoint { pc: 0x30 }));
        assert_eq!(console.borrow_mut().take_string(), "");
    }

    #[test]
    fn a_program_can_walk_a_string_out_of_rom() {
        // Exercises AUIPC, a backward branch, LBU, and a device store —
        // every one of which depended on a PC base or an extension rule.
        const CONSOLE: Address = 0x0400_0000;
        let console = Rc::new(RefCell::new(ConsoleOut::default()));

        let message = b"hello\n\0";
        // The message is appended straight after the code, so its address is
        // the code length — which AUIPC + ADDI reconstructs at run time.
        const CODE_WORDS: i32 = 9;
        let code = [
            I::AUIPC { rd: 5, imm: 0 },                     // 0x00 t0 = 0x00
            I::ADDI { rd: 5, rs1: 5, imm: CODE_WORDS * 4 }, // 0x04 t0 = &message
            I::LUI { rd: 6, imm: CONSOLE as i32 },          // 0x08 t1 = console
            I::LBU { rd: 7, rs1: 5, offset: 0 },            // 0x0C loop:
            I::BEQ { rs1: 7, rs2: 0, offset: 16 },          // 0x10 -> 0x20 done
            I::SB { rs1: 6, rs2: 7, offset: 0 },            // 0x14
            I::ADDI { rd: 5, rs1: 5, imm: 1 },              // 0x18
            I::JAL { rd: 0, offset: -16 },                  // 0x1C -> 0x0C loop
            I::EBREAK,                                      // 0x20 done:
        ];
        assert_eq!(code.len() as i32, CODE_WORDS);

        let mut rom = image(&code);
        rom.extend_from_slice(message);

        let mut bus = Bus::new();
        bus.map_rom(ROM, rom).unwrap();
        bus.map_device(CONSOLE, console.clone()).unwrap();
        let mut m = Machine::new(bus);

        assert_eq!(m.run(200), Err(Trap::Breakpoint { pc: 0x20 }));
        assert_eq!(console.borrow_mut().take_string(), "hello\n");
    }
}
