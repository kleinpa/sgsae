use crate::types::*;

mod code {
    use crate::types::*;

    /// Shift a slice of bits from one place in a bitfield to another
    fn bits<T: Into<u64>>(x: T, fr: u8, mask: u8, to: u8) -> u32 {
        ((x.into() as u32 >> fr) & ((1 << mask) - 1)) << to
    }

    // Instruction bitfield access
    pub fn get_opcode(x: u32) -> u8 {
        bits(x, 0, 7, 0) as u8
    }
    pub fn set_opcode(x: u8) -> u32 {
        bits(x, 0, 7, 0)
    }
    pub fn get_funct3(x: u32) -> u8 {
        bits(x, 12, 3, 0) as u8
    }
    pub fn set_funct3(x: u8) -> u32 {
        bits(x, 0, 3, 12)
    }
    pub fn get_funct7(x: u32) -> u8 {
        bits(x, 25, 7, 0) as u8
    }
    pub fn set_funct7(x: u8) -> u32 {
        bits(x, 0, 7, 25)
    }
    pub fn get_rs1(x: u32) -> Register {
        bits(x, 15, 5, 0) as Register
    }
    pub fn set_rs1(x: Register) -> u32 {
        bits(x as u32, 0, 7, 15)
    }
    pub fn get_rs2(x: u32) -> Register {
        bits(x, 20, 5, 0) as Register
    }
    pub fn set_rs2(x: Register) -> u32 {
        bits(x as u32, 0, 7, 20)
    }
    pub fn get_rd(x: u32) -> Register {
        bits(x, 7, 5, 0) as Register
    }
    pub fn set_rd(x: Register) -> u32 {
        bits(x as u32, 0, 7, 7)
    }
    pub fn get_i_imm(x: u32) -> u16 {
        bits(x, 20, 12, 0) as u16
    }
    pub fn set_i_imm(x: u16) -> u32 {
        bits(x as u32, 0, 12, 20)
    }
    pub fn get_s_imm(x: u32) -> u16 {
        (bits(x, 25, 7, 0) + bits(x, 7, 5, 0)) as u16
    }
    pub fn set_s_imm(x: u16) -> u32 {
        bits(x as u32, 5, 7, 25) + bits(x as u32, 0, 5, 7)
    }
    pub fn get_b_imm(x: u32) -> u16 {
        (bits(x, 31, 1, 12) + bits(x, 25, 6, 5) + bits(x, 8, 4, 1) + bits(x, 7, 1, 0)) as u16
    }
    pub fn set_b_imm(x: u16) -> u32 {
        bits(x, 12, 1, 31) + bits(x, 5, 6, 25) + bits(x, 1, 4, 8) + bits(x, 0, 1, 7)
    }
    pub fn get_u_imm(x: u32) -> u32 {
        bits(x, 12, 20, 0)
    }
    pub fn set_u_imm(x: u32) -> u32 {
        bits(x, 0, 20, 12)
    }
    pub fn get_j_imm(x: u32) -> u32 {
        bits(x, 31, 1, 20) + bits(x, 21, 10, 1) + bits(x, 20, 1, 11) + bits(x, 12, 8, 12)
    }
    pub fn set_j_imm(x: u32) -> u32 {
        bits(x, 20, 1, 31) + bits(x, 1, 10, 21) + bits(x, 11, 1, 20) + bits(x, 12, 8, 12)
    }

    pub fn encode_i(opcode: u8, funct3: u8, rd: Register, rs: Register, imm: u16) -> u32 {
        set_i_imm(imm) + set_rs1(rs) + set_funct3(funct3) + set_rd(rd) + set_opcode(opcode)
    }
    pub fn encode_r(
        opcode: u8,
        funct3: u8,
        funct7: u8,
        rd: Register,
        rs1: Register,
        rs2: Register,
    ) -> u32 {
        set_funct7(funct7)
            + set_rs2(rs2)
            + set_rs1(rs1)
            + set_funct3(funct3)
            + set_rd(rd)
            + set_opcode(opcode)
    }

    pub fn encode_s(opcode: u8, funct3: u8, rs1: Register, rs2: Register, imm: u16) -> u32 {
        set_s_imm(imm) + set_rs2(rs2) + set_rs1(rs1) + set_funct3(funct3) + set_opcode(opcode)
    }

