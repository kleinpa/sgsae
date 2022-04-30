use std::error::Error;
use std::fmt;

use crate::instructions::Instruction;
use crate::types::{Memory, ProgramCounter};

#[derive(Debug)]
pub struct ExecutionError {
    details: String,
}

impl ExecutionError {
    fn new(msg: &str) -> ExecutionError {
        ExecutionError { details: msg.to_string() }
    }
}

impl fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.details)
    }
}

impl Error for ExecutionError {
    fn description(&self) -> &str {
        &self.details
    }
}

struct ArrayMemory<const N: usize> {
    memory: [u8; N],
}
impl<const N: usize> ArrayMemory<N> {
    fn new() -> ArrayMemory<N> {
        return ArrayMemory { memory: [0; N] };
    }
    fn new_with_memory(memory: [u8; N]) -> ArrayMemory<N> {
        return ArrayMemory { memory: memory };
    }
}
impl<const N: usize> Memory for ArrayMemory<N> {
    fn load(&self, a: crate::types::Address) -> u8 {
        return self.memory[usize::try_from(a).unwrap()];
    }

    fn store(&mut self, a: crate::types::Address, x: u8) {
        self.memory[usize::try_from(a).unwrap()] = x;
    }
}

pub struct Machine {
    program_counter: ProgramCounter,
    registers: [u64; 32],
    memory: ArrayMemory<65536>,
}

impl Machine {
    pub fn new() -> Machine {
        Machine { program_counter: 0, registers: [0; 32], memory: ArrayMemory::new() }
    }
    pub fn new_with_memory(memory: [u8; 65536]) -> Machine {
        Machine {
            program_counter: 0,
            registers: [0; 32],
            memory: ArrayMemory::new_with_memory(memory),
        }
    }
    pub fn get_registers(&self) -> [u64; 32] {
        self.registers
    }

    // Constructor for tests and experiments. The provided instructions will be copied into memory and executed.
    pub fn new_with_instructions(instructions: &[Instruction]) -> Machine {
        let mut m = Machine::new();
        let mut i = 0; // start address

        for instruction in instructions {
            let bytes = instruction.encode().to_le_bytes();
            for x in bytes {
                m.memory.store(i, x);
                i = i + 1;
            }
        }
        return m;
    }

    pub fn run(&mut self) -> Result<(), ExecutionError> {
        loop {
            let instruction = self.fetch();

            if let Err(e) = self.execute(instruction) {
                return Err(e);
            };
        }
    }

    fn fetch(&mut self) -> Instruction {
        let data = u32::from_le_bytes([
            self.memory.load((self.program_counter) + 0),
            self.memory.load((self.program_counter) + 1),
            self.memory.load((self.program_counter) + 2),
            self.memory.load((self.program_counter) + 3),
        ]);
        self.program_counter += 4;
        return Instruction::decode(data);
    }

    fn read_register(&self, i: u8) -> u64 {
        if i == 0 {
            0
        } else {
            self.registers[usize::from(i)]
        }
    }
    fn write_register(&mut self, i: u8, x: u64) {
        if i == 0 {
            return;
        }
        let x: u64 = x.into();
        self.registers[usize::from(i)] = x;
    }