    pub fn encode_b(opcode: u8, funct3: u8, rs1: Register, rs2: Register, imm: u16) -> u32 {
        set_b_imm(imm) + set_rs2(rs2) + set_rs1(rs1) + set_funct3(funct3) + set_opcode(opcode)
    }

    pub fn encode_u(opcode: u8, rd: Register, imm: u32) -> u32 {
        set_u_imm(imm) + set_rd(rd) + set_opcode(opcode)
    }
    pub fn encode_j(opcode: u8, rd: Register, imm: u32) -> u32 {
        set_j_imm(imm) + set_rd(rd) + set_opcode(opcode)
    }
}
use code::*;

#[derive(Debug, PartialEq)]

#[rustfmt::skip]
pub enum Instruction {
    UNKNOWN { instruction: u32 },
    /// Upper Immediate
    LUI { rd: Register, imm: u32 },
    /// Add Upper Immediate to PC
    AUIPC { rd: Register, offset: u32 },
    /// Jump and Link
    JAL { rd: Register, offset: u32 },
    /// Jump and Link Register
    JALR { rd: Register, rs: Register, offset: u16 },
    /// Branch Equal
    BEQ { rs1: Register, rs2: Register, offset: u16 },
    /// Branch Not Equal
    BNE { rs1: Register, rs2: Register, offset: u16 },
    /// Branch Less Than
    BLT { rs1: Register, rs2: Register, offset: u16 },
    /// Branch Greater than Equal
    BGE { rs1: Register, rs2: Register, offset: u16 },
    /// Branch Less Than Unsigned
    BLTU { rs1: Register, rs2: Register, offset: u16 },
    /// Branch Greater than Equal Unsigned
    BGEU { rs1: Register, rs2: Register, offset: u16 },
    /// Load Byte
    LB { rs: Register, rd: Register, offset: u16 },
    /// Load Half
    LH { rs: Register, rd: Register, offset: u16 },
    /// Load Word
    LW { rs: Register, rd: Register, offset: u16 },
    /// Load Byte Unsigned
    LBU { rs: Register, rd: Register, offset: u16 },
    /// Load Half Unsigned
    LHU { rs: Register, rd: Register, offset: u16 },
    /// Store Byte
    SB { rs1: Register, rs2: Register, offset: u16 },
    /// Store Half
    SH { rs1: Register, rs2: Register, offset: u16 },
    /// Store Word
    SW { rs1: Register, rs2: Register, offset: u16 },
    /// Add Immediate
    ADDI { rd: Register, rs: Register, imm: u16 },
    /// Set Less Than Immediate
    SLTI { rd: Register, rs: Register, imm: u16 },
    /// Set Less Than Immediate Unsigned
    SLTIU { rd: Register, rs: Register, imm: u16 },
    /// Xor Immediate
    XORI { rd: Register, rs: Register, imm: u16 },
    /// Or Immediate
    ORI { rd: Register, rs: Register, imm: u16 },
    /// And Immediate
    ANDI { rd: Register, rs: Register, imm: u16 },
    /// Shift Left Logical Immediate
    SLLI { rd: Register, rs: Register, imm: u16 },
    /// Shift Right Logical Immediate
    SRLI { rd: Register, rs: Register, imm: u16 },
    /// Shift Right Arithmetic Immediate
    SRAI { rd: Register, rs: Register, imm: u16 },
    /// Add
    ADD { rd: Register, rs1: Register, rs2: Register },
    /// Subtract
    SUB { rd: Register, rs1: Register, rs2: Register },
    /// Shift Left Logical
    SLL { rd: Register, rs1: Register, rs2: Register },
    /// Set Less Than
    SLT { rd: Register, rs1: Register, rs2: Register },
    /// Set Less Than Unsigned
    SLTU { rd: Register, rs1: Register, rs2: Register },
    /// Xor
    XOR { rd: Register, rs1: Register, rs2: Register },
    /// Shift Right Logical
    SRL { rd: Register, rs1: Register, rs2: Register },
    /// Shift Right Arithmetic
    SRA { rd: Register, rs1: Register, rs2: Register },
    /// Or
    OR { rd: Register, rs1: Register, rs2: Register },
    /// And
    AND { rd: Register, rs1: Register, rs2: Register },
    /// Fence
    FENCE { pred: u8, succ: u8 },
    /// Fence Instruction
    FENCEI {},
    /// Load Word Unsigned
    LWU { rs: Register, rd: Register, offset: u16 },
    /// Load Double
    LD { rs: Register, rd: Register, offset: u16 },
    /// Store Double
    SD { rs1: Register, rs2: Register, offset: u16 },
    /// Add Immediate Word
    ADDIW { rd: Register, rs: Register, imm: u16 },
    /// Shift Left Logical Immediate Word
    SLLIW { rd: Register, rs: Register, imm: u16 },
    /// Shift Right Logical Immediate Word
    SRLIW { rd: Register, rs: Register, imm: u16 },
    /// Shift Right Arithmetic Immediate Word
    SRAIW { rd: Register, rs: Register, imm: u16 },
    /// Add Word
    ADDW { rd: Register, rs1: Register, rs2: Register },
    /// Subtract Word
    SUBW { rd: Register, rs1: Register, rs2: Register },
    /// Shift Left Logical Word
    SLLW { rd: Register, rs1: Register, rs2: Register },
    /// Shift Right Logical Word
    SRLW { rd: Register, rs1: Register, rs2: Register },
    /// Shift Right Arithmetic Word
    SRAW { rd: Register, rs1: Register, rs2: Register },
    /// Environment Call
    ECALL {},
    /// Environment Break
    EBREAK {},
    /// CSR Read and Write
    CSRRW { rs:  Register, rd: Register, csr: u16 },
    /// CSR Read and Set Bits
    CSRRS { rs:  Register, rd: Register, csr: u16 },
    /// CSR Read and Clear Bits
    CSRRC { rs:  Register, rd: Register, csr: u16 },
    /// CSR Read and Write bits Immediate
    CSRRWI { zimm: u16, rd: Register, csr: u16 },
    /// CSR Read and Set bits Immediate
    CSRRSI { zimm: u16, rd: Register, csr: u16 },
    /// CSR Read and Clear Immediate
    CSRRCI { zimm: u16, rd: Register, csr: u16 },
}

#[rustfmt::skip]
impl Instruction {
    pub fn decode(x: u32) -> Instruction {
        match get_opcode(x) {
            0b0000011 => match get_funct3(x) {
                0b000 =>              Instruction::LB { rd: get_rd(x), rs: get_rs1(x), offset: get_i_imm(x) },
                0b001 =>              Instruction::LH { rd: get_rd(x), rs: get_rs1(x), offset: get_i_imm(x) },
                0b010 =>              Instruction::LW { rd: get_rd(x), rs: get_rs1(x), offset: get_i_imm(x) },
                0b011 =>              Instruction::LD { rd: get_rd(x), rs: get_rs1(x), offset: get_i_imm(x) },
                0b100 =>              Instruction::LBU { rd: get_rd(x), rs: get_rs1(x), offset: get_i_imm(x) },
                0b101 =>              Instruction::LHU { rd: get_rd(x), rs: get_rs1(x), offset: get_i_imm(x) },
                0b110 =>              Instruction::LWU { rd: get_rd(x), rs: get_rs1(x), offset: get_i_imm(x) },
                _ =>                  Instruction::UNKNOWN { instruction: x },
            },
            0b0010011 => match (get_funct3(x), get_funct7(x)) {
                (0b000, _) =>         Instruction::ADDI { rd: get_rd(x), rs: get_rs1(x), imm: get_i_imm(x) },
                (0b001, 0b0000000) => Instruction::SLLI { rd: get_rd(x), rs: get_rs1(x), imm: get_i_imm(x) },
                (0b010, _) =>         Instruction::SLTI { rd: get_rd(x), rs: get_rs1(x), imm: get_i_imm(x) },
                (0b011, _) =>         Instruction::SLTIU { rd: get_rd(x), rs: get_rs1(x), imm: get_i_imm(x) },
                (0b100, _) =>         Instruction::XORI { rd: get_rd(x), rs: get_rs1(x), imm: get_i_imm(x) },
                (0b101, 0b0000000) => Instruction::SRLI { rd: get_rd(x), rs: get_rs1(x), imm: get_i_imm(x)&0b11111 },
                (0b101, 0b0100000) => Instruction::SRAI { rd: get_rd(x), rs: get_rs1(x), imm: get_i_imm(x)&0b11111 },
                (0b110, _) =>         Instruction::ORI { rd: get_rd(x), rs: get_rs1(x), imm: get_i_imm(x) },
                (0b111, _) =>         Instruction::ANDI { rd: get_rd(x), rs: get_rs1(x), imm: get_i_imm(x) },
                _ =>                  Instruction::UNKNOWN { instruction: x },
            },
            0b0011011 => match (get_funct3(x), get_funct7(x)) {
                (0b000, _) =>         Instruction::ADDIW { imm: get_i_imm(x), rs: get_rs1(x), rd: get_rd(x) },
                (0b001, 0b0000000) => Instruction::SLLIW { imm: get_i_imm(x), rs: get_rs1(x), rd: get_rd(x) },
                (0b101, 0b0000000) => Instruction::SRLIW { imm: get_i_imm(x)&0b11111, rs: get_rs1(x), rd: get_rd(x) },
                (0b101, 0b0100000) => Instruction::SRAIW { imm: get_i_imm(x)&0b11111, rs: get_rs1(x), rd: get_rd(x) },
                _ =>                  Instruction::UNKNOWN { instruction: x },
            },
            0b1100111 => match get_funct3(x) {
                0b000 =>              Instruction::JALR { offset: get_i_imm(x), rs: get_rs1(x), rd: get_rd(x) },
                _ =>                  Instruction::UNKNOWN { instruction: x },
            },
            0b0110011 => match (get_funct3(x), get_funct7(x)) {
                (0b000, 0b0000000) => Instruction::ADD { rs1: get_rs1(x), rs2: get_rs2(x), rd: get_rd(x) },
                (0b000, 0b0100000) => Instruction::SUB { rs1: get_rs1(x), rs2: get_rs2(x), rd: get_rd(x) },
                (0b001, 0b0000000) => Instruction::SLL { rs1: get_rs1(x), rs2: get_rs2(x), rd: get_rd(x) },
                (0b010, 0b0000000) => Instruction::SLT { rs1: get_rs1(x), rs2: get_rs2(x), rd: get_rd(x) },
                (0b011, 0b0000000) => Instruction::SLTU { rs1: get_rs1(x), rs2: get_rs2(x), rd: get_rd(x) },
                (0b100, 0b0000000) => Instruction::XOR { rs1: get_rs1(x), rs2: get_rs2(x), rd: get_rd(x) },
                (0b101, 0b0000000) => Instruction::SRL { rs1: get_rs1(x), rs2: get_rs2(x), rd: get_rd(x) },
                (0b101, 0b0100000) => Instruction::SRA { rs1: get_rs1(x), rs2: get_rs2(x), rd: get_rd(x) },
                (0b110, 0b0000000) => Instruction::OR { rs1: get_rs1(x), rs2: get_rs2(x), rd: get_rd(x) },
                (0b111, 0b0000000) => Instruction::AND { rs1: get_rs1(x), rs2: get_rs2(x), rd: get_rd(x) },
                _ =>                  Instruction::UNKNOWN { instruction: x },
            },
            0b0111011 => match (get_funct3(x), get_funct7(x)) {
                (0b000, 0b0000000) => Instruction::ADDW { rs1: get_rs1(x), rs2: get_rs2(x), rd: get_rd(x) },
                (0b000, 0b0100000) => Instruction::SUBW { rs1: get_rs1(x), rs2: get_rs2(x), rd: get_rd(x) },
                (0b001, 0b0000000) => Instruction::SLLW { rs1: get_rs1(x), rs2: get_rs2(x), rd: get_rd(x) },
                (0b101, 0b0000000) => Instruction::SRLW { rs1: get_rs1(x), rs2: get_rs2(x), rd: get_rd(x) },
                (0b101, 0b0100000) => Instruction::SRAW { rs1: get_rs1(x), rs2: get_rs2(x), rd: get_rd(x) },
                _ =>                  Instruction::UNKNOWN { instruction: x },
            },
            0b0100011 => match get_funct3(x) {
                0b000 =>              Instruction::SB { rs1: get_rs1(x), rs2: get_rs2(x), offset: get_s_imm(x) },
                0b001 =>              Instruction::SH { rs1: get_rs1(x), rs2: get_rs2(x), offset: get_s_imm(x) },
                0b010 =>              Instruction::SW { rs1: get_rs1(x), rs2: get_rs2(x), offset: get_s_imm(x) },
                0b011 =>              Instruction::SD { rs1: get_rs1(x), rs2: get_rs2(x), offset: get_s_imm(x) },
                _ =>                  Instruction::UNKNOWN { instruction: x },
            },
            0b1100011 => match get_funct3(x) {
                0b000 =>              Instruction::BEQ { rs1: get_rs1(x), rs2: get_rs2(x), offset: get_b_imm(x) },
                0b001 =>              Instruction::BNE { rs1: get_rs1(x), rs2: get_rs2(x), offset: get_b_imm(x) },
                0b100 =>              Instruction::BLT { rs1: get_rs1(x), rs2: get_rs2(x), offset: get_b_imm(x) },
                0b101 =>              Instruction::BGE { rs1: get_rs1(x), rs2: get_rs2(x), offset: get_b_imm(x) },
                0b110 =>              Instruction::BLTU { rs1: get_rs1(x), rs2: get_rs2(x), offset: get_b_imm(x) },
                0b111 =>              Instruction::BGEU { rs1: get_rs1(x), rs2: get_rs2(x), offset: get_b_imm(x) },
                _ =>                  Instruction::UNKNOWN { instruction: x },
            },
            0b0010111 =>              Instruction::AUIPC { rd: get_rd(x), offset: get_u_imm(x) },
            0b0110111 =>              Instruction::LUI { rd: get_rd(x), imm: get_u_imm(x) },
            0b1101111 =>              Instruction::JAL { rd: get_rd(x), offset: get_j_imm(x) },
            0b0001111 => match get_funct3(x) {
                0b000 =>              Instruction::FENCE { pred: ((get_i_imm(x)>>4)&0b1111) as u8, succ: (get_i_imm(x)&0b1111) as u8},
                0b001 =>              Instruction::FENCEI {  },
                _ =>                  Instruction::UNKNOWN { instruction: x },
            },
            0b1110011 => match get_funct3(x) {
                0b000 => match x {
                    0x00000073 =>     Instruction::ECALL {},
                    0x00100073 =>     Instruction::EBREAK {},
                    _ =>              Instruction::UNKNOWN { instruction: x },
                },
                0b001 =>              Instruction::CSRRW { rd: get_rd(x), rs: get_rs1(x), csr: get_i_imm(x) },
                0b010 =>              Instruction::CSRRS { rd: get_rd(x), rs: get_rs1(x), csr: get_i_imm(x) },
                0b011 =>              Instruction::CSRRC { rd: get_rd(x), rs: get_rs1(x), csr: get_i_imm(x) },
                0b101 =>              Instruction::CSRRWI { rd: get_rd(x), zimm: get_rs1(x) as u16, csr: get_i_imm(x) },
                0b110 =>              Instruction::CSRRSI { rd: get_rd(x), zimm: get_rs1(x) as u16, csr: get_i_imm(x) },
                0b111 =>              Instruction::CSRRCI { rd: get_rd(x), zimm: get_rs1(x) as u16, csr: get_i_imm(x) },
                _ =>                  Instruction::UNKNOWN { instruction: x },
            },
            _ => Instruction::UNKNOWN { instruction: x },
        }
    }