    fn execute(&mut self, instruction: Instruction) -> Result<(), ExecutionError> {
        /// Add a signed
        fn addiu(a: u64, b: i32) -> u64 {
            a.wrapping_add(b as u64)
        }
        // execute
        #[allow(unused_variables)]
        match instruction {
            Instruction::LB { rs, rd, offset } => {
                // rd ← s8[rs1 + offset]
                let a = addiu(self.read_register(rs), i32::from(offset));
                self.write_register(rd, i8::from_le_bytes(self.memory.load_array(a)) as u32 as u64);
                Ok(())
            }
            Instruction::LH { rs, rd, offset } => {
                // rd ← s16[rs1 + offset]
                let a = addiu(self.read_register(rs), i32::from(offset));
                self.write_register(
                    rd,
                    u64::from(i16::from_le_bytes(self.memory.load_array(a)) as u32),
                );
                Ok(())
            }
            Instruction::LW { rs, rd, offset } => {
                // rd ← s32[rs1 + offset]
                let a = addiu(self.read_register(rs), i32::from(offset));
                self.write_register(rd, i32::from_le_bytes(self.memory.load_array(a)) as u64);
                Ok(())
            }
            Instruction::LD { rs, rd, offset } => {
                // rd ← u64[rs1 + offset]
                let a = addiu(self.read_register(rs), i32::from(offset));
                self.write_register(rd, i64::from_le_bytes(self.memory.load_array(a)) as u64);
                Ok(())
            }
            Instruction::LBU { rs, rd, offset } => {
                // rd ← u8[rs1 + offset]
                let a = addiu(self.read_register(rs), i32::from(offset));
                self.write_register(rd, u8::from_le_bytes(self.memory.load_array(a)) as u64);
                Ok(())
            }
            Instruction::LHU { rs, rd, offset } => {
                // rd ← u16[rs1 + offset]
                let a = addiu(self.read_register(rs), i32::from(offset));
                self.write_register(rd, u16::from_le_bytes(self.memory.load_array(a)) as u64);
                Ok(())
            }
            Instruction::LWU { rs, rd, offset } => {
                // rd ← u32[rs1 + offset]
                let a = addiu(self.read_register(rs), i32::from(offset));
                self.write_register(rd, u32::from_le_bytes(self.memory.load_array(a)) as u64);
                Ok(())
            }
            Instruction::ADDI { rd, rs, imm } => {
                // rd ← rs1 + sx(imm)
                self.write_register(rd, addiu(self.read_register(rs), i32::from(imm)));
                Ok(())
            }
            Instruction::SLTI { rd, rs, imm } => {
                // rd ← sx(rs1) < sx(imm)
                self.write_register(
                    rd,
                    if (self.read_register(rs) as i64) < i64::from(imm) { 1 } else { 0 },
                );
                Ok(())
            }
            Instruction::SLTIU { rd, rs, imm } => {
                // rd ← ux(rs1) < ux(imm)
                self.write_register(
                    rd,
                    if self.read_register(rs) < u64::from(imm) { 1 } else { 0 },
                );
                Ok(())
            }
            Instruction::XORI { rd, rs, imm } => {
                // rd ← ux(rs1) ⊕ ux(imm)
                self.write_register(rd, self.read_register(rs) ^ u64::from(imm));
                Ok(())
            }
            Instruction::ORI { rd, rs, imm } => {
                // rd ← ux(rs1) ∨ ux(imm)
                self.write_register(rd, self.read_register(rs) | u64::from(imm));
                Ok(())
            }
            Instruction::ANDI { rd, rs, imm } => {
                // rd ← ux(rs1) ∧ ux(imm)
                self.write_register(rd, self.read_register(rs) & u64::from(imm));
                Ok(())
            }
            Instruction::SLLI { rd, rs, imm } => {
                // rd ← ux(rs1) « ux(imm)
                self.write_register(rd, self.read_register(rs).wrapping_shl(u32::from(imm)));
                Ok(())
            }
            Instruction::SRLI { rd, rs, imm } => {
                // rd ← ux(rs1) » ux(imm)
                self.write_register(rd, self.read_register(rs).wrapping_shr(u32::from(imm)));
                Ok(())
            }
            Instruction::SRAI { rd, rs, imm } => {
                // rd ← sx(rs1) » ux(imm)
                self.write_register(
                    rd,
                    (self.read_register(rs) as i64).wrapping_shr(u32::from(imm)) as u64,
                );
                Ok(())
            }
            Instruction::ADDIW { rd, rs, imm } => {
                // rd ← s32(rs1) + imm
                self.write_register(
                    rd,
                    (self.read_register(rs) as i32).wrapping_add(i32::from(imm)) as u64,
                );
                Ok(())
            }
            Instruction::SLLIW { rd, rs, imm } => {
                // rd ← s32(u32(rs1) « imm)
                self.write_register(
                    rd,
                    (self.read_register(rs) as i32).wrapping_shl(u32::from(imm)) as u32 as u64,
                );
                Ok(())
            }
            Instruction::SRLIW { rd, rs, imm } => {
                // rd ← s32(u32(rs1) » imm)
                self.write_register(
                    rd,
                    u64::from((self.read_register(rs) as u32).wrapping_shr(u32::from(imm)) as u32),
                );
                Ok(())
            }
            Instruction::SRAIW { rd, rs, imm } => {
                // rd ← s32(rs1) » imm
                self.write_register(
                    rd,
                    u64::from((self.read_register(rs) as i32).wrapping_shr(u32::from(imm)) as u32),
                );
                Ok(())
            }
            Instruction::JALR { rd, rs, offset } => {
                // rd ← pc + length(inst), pc ← (rs1 + offset) ∧ -2
                let ret = self.program_counter.wrapping_add(4);
                self.program_counter = addiu(self.read_register(rs), i32::from(offset));
                self.write_register(rd, ret);
                Ok(())
            }
            Instruction::JAL { rd, offset } => {
                // rd ← pc + length(inst), pc ← pc + offset
                self.write_register(rd, self.program_counter.wrapping_add(4));
                self.program_counter = addiu(self.program_counter, offset);
                Ok(())
            }
            Instruction::ADD { rd, rs1, rs2 } => {
                // rd ← sx(rs1) + sx(rs2)
                self.write_register(
                    rd,
                    (self.read_register(rs1) as i64).wrapping_add(self.read_register(rs2) as i64)
                        as u64,
                );
                Ok(())
            }
            Instruction::SUB { rd, rs1, rs2 } => {
                // rd ← sx(rs1) - sx(rs2)
                self.write_register(
                    rd,
                    (self.read_register(rs1) as i64).wrapping_sub(self.read_register(rs2) as i64)
                        as u64,
                );
                Ok(())
            }
            Instruction::SLL { rd, rs1, rs2 } => {
                // rd ← ux(rs1) « rs2
                self.write_register(
                    rd,
                    self.read_register(rs1).wrapping_shl(self.read_register(rs2) as u32),
                );
                Ok(())
            }
            Instruction::SLT { rd, rs1, rs2 } => {
                // rd ← sx(rs1) < sx(rs2)
                //self.write_register(rd.
                self.write_register(
                    rd,
                    if (self.read_register(rs1) as i64) < (self.read_register(rs2) as i64) {
                        1
                    } else {
                        0
                    },
                );
                Ok(())
            }
            Instruction::SLTU { rd, rs1, rs2 } => {
                // rd ← ux(rs1) < ux(rs2)
                self.write_register(
                    rd,
                    if self.read_register(rs1) < self.read_register(rs2) { 1 } else { 0 },
                );
                Ok(())
            }
            Instruction::XOR { rd, rs1, rs2 } => {
                // rd ← ux(rs1) ⊕ ux(rs2)
                self.write_register(rd, self.read_register(rs1) ^ self.read_register(rs2));
                Ok(())
            }
            Instruction::SRL { rd, rs1, rs2 } => {
                // rd ← ux(rs1) » rs2
                self.write_register(
                    rd,
                    self.read_register(rs1).wrapping_shr(self.read_register(rs2) as u32),
                );
                Ok(())
            }
            Instruction::SRA { rd, rs1, rs2 } => {
                // rd ← sx(rs1) » rs2
                self.write_register(
                    rd,
                    (self.read_register(rs1) as i64).wrapping_shr(self.read_register(rs2) as u32)
                        as u64,
                );
                Ok(())
            }
            Instruction::OR { rd, rs1, rs2 } => {
                // rd ← ux(rs1) ∨ ux(rs2)
                self.write_register(rd, self.read_register(rs1) | self.read_register(rs2));
                Ok(())
            }
            Instruction::AND { rd, rs1, rs2 } => {
                // rd ← ux(rs1) ∧ ux(rs2)
                self.write_register(rd, self.read_register(rs1) & self.read_register(rs2));
                Ok(())
            }
            Instruction::ADDW { rd, rs1, rs2 } => {
                // rd ← s32(rs1) + s32(rs2)
                self.write_register(
                    rd,
                    (self.read_register(rs1) as i32).wrapping_add(self.read_register(rs2) as i32)
                        as u64,
                );
                Ok(())
            }
            Instruction::SUBW { rd, rs1, rs2 } => {
                // rd ← s32(rs1) - s32(rs2)
                self.write_register(
                    rd,
                    (self.read_register(rs1) as i32).wrapping_sub(self.read_register(rs2) as i32)
                        as u64,
                );
                Ok(())
            }
            Instruction::SLLW { rd, rs1, rs2 } => {
                // rd ← s32(u32(rs1) « rs2)
                self.write_register(
                    rd,
                    (self.read_register(rs1) as u32).wrapping_shl(self.read_register(rs2) as u32)
                        as u64,
                );
                Ok(())
            }
            Instruction::SRLW { rd, rs1, rs2 } => {
                // rd ← s32(u32(rs1) » rs2)
                self.write_register(
                    rd,
                    (self.read_register(rs1) as u32).wrapping_shr(self.read_register(rs2) as u32)
                        as u64,
                );
                Ok(())
            }
            Instruction::SRAW { rd, rs1, rs2 } => {
                // rd ← s32(rs1) » rs2
                self.write_register(
                    rd,
                    (self.read_register(rs1) as i32).wrapping_shr(self.read_register(rs2) as u32)
                        as u32 as u64,
                );
                Ok(())
            }
            Instruction::SB { rs1, rs2, offset } => {
                // u8[rs1 + offset] ← rs2
                let a = addiu(self.read_register(rs1), i32::from(offset));
                self.memory.store_slice(a, &(self.read_register(rs2) as u8).to_le_bytes());
                Ok(())
            }
            Instruction::SH { rs1, rs2, offset } => {
                // u16[rs1 + offset] ← rs2
                let a = addiu(self.read_register(rs1), i32::from(offset));
                self.memory.store_slice(a, &(self.read_register(rs2) as u16).to_le_bytes());
                Ok(())
            }
            Instruction::SW { rs1, rs2, offset } => {
                // u32[rs1 + offset] ← rs2
                let a = addiu(self.read_register(rs1), i32::from(offset));
                self.memory.store_slice(a, &(self.read_register(rs2) as u32).to_le_bytes());
                Ok(())
            }
            Instruction::SD { rs1, rs2, offset } => {
                // u64[rs1 + offset] ← rs2
                let a = addiu(self.read_register(rs1), i32::from(offset));
                self.memory.store_slice(a, &(self.read_register(rs2) as u64).to_le_bytes());
                Ok(())
            }
            Instruction::LUI { rd, imm } => {
                // rd ← imm
                self.write_register(rd, u64::from(imm.wrapping_shl(12)));
                Ok(())
            }
            Instruction::AUIPC { rd, offset } => {
                // rd ← pc + offset
                self.write_register(
                    rd,
                    (((self.program_counter as u32).wrapping_add(offset as u32)) << 12) as u64,
                );
                Ok(())
            }
            Instruction::BEQ { rs1, rs2, offset } => {
                // if rs1 = rs2 then pc ← pc + offset
                if self.read_register(rs1) == self.read_register(rs2) {
                    self.program_counter = addiu(self.program_counter as u64, i32::from(offset));
                }
                Ok(())
            }
            Instruction::BNE { rs1, rs2, offset } => {
                // if rs1 ≠ rs2 then pc ← pc + offset
                if self.read_register(rs1) != self.read_register(rs2) {
                    self.program_counter = addiu(self.program_counter as u64, i32::from(offset));
                }
                Ok(())
            }
            Instruction::BLT { rs1, rs2, offset } => {
                // if rs1 < rs2 then pc ← pc + offset
                if (self.read_register(rs1) as i64) < (self.read_register(rs2) as i64) {
                    self.program_counter = addiu(self.program_counter as u64, i32::from(offset));
                }
                Ok(())
            }
            Instruction::BGE { rs1, rs2, offset } => {
                // if rs1 ≥ rs2 then pc ← pc + offset
                if (self.read_register(rs1) as i64) >= (self.read_register(rs2) as i64) {
                    self.program_counter = addiu(self.program_counter as u64, i32::from(offset));
                }
                Ok(())
            }
            Instruction::BLTU { rs1, rs2, offset } => {
                // if rs1 < rs2 then pc ← pc + offset
                if (self.read_register(rs1) as u64) < (self.read_register(rs2) as u64) {
                    self.program_counter = addiu(self.program_counter as u64, i32::from(offset));
                }
                Ok(())
            }
            Instruction::BGEU { rs1, rs2, offset } => {
                // if rs1 ≥ rs2 then pc ← pc + offset
                if (self.read_register(rs1) as u64) >= (self.read_register(rs2) as u64) {
                    self.program_counter = addiu(self.program_counter as u64, i32::from(offset));
                }
                Ok(())
            }
            Instruction::FENCE { .. } | Instruction::FENCEI {} => {
                Ok(()) /* no-op */
            }
            Instruction::ECALL {} => Err(ExecutionError::new("instruction ECALL not implemented")),
            Instruction::EBREAK {} => {
                Err(ExecutionError::new("instruction EBREAK not implemented"))
            }
            Instruction::CSRRW { .. } => {
                Err(ExecutionError::new("instruction CSRRW not implemented"))
            }
            Instruction::CSRRS { .. } => {
                Err(ExecutionError::new("instruction CSRRS not implemented"))
            }
            Instruction::CSRRC { .. } => {
                Err(ExecutionError::new("instruction CSRRC not implemented"))
            }
            Instruction::CSRRWI { .. } => {
                Err(ExecutionError::new("instruction CSRRWI not implemented"))
            }
            Instruction::CSRRSI { .. } => {
                Err(ExecutionError::new("instruction CSRRSI not implemented"))
            }
            Instruction::CSRRCI { .. } => {
                Err(ExecutionError::new("instruction CSRRCI not implemented"))
            }
            Instruction::UNKNOWN { instruction } => {
                Err(ExecutionError::new(&format!("unknown instruction: {:X}", instruction)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        instructions::Instruction,
        types::{Address, Memory},
    };

    use super::Machine;

    #[test]
    /// Writes to register[0] should not alter it's value
    fn test_zero_register() {
        let mut m = Machine::new();
        assert!(m.execute(Instruction::ADDI { rd: 0, rs: 0, imm: 1 }).is_ok());
        assert_eq!(m.registers[0], 0);
    }

    #[test]
    fn test_lb() {
        fn t(x: u64, offset: i16) -> u64 {
            let mut m = Machine::new();
            let a: u64 = 0x1000;
            m.registers[10] = a as u64 - offset as u64;
            m.memory.store_slice(a as Address, &x.to_le_bytes());
            assert!(m.execute(Instruction::LB { rs: 10, rd: 10, offset: offset }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xF7F6F5F4F3F2F1F0, 0x80), 0xFFFFFFF0)
    }
    #[test]
    fn test_lh() {
        fn t(x: u64, offset: i16) -> u64 {
            let mut m = Machine::new();
            let a = 0x1000;
            m.registers[10] = a as u64 - offset as u64;
            m.memory.store_slice(a as Address, &x.to_le_bytes());
            assert!(m.execute(Instruction::LH { rs: 10, rd: 10, offset: offset }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xF7F6F5F4F3F2F1F0, 0x80), 0xFFFFF1F0)
    }
    #[test]
    fn test_lw() {
        fn t(x: u64, offset: i16) -> u64 {
            let mut m = Machine::new();
            let a = 0x1000;
            m.registers[10] = a as u64 - offset as u64;
            m.memory.store_slice(a as Address, &x.to_le_bytes());
            assert!(m.execute(Instruction::LW { rs: 10, rd: 10, offset: offset }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xF7F6F5F4F3F2F1F0, 0x80), 0xFFFFFFFFF3F2F1F0)
    }
    #[test]
    fn test_ld() {
        fn t(x: u64, offset: i16) -> u64 {
            let mut m = Machine::new();
            let a = 0x1000;
            m.registers[10] = a as u64 - offset as u64;
            m.memory.store_slice(a as Address, &x.to_le_bytes());
            assert!(m.execute(Instruction::LD { rs: 10, rd: 10, offset: offset }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xF7F6F5F4F3F2F1F0, 0x80), 0xF7F6F5F4F3F2F1F0)
    }
    #[test]
    fn test_lbu() {
        fn t(x: u64, offset: i16) -> u64 {
            let mut m = Machine::new();
            let a = 0x1000;
            m.registers[10] = a as u64 - offset as u64;
            m.memory.store_slice(a as Address, &x.to_le_bytes());
            assert!(m.execute(Instruction::LBU { rs: 10, rd: 10, offset: offset }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xF7F6F5F4F3F2F1F0, 0x80), 0x000000F0)
    }
    #[test]
    fn test_lhu() {
        fn t(x: u64, offset: i16) -> u64 {
            let mut m = Machine::new();
            let a = 0x1000;
            m.registers[10] = a as u64 - offset as u64;
            m.memory.store_slice(a as Address, &x.to_le_bytes());
            assert!(m.execute(Instruction::LHU { rs: 10, rd: 10, offset: offset }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xF7F6F5F4F3F2F1F0, 0x80), 0x0000F1F0)
    }
    #[test]
    fn test_lwu() {
        fn t(x: u64, offset: i16) -> u64 {
            let mut m = Machine::new();
            let a = 0x1000;
            m.registers[10] = a as u64 - offset as u64;
            m.memory.store_slice(a as Address, &x.to_le_bytes());
            assert!(m.execute(Instruction::LWU { rs: 10, rd: 10, offset: offset }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xF7F6F5F4F3F2F1F0, 0x80), 0x0000000F3F2F1F0)
    }
    #[test]
    fn test_addi() {
        fn t(a: u64, i: i16) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a;
            assert!(m.execute(Instruction::ADDI { rd: 10, rs: 10, imm: i }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0x1000, 2), 0x1002);
    }
    #[test]
    fn test_slli() {
        fn t(a: u64, i: u8) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a;
            assert!(m.execute(Instruction::SLLI { rd: 10, rs: 10, imm: i }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0x1000, 4), 0x10000);
    }

    #[test]
    fn test_slti() {
        fn t(a: u64, i: i16) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a;
            assert!(m.execute(Instruction::SLTI { rd: 10, rs: 10, imm: i }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(1, -1), 0);
        assert_eq!(t(1, 5), 1);
    }
    #[test]
    fn test_sltiu() {
        fn t(a: u64, i: i16) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a;
            assert!(m.execute(Instruction::SLTIU { rd: 10, rs: 10, imm: i as u16 }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(1, 0), 0);
        assert_eq!(t(66, 9999), 1);
    }

    #[test]
    fn test_xori() {
        fn t(a: u64, i: u16) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a;
            assert!(m.execute(Instruction::XORI { rd: 10, rs: 10, imm: i }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xAA, 0x3C), 0b10010110);
    }
    #[test]
    fn test_srli() {
        fn t(a: u64, i: u8) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a;
            assert!(m.execute(Instruction::SRLI { rd: 10, rs: 10, imm: i }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xA7A6A5A4A3A2A1A0, 24), 0x000000A7A6A5A4A3);
    }
    #[test]
    fn test_srai() {
        fn t(a: u64, i: u8) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a;
            assert!(m.execute(Instruction::SRAI { rd: 10, rs: 10, imm: i }).is_ok());
            m.registers[10]
        }

        assert_eq!(t(0xA7A6A5A4A3A2A1A0, 24), 0xFFFFFFA7A6A5A4A3);
    }
    #[test]
    fn test_ori() {
        fn t(a: u64, i: u16) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a;
            assert!(m.execute(Instruction::ORI { rd: 10, rs: 10, imm: i }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xAA, 0x3C), 0b10111110);
    }
    #[test]
    fn test_andi() {
        fn t(a: u64, i: u16) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a;
            assert!(m.execute(Instruction::ANDI { rd: 10, rs: 10, imm: i }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xAA, 0x3C), 0b00101000);
    }
    #[test]
    fn test_addiw() {
        fn t(a: i64, i: i16) -> i64 {
            let mut m = Machine::new();
            m.registers[10] = a as u64;
            assert!(m.execute(Instruction::ADDIW { rd: 10, rs: 10, imm: i }).is_ok());
            m.registers[10] as i64
        }
        assert_eq!(t(0, -1), -1);
        assert_eq!(t(-1, -1), -2);
    }
    #[test]
    fn test_slliw() {
        fn t(a: u64, i: u8) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a;
            assert!(m.execute(Instruction::SLLIW { rd: 10, rs: 10, imm: i }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xA7A6A5A4A3A2A1A0, 8), 0xA2A1A000);
    }
    #[test]
    fn test_srliw() {
        fn t(a: u64, i: u8) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a;
            assert!(m.execute(Instruction::SRLIW { rd: 10, rs: 10, imm: i }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xA7A6A5A4A3A2A1A0, 8), 0x00A3A2A1);
    }
    #[test]
    fn test_sraiw() {
        fn t(a: u64, i: u8) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a;
            assert!(m.execute(Instruction::SRAIW { rd: 10, rs: 10, imm: i }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xA7A6A5A4A3A2A1A0, 8), 0xFFA3A2A1);
    }

    #[test]
    fn test_jalr() {
        fn t(pc: Address, a: u64, i: i16) -> (Address, u64) {
            let mut m = Machine::new();
            m.program_counter = pc;
            m.registers[10] = a;
            assert!(m.execute(Instruction::JALR { rd: 10, rs: 10, offset: i }).is_ok());
            (m.program_counter, m.registers[10])
        }
        assert_eq!(t(0xF7F6F5F4F3F2F1F0, 0x2000, 7), (0x2007, 0xF7F6F5F4F3F2F1F4));
        assert_eq!(t(0xF7F6F5F4F3F2F1F0, 0x2000, -6), (0x1FFA, 0xF7F6F5F4F3F2F1F4));
    }
    #[test]
    fn test_jal() {
        fn t(pc: Address, offset: i32) -> (Address, u64) {
            let mut m = Machine::new();
            m.program_counter = pc;
            assert!(m.execute(Instruction::JAL { rd: 10, offset: offset }).is_ok());
            (m.program_counter, m.registers[10])
        }
        assert_eq!(t(0xF000, 0xA0), (0xF0A0, 0xF004));

        // j-immediate should drop low bit
    }
    #[test]
    fn test_add() {
        fn t(a: u64, b: u64) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a;
            m.registers[11] = b;
            assert!(m.execute(Instruction::ADD { rd: 10, rs1: 10, rs2: 11 }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0x5756555453525150, 0x5F5E5D5C5B5A5958), 0xB6B4B2B0AEACAAA8);
        assert_eq!(t(22, -1 as i64 as u64), 21);
    }
    #[test]
    fn test_sub() {
        fn t(a: u64, b: u64) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a;
            m.registers[11] = b;
            assert!(m.execute(Instruction::SUB { rd: 10, rs1: 10, rs2: 11 }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0x5F5E5D5C5B5A5958, 0x5756555453525150), 0x0808080808080808);
    }
    #[test]
    fn test_sll() {
        fn t(a: u64, b: u64) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::SLL { rd: 10, rs1: 10, rs2: 11 }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xF7F6F5F4F3F2F1F0, 0), 0xF7F6F5F4F3F2F1F0);
        assert_eq!(t(0xF7F6F5F4F3F2F1F0, 8), 0xF6F5F4F3F2F1F000);
    }
    #[test]
    fn test_slt() {
        fn t(a: i64, b: i64) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::SLT { rd: 10, rs1: 10, rs2: 11 }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(1, -1), 0);
        assert_eq!(t(1, 5), 1);
    }
    #[test]
    fn test_sltu() {
        fn t(a: i64, b: i64) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::SLTU { rd: 10, rs1: 10, rs2: 11 }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(1, 0), 0);
        assert_eq!(t(66, 9999), 1);
    }
    #[test]
    fn test_xor() {
        fn t(a: u64, b: u64) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::XOR { rd: 10, rs1: 10, rs2: 11 }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0b10101010, 0b00111100), 0b10010110);
    }
    #[test]
    fn test_srl() {
        fn t(a: u64, b: u64) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::SRL { rd: 10, rs1: 10, rs2: 11 }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xF7F6F5F4F3F2F1F0, 32), 0x00000000F7F6F5F4);
    }
    #[test]
    fn test_sra() {
        fn t(a: u64, b: u64) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::SRA { rd: 10, rs1: 10, rs2: 11 }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xF7F6F5F4F3F2F1F0, 32), 0xFFFFFFFFF7F6F5F4);
    }
    #[test]
    fn test_or() {
        fn t(a: u64, b: u64) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::OR { rd: 10, rs1: 10, rs2: 11 }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0b10101010, 0b00111100), 0b10111110);
    }
    #[test]
    fn test_and() {
        fn t(a: u64, b: u64) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::AND { rd: 10, rs1: 10, rs2: 11 }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0b10101010, 0b00111100), 0b00101000);
    }
    #[test]
    fn test_addw() {
        fn t(a: u64, b: u64) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::ADDW { rd: 10, rs1: 10, rs2: 11 }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(10, 7), 17);
    }
    #[test]
    fn test_subw() {
        fn t(a: u64, b: u64) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::SUBW { rd: 10, rs1: 10, rs2: 11 }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(10, 7), 3);
    }
    #[test]
    fn test_sllw() {
        fn t(a: u64, b: u64) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::SLLW { rd: 10, rs1: 10, rs2: 11 }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xF7F6F5F4F3F2F1F0, 8), 0x00000000F2F1F000);
    }
    #[test]
    fn test_srlw() {
        fn t(a: u64, b: u64) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::SRLW { rd: 10, rs1: 10, rs2: 11 }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xF7F6F5F4F3F2F1F0, 8), 0x0000000000F3F2F1);
    }
    #[test]
    fn test_sraw() {
        fn t(a: u64, b: u64) -> u64 {
            let mut m = Machine::new();
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::SRAW { rd: 10, rs1: 10, rs2: 11 }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xF7F6F5F4F3F2F1F0, 8), 0x00000000FFF3F2F1);
    }
    #[test]
    fn test_sb() {
        fn t(a: u64, b: u64, i: i16) -> [u8; 8] {
            let mut m = Machine::new();
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::SB { rs1: 10, rs2: 11, offset: i }).is_ok());
            m.memory.load_array(2000)
        }

        assert_eq!(t(2000, 0xA7A6A5A4A3A2A1A0, 0), [0xA0, 0, 0, 0, 0, 0, 0, 0])
    }
    #[test]
    fn test_sh() {
        fn t(a: u64, b: u64, i: i16) -> [u8; 8] {
            let mut m = Machine::new();
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::SH { rs1: 10, rs2: 11, offset: i }).is_ok());
            m.memory.load_array(2000)
        }

        assert_eq!(t(2000, 0xA7A6A5A4A3A2A1A0, 0), [0xA0, 0xA1, 0, 0, 0, 0, 0, 0])
    }
    #[test]
    fn test_sw() {
        fn t(a: u64, b: u64, i: i16) -> [u8; 8] {
            let mut m = Machine::new();
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::SW { rs1: 10, rs2: 11, offset: i }).is_ok());
            m.memory.load_array(2000)
        }

        assert_eq!(t(2000, 0xA7A6A5A4A3A2A1A0, 0), [0xA0, 0xA1, 0xA2, 0xA3, 0, 0, 0, 0])
    }
    #[test]
    fn test_sd() {
        fn t(a: u64, b: u64, i: i16) -> [u8; 8] {
            let mut m = Machine::new();
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::SD { rs1: 10, rs2: 11, offset: i }).is_ok());
            m.memory.load_array(2000)
        }

        assert_eq!(t(2000, 0xA7A6A5A4A3A2A1A0, 0), [0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7])
    }
    #[test]
    fn test_beq() {
        fn t(a: u64, b: u64, offset: i16) -> Address {
            let mut m = Machine::new();
            m.program_counter = 0x1000;
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::BEQ { rs1: 10, rs2: 11, offset: offset }).is_ok());
            m.program_counter
        }
        assert_eq!(t(0xA, 0xA, 0x3FF), 0x13FF);
        assert_eq!(t(0xA, 0xB, 0x3FF), 0x1000);
    }
    #[test]
    fn test_bne() {
        fn t(a: u64, b: u64, offset: i16) -> Address {
            let mut m = Machine::new();
            m.program_counter = 0x1000;
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::BNE { rs1: 10, rs2: 11, offset: offset }).is_ok());
            m.program_counter
        }
        assert_eq!(t(0xA, 0xB, 1), 0x1001);
        assert_eq!(t(0xA, 0xA, 1), 0x1000);
    }
    #[test]
    fn test_blt() {
        fn t(a: u64, b: u64, offset: i16) -> Address {
            let mut m = Machine::new();
            m.program_counter = 0x1000;
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::BLT { rs1: 10, rs2: 11, offset: offset }).is_ok());
            m.program_counter
        }
        assert_eq!(t(0xA, 0xB, 0x1), 0x1001);
        assert_eq!(t(0xB, 0xB, 0x1), 0x1000);
        assert_eq!(t(0xC, 0xB, 0x1), 0x1000);
    }
    #[test]
    fn test_bge() {
        fn t(a: u64, b: u64, offset: i16) -> Address {
            let mut m = Machine::new();
            m.program_counter = 0x1000;
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::BGE { rs1: 10, rs2: 11, offset: offset }).is_ok());
            m.program_counter
        }
        assert_eq!(t(0xA, 0xB, 0x1), 0x1000);
        assert_eq!(t(0xB, 0xB, 0x1), 0x1001);
        assert_eq!(t(0xC, 0xB, 0x1), 0x1001);
    }
    #[test]
    fn test_bltu() {
        fn t(a: u64, b: u64, offset: i16) -> Address {
            let mut m = Machine::new();
            m.program_counter = 0x1000;
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::BLTU { rs1: 10, rs2: 11, offset: offset }).is_ok());
            m.program_counter
        }
        assert_eq!(t(0xA, 0xB, 0x1), 0x1001);
        assert_eq!(t(0xB, 0xB, 0x1), 0x1000);
        assert_eq!(t(0xC, 0xB, 0x1), 0x1000);
    }
    #[test]
    fn test_bgeu() {
        fn t(a: u64, b: u64, offset: i16) -> Address {
            let mut m = Machine::new();
            m.program_counter = 0x1000;
            m.registers[10] = a as u64;
            m.registers[11] = b as u64;
            assert!(m.execute(Instruction::BGEU { rs1: 10, rs2: 11, offset: offset }).is_ok());
            m.program_counter
        }
        assert_eq!(t(0xA, 0xB, 0x1), 0x1000);
        assert_eq!(t(0xB, 0xB, 0x1), 0x1001);
        assert_eq!(t(0xC, 0xB, 0x1), 0x1001);
    }
    #[test]
    fn test_auipc() {
        fn t(offset: u32, pc: Address) -> u64 {
            let mut m = Machine::new();
            m.program_counter = pc;
            assert!(m.execute(Instruction::AUIPC { rd: 10, offset }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xB, 0xA0), 0xAB000);
        assert_eq!(t(0xF0F0F0F0, 0x1), 0x0F0F1000);
    }
    #[test]
    fn test_lui() {
        fn t(offset: u32) -> u64 {
            let mut m = Machine::new();
            assert!(m.execute(Instruction::LUI { rd: 10, imm: offset }).is_ok());
            m.registers[10]
        }
        assert_eq!(t(0xB), 0xB000);
        assert_eq!(t(0xF0F0F0F0), 0x0F0F0000);
    }
    #[test]
    fn test_fence() {
        assert!(Machine::new().execute(Instruction::FENCE { pred: 0, succ: 0 }).is_ok());
    }
    #[test]
    fn test_fencei() {
        assert!(Machine::new().execute(Instruction::FENCEI {}).is_ok());
    }
    #[test]
    fn test_ecall() {
        assert!(Machine::new().execute(Instruction::ECALL {}).is_err());
    }
    #[test]
    fn test_ebreak() {
        assert!(Machine::new().execute(Instruction::EBREAK {}).is_err());
    }
    #[test]
    fn test_csrrw() {
        assert!(Machine::new().execute(Instruction::CSRRW { rd: 10, rs: 10, csr: 0 }).is_err());
    }
    #[test]
    fn test_csrrs() {
        assert!(Machine::new().execute(Instruction::CSRRS { rd: 10, rs: 10, csr: 0 }).is_err());
    }
    #[test]
    fn test_csrrc() {
        assert!(Machine::new().execute(Instruction::CSRRC { rd: 10, rs: 10, csr: 0 }).is_err());
    }
    #[test]
    fn test_csrrwi() {
        assert!(Machine::new().execute(Instruction::CSRRWI { rd: 10, zimm: 0, csr: 0 }).is_err());
    }
    #[test]
    fn test_csrrsi() {
        assert!(Machine::new().execute(Instruction::CSRRSI { rd: 10, zimm: 0, csr: 0 }).is_err());
    }
    #[test]
    fn test_csrrci() {
        assert!(Machine::new().execute(Instruction::CSRRCI { rd: 10, zimm: 0, csr: 0 }).is_err());
    }
    #[test]
    fn test_unknown() {
        assert!(Machine::new().execute(Instruction::UNKNOWN { instruction: 0 }).is_err());
    }
}