    pub fn encode(&self) -> u32 {
        match self {
            Instruction::LB { rd, offset, rs } => encode_i(0b0000011, 0b000, *rd, *rs, *offset),
            Instruction::LH { rd, offset, rs } => encode_i(0b0000011, 0b001, *rd, *rs, *offset),
            Instruction::LW { rd, offset, rs } => encode_i(0b0000011, 0b010, *rd, *rs, *offset),
            Instruction::LD { rd, offset, rs } => encode_i(0b0000011, 0b011, *rd, *rs, *offset),
            Instruction::LBU { rd, offset, rs } => encode_i(0b0000011, 0b100, *rd, *rs, *offset),
            Instruction::LHU { rd, offset, rs } => encode_i(0b0000011, 0b101, *rd, *rs, *offset),
            Instruction::LWU { rd, offset, rs } => encode_i(0b0000011, 0b110, *rd, *rs, *offset),
            Instruction::ADDI { rd, rs, imm } => encode_i(0b0010011, 0b000, *rd, *rs, *imm),
            Instruction::SLLI { rd, rs, imm } => encode_i(0b0010011, 0b001, *rd, *rs, *imm),
            Instruction::SLTI { rd, rs, imm } => encode_i(0b0010011, 0b010, *rd, *rs, *imm),
            Instruction::SLTIU { rd, rs, imm } => encode_i(0b0010011, 0b011, *rd, *rs, *imm),
            Instruction::XORI { rd, rs, imm } => encode_i(0b0010011, 0b100, *rd, *rs, *imm),
            Instruction::SRLI { rd, rs, imm } => encode_i(0b0010011, 0b101, *rd, *rs, *imm & 0b11111),
            Instruction::SRAI { rd, rs, imm } => encode_i(0b0010011, 0b101, *rd, *rs, (*imm & 0b11111)|(1<<10)),
            Instruction::ORI { rd, rs, imm } => encode_i(0b0010011, 0b110, *rd, *rs, *imm),
            Instruction::ANDI { rd, rs, imm } => encode_i(0b0010011, 0b111, *rd, *rs, *imm),
            Instruction::ADDIW { rd, rs, imm } => encode_i(0b0011011, 0b000, *rd, *rs, *imm),
            Instruction::SLLIW { rd, rs, imm } => encode_i(0b0011011, 0b001, *rd, *rs, *imm & 0b11111),
            Instruction::SRLIW { rd, rs, imm } => encode_i(0b0011011, 0b101, *rd, *rs, *imm & 0b11111),
            Instruction::SRAIW { rd, rs, imm } => encode_i(0b0011011, 0b101, *rd, *rs, (*imm & 0b11111)|(1<<10)),
            Instruction::JALR { rd, rs, offset } => encode_i(0b1100111, 0b000, *rd, *rs, *offset),
            Instruction::ADD { rd, rs1, rs2 } => encode_r(0b0110011, 0b000, 0b0000000, *rd, *rs1, *rs2),
            Instruction::SUB { rd, rs1, rs2 } => encode_r(0b0110011, 0b000, 0b0100000, *rd, *rs1, *rs2),
            Instruction::SLL { rd, rs1, rs2 } => encode_r(0b0110011, 0b001, 0b0000000, *rd, *rs1, *rs2),
            Instruction::SLT { rd, rs1, rs2 } => encode_r(0b0110011, 0b010, 0b0000000, *rd, *rs1, *rs2),
            Instruction::SLTU { rd, rs1, rs2 } => encode_r(0b0110011, 0b011, 0b0000000, *rd, *rs1, *rs2),
            Instruction::XOR { rd, rs1, rs2 } => encode_r(0b0110011, 0b100, 0b0000000, *rd, *rs1, *rs2),
            Instruction::SRL { rd, rs1, rs2 } => encode_r(0b0110011, 0b101, 0b0000000, *rd, *rs1, *rs2),
            Instruction::SRA { rd, rs1, rs2 } => encode_r(0b0110011, 0b101, 0b0100000, *rd, *rs1, *rs2),
            Instruction::OR { rd, rs1, rs2 } => encode_r(0b0110011, 0b110, 0b0000000, *rd, *rs1, *rs2),
            Instruction::AND { rd, rs1, rs2 } => encode_r(0b0110011, 0b111, 0b0000000, *rd, *rs1, *rs2),
            Instruction::ADDW { rd, rs1, rs2 } => encode_r(0b0111011, 0b000, 0b0000000, *rd, *rs1, *rs2),
            Instruction::SUBW { rd, rs1, rs2 } => encode_r(0b0111011, 0b000, 0b0100000, *rd, *rs1, *rs2),
            Instruction::SLLW { rd, rs1, rs2 } => encode_r(0b0111011, 0b001, 0b0000000, *rd, *rs1, *rs2),
            Instruction::SRLW { rd, rs1, rs2 } => encode_r(0b0111011, 0b101, 0b0000000, *rd, *rs1, *rs2),
            Instruction::SRAW { rd, rs1, rs2 } => encode_r(0b0111011, 0b101, 0b0100000, *rd, *rs1, *rs2),
            Instruction::SB { rs2, offset, rs1 } => encode_s(0b0100011, 0b000, *rs1, *rs2, *offset),
            Instruction::SH { offset, rs1, rs2 } => encode_s(0b0100011, 0b001, *rs1, *rs2, *offset),
            Instruction::SW { offset, rs1, rs2 } => encode_s(0b0100011, 0b010, *rs1, *rs2, *offset),
            Instruction::SD { offset, rs1, rs2 } => encode_s(0b0100011, 0b011, *rs1, *rs2, *offset),
            Instruction::BEQ { rs1, rs2, offset } => encode_b(0b1100011, 0b000, *rs1, *rs2, *offset),
            Instruction::BNE { rs1, rs2, offset } => encode_b(0b1100011, 0b001, *rs1, *rs2, *offset),
            Instruction::BLT { rs1, rs2, offset } => encode_b(0b1100011, 0b100, *rs1, *rs2, *offset),
            Instruction::BGE { rs1, rs2, offset } => encode_b(0b1100011, 0b101, *rs1, *rs2, *offset),
            Instruction::BLTU { rs1, rs2, offset } => encode_b(0b1100011, 0b110, *rs1, *rs2, *offset),
            Instruction::BGEU { rs1, rs2, offset } => encode_b(0b1100011, 0b111, *rs1, *rs2, *offset),
            Instruction::AUIPC { rd, offset } => encode_u(0b0010111, *rd, *offset),
            Instruction::LUI { rd, imm } => encode_u(0b0110111, *rd, *imm),
            Instruction::JAL { rd, offset } => encode_j(0b1101111, *rd, *offset),
            Instruction::FENCE { pred, succ } =>encode_i(0b0001111,0b000,0,0,(((*pred as u16)&0b1111)<<4)+((*succ as u16)&0b1111)), // pred and succ?
            Instruction::FENCEI {} =>encode_i(0b0001111,0b001,0,0,0),
            Instruction::ECALL {} => 0b00000000000000000000000001110011,
            Instruction::EBREAK {} => 0b00000000000100000000000001110011,
            Instruction::CSRRW { rs, rd, csr } => encode_i(0b1110011, 0b001, *rd, *rs, *csr),
            Instruction::CSRRS { rs, rd, csr } => encode_i(0b1110011, 0b010, *rd, *rs, *csr),
            Instruction::CSRRC { rs, rd, csr } => encode_i(0b1110011, 0b011, *rd, *rs, *csr),
            Instruction::CSRRWI { zimm, rd, csr } => encode_i(0b1110011, 0b101, *rd, *zimm as usize, *csr),
            Instruction::CSRRSI { zimm, rd, csr } => encode_i(0b1110011, 0b110, *rd, *zimm as usize, *csr),
            Instruction::CSRRCI { zimm, rd, csr } => encode_i(0b1110011, 0b111, *rd, *zimm as usize, *csr),
            x => panic!("no encoding for instruction: {:?}", x)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rustfmt::skip]
    static INSTRUCTIONS: &[(u32, Instruction)] = &[
        (0x00858503, Instruction::LB { rs: 11, rd: 10, offset: 8 }),
        (0x00859503, Instruction::LH { rs: 11, rd: 10, offset: 8 }),
        (0x0085a503, Instruction::LW { rs: 11, rd: 10, offset: 8 }),
        (0x0085b503, Instruction::LD { rs: 11, rd: 10, offset: 8 }),
        (0x0085c503, Instruction::LBU { rs: 11, rd: 10, offset: 8 }),
        (0x0085d503, Instruction::LHU { rs: 11, rd: 10, offset: 8 }),
        (0x0085e503, Instruction::LWU { rs: 11, rd: 10, offset: 8 }),
        (0x01058513, Instruction::ADDI { rd: 10, rs: 11, imm: 16 }),
        (0x01059513, Instruction::SLLI { rd: 10, rs: 11, imm: 16 }),
        (0x0105a513, Instruction::SLTI { rd: 10, rs: 11, imm: 16 }),
        (0x0105b513, Instruction::SLTIU { rd: 10, rs: 11, imm: 16 }),
        (0x0105c513, Instruction::XORI { rd: 10, rs: 11, imm: 16 }),
        (0x0105d513, Instruction::SRLI { rd: 10, rs: 11, imm: 16 }),
        (0x4105d513, Instruction::SRAI { rd: 10, rs: 11, imm: 16 }),
        (0x0105e513, Instruction::ORI { rd: 10, rs: 11, imm: 16 }),
        (0x0105f513, Instruction::ANDI { rd: 10, rs: 11, imm: 16 }),
        (0x00008517, Instruction::AUIPC { rd: 10, offset: 8 }),
        (0x00010537, Instruction::LUI { rd: 10, imm: 16 }),
        (0x0000056f, Instruction::JAL { rd: 10, offset: 0 }),
        (0x0105851b, Instruction::ADDIW { rd: 10, rs: 11, imm: 16 }),
        (0x0105951b, Instruction::SLLIW { rd: 10, rs: 11, imm: 16 }),
        (0x0105d51b, Instruction::SRLIW { rd: 10, rs: 11, imm: 16 }),
        (0x4105d51b, Instruction::SRAIW { rd: 10, rs: 11, imm: 16 }),
        (0x00c58463, Instruction::BEQ { rs1: 11, rs2: 12, offset: 8 }),
        (0x00c59463, Instruction::BNE { rs1: 11, rs2: 12, offset: 8 }),
        (0x00c5c463, Instruction::BLT { rs1: 11, rs2: 12, offset: 8 }),
        (0x00c5d463, Instruction::BGE { rs1: 11, rs2: 12, offset: 8 }),
        (0x00c5e463, Instruction::BLTU { rs1: 11, rs2: 12, offset: 8 }),
        (0x00c5f463, Instruction::BGEU { rs1: 11, rs2: 12, offset: 8 }),
        (0x00c58423, Instruction::SB { rs1: 11, rs2: 12, offset: 8 }),
        (0x00c59423, Instruction::SH { rs1: 11, rs2: 12, offset: 8 }),
        (0x00c5a423, Instruction::SW { rs1: 11, rs2: 12, offset: 8 }),
        (0x00c5b423, Instruction::SD { rs1: 11, rs2: 12, offset: 8 }),
        (0x00c58533, Instruction::ADD { rd: 10, rs1: 11, rs2: 12 }),
        (0x40c58533, Instruction::SUB { rd: 10, rs1: 11, rs2: 12 }),
        (0x00c59533, Instruction::SLL { rd: 10, rs1: 11, rs2: 12 }),
        (0x00c5a533, Instruction::SLT { rd: 10, rs1: 11, rs2: 12 }),
        (0x00c5b533, Instruction::SLTU { rd: 10, rs1: 11, rs2: 12 }),
        (0x00c5c533, Instruction::XOR { rd: 10, rs1: 11, rs2: 12 }),
        (0x00c5d533, Instruction::SRL { rd: 10, rs1: 11, rs2: 12 }),
        (0x40c5d533, Instruction::SRA { rd: 10, rs1: 11, rs2: 12 }),
        (0x00c5e533, Instruction::OR { rd: 10, rs1: 11, rs2: 12 }),
        (0x00c5f533, Instruction::AND { rd: 10, rs1: 11, rs2: 12 }),
        (0x00c5853b, Instruction::ADDW { rd: 10, rs1: 11, rs2: 12 }),
        (0x40c5853b, Instruction::SUBW { rd: 10, rs1: 11, rs2: 12 }),
        (0x00c5953b, Instruction::SLLW { rd: 10, rs1: 11, rs2: 12 }),
        (0x00c5d53b, Instruction::SRLW { rd: 10, rs1: 11, rs2: 12 }),
        (0x40c5d53b, Instruction::SRAW { rd: 10, rs1: 11, rs2: 12 }),
        (0x00858567, Instruction::JALR { rd: 10, rs: 11, offset: 8 }),
        (0x0ff0000f, Instruction::FENCE { pred: 15, succ: 15 }),
        (0x0000100f, Instruction::FENCEI {}),
        (0x00000073, Instruction::ECALL {}),
        (0x00100073, Instruction::EBREAK {}),
        (0x00059573, Instruction::CSRRW { rs: 11, rd: 10, csr: 0 }),
        (0x0015a573, Instruction::CSRRS { rs: 11, rd: 10, csr: 1 }),
        (0x0025b573, Instruction::CSRRC { rs: 11, rd: 10, csr: 2 }),
        (0x00385573, Instruction::CSRRWI { zimm: 16, rd: 10, csr: 3 }),
        (0x00486573, Instruction::CSRRSI { zimm: 16, rd: 10, csr: 4 }),
        (0x00587573, Instruction::CSRRCI { zimm: 16, rd: 10, csr: 5 }),
    ];

    #[test]
    fn decode() {
        for (x, i) in INSTRUCTIONS {
            assert_eq!(&Instruction::decode(*x), i, "decode {:?}", i);
        }
    }
    #[test]
    fn encode() {
        for (x, i) in INSTRUCTIONS {
            assert_eq!(&i.encode(), x, "encode {:?}", i);
        }
    }
}
