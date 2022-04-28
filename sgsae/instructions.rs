use crate::types::*;

mod code {
    use crate::types::*;

    // The following types are currently not needed:
    //   AMOType with 'aqrl' var
    //   ISHWType with 'shamtw' var
    //   CsrType
    //   R4Type with 'rs3' var

    /// Shift a slice of bits from one place in a bitfield to another
    fn bits<T: Into<u64>>(x: T, fr: u8, mask: u8, to: u8) -> u32 {
        ((x.into() as u32 >> fr) & ((1 << mask) - 1)) << to
    }

    // Instruction bitfield access
    pub fn get_opcode(x: u32) -> u8 {
        bits(x, 0, 7, 0) as u8
    }
    fn set_opcode(x: u8) -> u32 {
        bits(x, 0, 7, 0)
    }
    fn get_funct3(x: u32) -> u8 {
        bits(x, 12, 3, 0) as u8
    }
    fn set_funct3(x: u8) -> u32 {
        bits(x, 0, 3, 12)
    }
    fn get_funct7(x: u32) -> u8 {
        bits(x, 25, 7, 0) as u8
    }
    fn set_funct7(x: u8) -> u32 {
        bits(x, 0, 7, 25)
    }
    fn get_rs1(x: u32) -> Register {
        bits(x, 15, 5, 0) as Register
    }
    fn set_rs1(x: Register) -> u32 {
        bits(x as u32, 0, 7, 15)
    }
    fn get_rs2(x: u32) -> Register {
        bits(x, 20, 5, 0) as Register
    }
    fn set_rs2(x: Register) -> u32 {
        bits(x as u32, 0, 7, 20)
    }
    fn get_rd(x: u32) -> Register {
        bits(x, 7, 5, 0) as Register
    }
    fn set_rd(x: Register) -> u32 {
        bits(x as u32, 0, 7, 7)
    }
    fn get_i_imm(x: u32) -> u16 {
        bits(x, 20, 12, 0) as u16
    }
    fn set_i_imm(x: u16) -> u32 {
        bits(x as u32, 0, 12, 20)
    }
    fn get_s_imm(x: u32) -> u16 {
        (bits(x, 25, 7, 0) | bits(x, 7, 5, 0)) as u16
    }
    fn set_s_imm(x: u16) -> u32 {
        bits(x as u32, 5, 7, 25) | bits(x as u32, 0, 5, 7)
    }
    fn get_b_imm(x: u32) -> u16 {
        (bits(x, 31, 1, 12) | bits(x, 25, 6, 5) | bits(x, 8, 4, 1) | bits(x, 7, 1, 0)) as u16
    }
    fn set_b_imm(x: u16) -> u32 {
        bits(x, 12, 1, 31) | bits(x, 5, 6, 25) | bits(x, 1, 4, 8) | bits(x, 0, 1, 7)
    }
    fn get_u_imm(x: u32) -> u32 {
        bits(x, 12, 20, 0)
    }
    fn set_u_imm(x: u32) -> u32 {
        bits(x, 0, 20, 12)
    }
    fn get_j_imm(x: u32) -> u32 {
        bits(x, 31, 1, 20) | bits(x, 21, 10, 1) | bits(x, 20, 1, 11) | bits(x, 12, 8, 12)
    }
    fn set_j_imm(x: u32) -> u32 {
        bits(x, 20, 1, 31) | bits(x, 1, 10, 21) | bits(x, 11, 1, 20) | bits(x, 12, 8, 12)
    }

    pub struct IType {
        pub funct3: u8,
        pub rd: Register,
        pub rs: Register,
        pub imm: u16,
    }
    impl IType {
        pub fn encode(opcode: u8, funct3: u8, rd: Register, rs: Register, imm: u16) -> u32 {
            set_i_imm(imm) | set_rs1(rs) | set_funct3(funct3) | set_rd(rd) | set_opcode(opcode)
        }

        pub fn decode(instruction: u32) -> IType {
            return IType {
                funct3: get_funct3(instruction),
                rd: get_rd(instruction),
                rs: get_rs1(instruction),
                imm: get_i_imm(instruction),
            };
        }
    }

    pub struct IshType {
        pub funct3: u8,
        pub rd: Register,
        pub rs: Register,
        pub shamt: u8,
        pub ish: u8,
    }
    impl IshType {
        pub fn encode(
            opcode: u8,
            funct3: u8,
            rd: Register,
            rs: Register,
            shamt: u8,
            ish: u8,
        ) -> u32 {
            if shamt > 0b11111 {
                panic!("shamt out of range");
            }
            bits(shamt, 0, 5, 20)
                | bits(ish, 0, 7, 25)
                | set_rs1(rs)
                | set_funct3(funct3)
                | set_rd(rd)
                | set_opcode(opcode)
        }
        pub fn decode(instruction: u32) -> IshType {
            return IshType {
                funct3: get_funct3(instruction),
                rd: get_rd(instruction),
                rs: get_rs1(instruction),
                shamt: bits(instruction, 20, 5, 0) as u8,
                ish: bits(instruction, 25, 7, 0) as u8,
            };
        }
    }

    pub struct RType {
        pub funct3: u8,
        pub funct7: u8,
        pub rd: Register,
        pub rs1: Register,
        pub rs2: Register,
    }
    impl RType {
        pub fn decode(instruction: u32) -> RType {
            return RType {
                funct3: get_funct3(instruction),
                funct7: get_funct7(instruction),
                rd: get_rd(instruction),
                rs1: get_rs1(instruction),
                rs2: get_rs2(instruction),
            };
        }

        pub fn encode(
            opcode: u8,
            funct3: u8,
            funct7: u8,
            rd: Register,
            rs1: Register,
            rs2: Register,
        ) -> u32 {
            set_funct7(funct7)
                | set_rs2(rs2)
                | set_rs1(rs1)
                | set_funct3(funct3)
                | set_rd(rd)
                | set_opcode(opcode)
        }
    }

    pub struct SType {
        pub funct3: u8,
        pub rs1: Register,
        pub rs2: Register,
        pub imm: u16,
    }
    impl SType {
        pub fn decode(instruction: u32) -> SType {
            return SType {
                funct3: get_funct3(instruction),
                rs1: get_rs1(instruction),
                rs2: get_rs2(instruction),
                imm: get_s_imm(instruction),
            };
        }

        pub fn encode(opcode: u8, funct3: u8, rs1: Register, rs2: Register, imm: u16) -> u32 {
            set_s_imm(imm) | set_rs2(rs2) | set_rs1(rs1) | set_funct3(funct3) | set_opcode(opcode)
        }
    }

    pub struct SBType {
        pub funct3: u8,
        pub rs1: Register,
        pub rs2: Register,
        pub imm: u16,
    }
    impl SBType {
        pub fn decode(instruction: u32) -> SBType {
            return SBType {
                funct3: get_funct3(instruction),
                rs1: get_rs1(instruction),
                rs2: get_rs2(instruction),
                imm: get_b_imm(instruction),
            };
        }

        pub fn encode(opcode: u8, funct3: u8, rs1: Register, rs2: Register, imm: u16) -> u32 {
            set_b_imm(imm) | set_rs2(rs2) | set_rs1(rs1) | set_funct3(funct3) | set_opcode(opcode)
        }
    }

    pub struct UType {
        pub rd: Register,
        pub imm: u32,
    }
    impl UType {
        pub fn decode(instruction: u32) -> UType {
            return UType { rd: get_rd(instruction), imm: get_u_imm(instruction) };
        }

        pub fn encode(opcode: u8, rd: Register, imm: u32) -> u32 {
            set_u_imm(imm) | set_rd(rd) | set_opcode(opcode)
        }
    }

    pub struct UJType {
        pub rd: Register,
        pub offset: u32,
    }
    impl UJType {
        pub fn decode(instruction: u32) -> UJType {
            return UJType { rd: get_rd(instruction), offset: get_j_imm(instruction) };
        }
        pub fn encode(opcode: u8, rd: Register, imm: u32) -> u32 {
            set_j_imm(imm) | set_rd(rd) | set_opcode(opcode)
        }
    }

    pub struct FenceType {
        pub funct3: u8,
        pub pred: u8,
        pub succ: u8,
    }
    impl FenceType {
        pub fn decode(instruction: u32) -> FenceType {
            return FenceType {
                funct3: get_funct3(instruction),
                pred: bits(instruction, 24, 4, 0) as u8,
                succ: bits(instruction, 20, 4, 0) as u8,
            };
        }
        pub fn encode(opcode: u8, funct3: u8, pred: u8, succ: u8) -> u32 {
            return {
                bits(pred, 0, 4, 24)
                    | bits(succ, 0, 4, 20)
                    | set_rs1(0)
                    | set_funct3(funct3)
                    | set_rd(0)
                    | set_opcode(opcode)
            };
        }
    }
}
use code::*;

#[derive(Debug, PartialEq)]

#[rustfmt::skip]
pub enum Instruction {
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
    SLLI { rd: Register, rs: Register, imm: u8 },
    /// Shift Right Logical Immediate
    SRLI { rd: Register, rs: Register, imm: u8 },
    /// Shift Right Arithmetic Immediate
    SRAI { rd: Register, rs: Register, imm: u8 },
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
    /// Load Word Unsigned
    LWU { rs: Register, rd: Register, offset: u16 },
    /// Load Double
    LD { rs: Register, rd: Register, offset: u16 },
    /// Store Double
    SD { rs1: Register, rs2: Register, offset: u16 },
    /// Add Immediate Word
    ADDIW { rd: Register, rs: Register, imm: u16 },
    /// Shift Left Logical Immediate Word
    SLLIW { rd: Register, rs: Register, imm: u8 },
    /// Shift Right Logical Immediate Word
    SRLIW { rd: Register, rs: Register, imm: u8 },
    /// Shift Right Arithmetic Immediate Word
    SRAIW { rd: Register, rs: Register, imm: u8 },
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
    /// Fence
    FENCE { pred: u8, succ: u8 },
    /// Fence Instruction
    FENCEI {},
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

    UNKNOWN { instruction: u32 },
}

#[rustfmt::skip]
impl Instruction {
    pub fn decode(x: u32) -> Instruction {
        match get_opcode(x) {
            0b0000011 => match IType::decode(x) {
                code::IType { funct3: 0b000, rd, rs, imm } => Instruction::LB { rd: rd, rs: rs, offset: imm },
                code::IType { funct3: 0b001, rd, rs, imm } => Instruction::LH { rd: rd, rs: rs, offset: imm },
                code::IType { funct3: 0b010, rd, rs, imm } => Instruction::LW { rd: rd, rs: rs, offset: imm },
                code::IType { funct3: 0b011, rd, rs, imm } => Instruction::LD { rd: rd, rs: rs, offset: imm },
                code::IType { funct3: 0b100, rd, rs, imm } => Instruction::LBU { rd: rd, rs: rs, offset: imm },
                code::IType { funct3: 0b101, rd, rs, imm } => Instruction::LHU { rd: rd, rs: rs, offset: imm },
                code::IType { funct3: 0b110, rd, rs, imm } => Instruction::LWU { rd: rd, rs: rs, offset: imm },
                _ => Instruction::UNKNOWN { instruction: x },
            },
            0b0010011 => match IType::decode(x) {
                code::IType { funct3: 0b000, rd, rs, imm } => Instruction::ADDI { rd: rd, rs: rs, imm: imm },
                code::IType { funct3: 0b001, .. } => match IshType::decode(x) {
                    code::IshType { ish: 0b0000000, rd, rs, shamt, .. } => Instruction::SLLI { rd: rd, rs: rs, imm: shamt },
                    _  => Instruction::UNKNOWN { instruction: x },
                }
                code::IType { funct3: 0b010, rd, rs, imm } => Instruction::SLTI { rd: rd, rs: rs, imm: imm },
                code::IType { funct3: 0b011, rd, rs, imm } => Instruction::SLTIU { rd: rd, rs: rs, imm: imm },
                code::IType { funct3: 0b100, rd, rs, imm } => Instruction::XORI { rd: rd, rs: rs, imm: imm },
                code::IType { funct3: 0b101, .. } => match IshType::decode(x) {
                    code::IshType { ish: 0b0000000, shamt, rs, rd, .. } => Instruction::SRLI { rd: rd, rs: rs, imm: shamt },
                    code::IshType { ish: 0b0100000, shamt, rs, rd, .. } => Instruction::SRAI { rd: rd, rs: rs, imm: shamt },
                    _  => Instruction::UNKNOWN { instruction: x },
                }
                code::IType { funct3: 0b110, rd, rs, imm } => Instruction::ORI { rd: rd, rs: rs, imm: imm },
                code::IType { funct3: 0b111, rd, rs, imm } => Instruction::ANDI { rd: rd, rs: rs, imm: imm },
                _  => Instruction::UNKNOWN { instruction: x },
            },
            0b0011011 => match IType::decode(x) {
                code::IType { funct3: 0b000, rd, rs, imm } => Instruction::ADDIW { imm: imm, rs: rs, rd: rd },
                code::IType { funct3: 0b001, .. } => match IshType::decode(x) {
                     code::IshType { ish: 0b0000000, shamt, rs, rd, .. } => Instruction::SLLIW { imm: shamt, rs: rs, rd: rd }, // assert shamt[5]==0
                     _  => Instruction::UNKNOWN { instruction: x },
                }
                code::IType { funct3: 0b101, .. } => match IshType::decode(x) {
                     code::IshType { ish: 0b0000000, shamt, rs, rd, .. } => Instruction::SRLIW { imm: shamt, rs: rs, rd: rd },// assert shamt[5]==0
                     code::IshType { ish: 0b0100000, shamt, rs, rd, .. } => Instruction::SRAIW { imm: shamt, rs: rs, rd: rd },// assert shamt[5]==0
                     _  => Instruction::UNKNOWN { instruction: x },
                }
                _  => Instruction::UNKNOWN { instruction: x },
            },
            0b1100111 => match IType::decode(x) {
                code::IType { funct3: 0b000, rd, rs, imm } => Instruction::JALR  { rd: rd, rs: rs, offset: imm },
                 _  => Instruction::UNKNOWN { instruction: x },
            },
            0b0110011 => match RType::decode(x) {
                code::RType { funct3: 0b000, funct7: 0b0000000, rd, rs1, rs2 } => Instruction::ADD { rs1: rs1, rs2: rs2, rd: rd },
                code::RType { funct3: 0b000, funct7: 0b0100000, rd, rs1, rs2 } => Instruction::SUB { rs1: rs1, rs2: rs2, rd: rd },
                code::RType { funct3: 0b001, funct7: 0b0000000, rd, rs1, rs2 } => Instruction::SLL { rs1: rs1, rs2: rs2, rd: rd },
                code::RType { funct3: 0b010, funct7: 0b0000000, rd, rs1, rs2 } => Instruction::SLT { rs1: rs1, rs2: rs2, rd: rd },
                code::RType { funct3: 0b011, funct7: 0b0000000, rd, rs1, rs2 } => Instruction::SLTU { rs1: rs1, rs2: rs2, rd: rd },
                code::RType { funct3: 0b100, funct7: 0b0000000, rd, rs1, rs2 } => Instruction::XOR { rs1: rs1, rs2: rs2, rd: rd },
                code::RType { funct3: 0b101, funct7: 0b0000000, rd, rs1, rs2 } => Instruction::SRL { rs1: rs1, rs2: rs2, rd: rd },
                code::RType { funct3: 0b101, funct7: 0b0100000, rd, rs1, rs2 } => Instruction::SRA { rs1: rs1, rs2: rs2, rd: rd },
                code::RType { funct3: 0b110, funct7: 0b0000000, rd, rs1, rs2 } => Instruction::OR { rs1: rs1, rs2: rs2, rd: rd },
                code::RType { funct3: 0b111, funct7: 0b0000000, rd, rs1, rs2 } => Instruction::AND { rs1: rs1, rs2: rs2, rd: rd },
                _ => Instruction::UNKNOWN { instruction: x },
            },
            0b0111011 => match RType::decode(x) {
                code::RType { funct3: 0b000, funct7: 0b0000000, rd, rs1, rs2 } => Instruction::ADDW { rs1: rs1, rs2: rs2, rd: rd },
                code::RType { funct3: 0b000, funct7: 0b0100000, rd, rs1, rs2 } => Instruction::SUBW { rs1: rs1, rs2: rs2, rd: rd },
                code::RType { funct3: 0b001, funct7: 0b0000000, rd, rs1, rs2 } => Instruction::SLLW { rs1: rs1, rs2: rs2, rd: rd },
                code::RType { funct3: 0b101, funct7: 0b0000000, rd, rs1, rs2 } => Instruction::SRLW { rs1: rs1, rs2: rs2, rd: rd },
                code::RType { funct3: 0b101, funct7: 0b0100000, rd, rs1, rs2 } => Instruction::SRAW { rs1: rs1, rs2: rs2, rd: rd },
                _ => Instruction::UNKNOWN { instruction: x },
            },
            0b0100011 => match SType::decode(x) {
                code::SType { funct3: 0b000, rs1, rs2, imm } => Instruction::SB { rs1: rs1, rs2: rs2, offset: imm },
                code::SType { funct3: 0b001, rs1, rs2, imm } => Instruction::SH { rs1: rs1, rs2: rs2, offset: imm },
                code::SType { funct3: 0b010, rs1, rs2, imm } => Instruction::SW { rs1: rs1, rs2: rs2, offset: imm },
                code::SType { funct3: 0b011, rs1, rs2, imm } => Instruction::SD { rs1: rs1, rs2: rs2, offset: imm },
                _ => Instruction::UNKNOWN { instruction: x },
            },
            0b1100011 => match SBType::decode(x) {
                code::SBType { funct3: 0b000, rs1, rs2, imm } => Instruction::BEQ { rs1: rs1, rs2: rs2, offset: imm },
                code::SBType { funct3: 0b001, rs1, rs2, imm } => Instruction::BNE { rs1: rs1, rs2: rs2, offset: imm },
                code::SBType { funct3: 0b100, rs1, rs2, imm } => Instruction::BLT { rs1: rs1, rs2: rs2, offset: imm },
                code::SBType { funct3: 0b101, rs1, rs2, imm } => Instruction::BGE { rs1: rs1, rs2: rs2, offset: imm },
                code::SBType { funct3: 0b110, rs1, rs2, imm } => Instruction::BLTU { rs1: rs1, rs2: rs2, offset: imm },
                code::SBType { funct3: 0b111, rs1, rs2, imm } => Instruction::BGEU { rs1: rs1, rs2: rs2, offset: imm },
                _ => Instruction::UNKNOWN { instruction: x },
            },
            0b0010111 => match UType::decode(x) {
                code::UType { rd, imm } => Instruction::AUIPC { rd: rd, offset: imm },
            }
            0b0110111 => match UType::decode(x) {
                code::UType { rd, imm } => Instruction::LUI { rd: rd, imm: imm },
            }
            0b1101111 => match UJType::decode(x) {
                code::UJType {rd, offset} => Instruction::JAL { rd: rd, offset: offset },
            }
            0b0001111 => match FenceType::decode(x) {
                code::FenceType {funct3: 0b000, pred, succ } => Instruction::FENCE { pred: pred, succ: succ},
                code::FenceType {funct3: 0b001, pred: 0, succ: 0 } => Instruction::FENCEI {  },
                _ => Instruction::UNKNOWN { instruction: x },
            },
            0b1110011 => match IType::decode(x) {
                code::IType { funct3: 0b000, rd: 0, rs: 0, imm: 0 } => Instruction::ECALL {},
                code::IType { funct3: 0b000, rd: 0, rs: 0, imm: 1 } => Instruction::EBREAK {},
                code::IType { funct3: 0b001, rd, rs, imm } => Instruction::CSRRW { rd: rd, rs: rs, csr: imm },
                code::IType { funct3: 0b010, rd, rs, imm } => Instruction::CSRRS { rd: rd, rs: rs, csr: imm },
                code::IType { funct3: 0b011, rd, rs, imm } => Instruction::CSRRC { rd: rd, rs: rs, csr: imm },
                code::IType { funct3: 0b101, rd, rs, imm } => Instruction::CSRRWI { rd: rd, zimm: rs as u16, csr: imm },
                code::IType { funct3: 0b110, rd, rs, imm } => Instruction::CSRRSI { rd: rd, zimm: rs as u16, csr: imm },
                code::IType { funct3: 0b111, rd, rs, imm } => Instruction::CSRRCI { rd: rd, zimm: rs as u16, csr: imm },
                _ => Instruction::UNKNOWN { instruction: x },
            },
            _ => Instruction::UNKNOWN { instruction: x },
        }
    }

    pub fn encode(&self) -> u32 {
        match self {
            Instruction::LB { rd, offset, rs } => IType::encode(0b0000011, 0b000, *rd, *rs, *offset),
            Instruction::LH { rd, offset, rs } => IType::encode(0b0000011, 0b001, *rd, *rs, *offset),
            Instruction::LW { rd, offset, rs } => IType::encode(0b0000011, 0b010, *rd, *rs, *offset),
            Instruction::LD { rd, offset, rs } => IType::encode(0b0000011, 0b011, *rd, *rs, *offset),
            Instruction::LBU { rd, offset, rs } => IType::encode(0b0000011, 0b100, *rd, *rs, *offset),
            Instruction::LHU { rd, offset, rs } => IType::encode(0b0000011, 0b101, *rd, *rs, *offset),
            Instruction::LWU { rd, offset, rs } => IType::encode(0b0000011, 0b110, *rd, *rs, *offset),
            Instruction::ADDI { rd, rs, imm } => IType::encode(0b0010011, 0b000, *rd, *rs, *imm),
            Instruction::SLLI { rd, rs, imm } => IshType::encode(0b0010011, 0b001, *rd, *rs, *imm, 0),
            Instruction::SLTI { rd, rs, imm } => IType::encode(0b0010011, 0b010, *rd, *rs, *imm),
            Instruction::SLTIU { rd, rs, imm } => IType::encode(0b0010011, 0b011, *rd, *rs, *imm),
            Instruction::XORI { rd, rs, imm } => IType::encode(0b0010011, 0b100, *rd, *rs, *imm),
            Instruction::SRLI { rd, rs, imm } => IshType::encode(0b0010011, 0b101, *rd, *rs, *imm, 0),
            Instruction::SRAI { rd, rs, imm } => IshType::encode(0b0010011, 0b101, *rd, *rs, *imm, 0b0100000),
            Instruction::ORI { rd, rs, imm } => IType::encode(0b0010011, 0b110, *rd, *rs, *imm),
            Instruction::ANDI { rd, rs, imm } => IType::encode(0b0010011, 0b111, *rd, *rs, *imm),
            Instruction::ADDIW { rd, rs, imm } => IType::encode(0b0011011, 0b000, *rd, *rs, *imm),
            Instruction::SLLIW { rd, rs, imm } => IshType::encode(0b0011011, 0b001, *rd, *rs, *imm, 0),
            Instruction::SRLIW { rd, rs, imm } => IshType::encode(0b0011011, 0b101, *rd, *rs, *imm, 0),
            Instruction::SRAIW { rd, rs, imm } => IshType::encode(0b0011011, 0b101, *rd, *rs, *imm, 0b0100000),
            Instruction::JALR { rd, rs, offset } => IType::encode(0b1100111, 0b000, *rd, *rs, *offset),
            Instruction::ADD { rd, rs1, rs2 } => RType::encode(0b0110011, 0b000, 0b0000000, *rd, *rs1, *rs2),
            Instruction::SUB { rd, rs1, rs2 } => RType::encode(0b0110011, 0b000, 0b0100000, *rd, *rs1, *rs2),
            Instruction::SLL { rd, rs1, rs2 } => RType::encode(0b0110011, 0b001, 0b0000000, *rd, *rs1, *rs2),
            Instruction::SLT { rd, rs1, rs2 } => RType::encode(0b0110011, 0b010, 0b0000000, *rd, *rs1, *rs2),
            Instruction::SLTU { rd, rs1, rs2 } => RType::encode(0b0110011, 0b011, 0b0000000, *rd, *rs1, *rs2),
            Instruction::XOR { rd, rs1, rs2 } => RType::encode(0b0110011, 0b100, 0b0000000, *rd, *rs1, *rs2),
            Instruction::SRL { rd, rs1, rs2 } => RType::encode(0b0110011, 0b101, 0b0000000, *rd, *rs1, *rs2),
            Instruction::SRA { rd, rs1, rs2 } => RType::encode(0b0110011, 0b101, 0b0100000, *rd, *rs1, *rs2),
            Instruction::OR { rd, rs1, rs2 } => RType::encode(0b0110011, 0b110, 0b0000000, *rd, *rs1, *rs2),
            Instruction::AND { rd, rs1, rs2 } => RType::encode(0b0110011, 0b111, 0b0000000, *rd, *rs1, *rs2),
            Instruction::ADDW { rd, rs1, rs2 } => RType::encode(0b0111011, 0b000, 0b0000000, *rd, *rs1, *rs2),
            Instruction::SUBW { rd, rs1, rs2 } => RType::encode(0b0111011, 0b000, 0b0100000, *rd, *rs1, *rs2),
            Instruction::SLLW { rd, rs1, rs2 } => RType::encode(0b0111011, 0b001, 0b0000000, *rd, *rs1, *rs2),
            Instruction::SRLW { rd, rs1, rs2 } => RType::encode(0b0111011, 0b101, 0b0000000, *rd, *rs1, *rs2),
            Instruction::SRAW { rd, rs1, rs2 } => RType::encode(0b0111011, 0b101, 0b0100000, *rd, *rs1, *rs2),
            Instruction::SB { rs2, offset, rs1 } => SType::encode(0b0100011, 0b000, *rs1, *rs2, *offset),
            Instruction::SH { offset, rs1, rs2 } => SType::encode(0b0100011, 0b001, *rs1, *rs2, *offset),
            Instruction::SW { offset, rs1, rs2 } => SType::encode(0b0100011, 0b010, *rs1, *rs2, *offset),
            Instruction::SD { offset, rs1, rs2 } => SType::encode(0b0100011, 0b011, *rs1, *rs2, *offset),
            Instruction::BEQ { rs1, rs2, offset } => SBType::encode(0b1100011, 0b000, *rs1, *rs2, *offset),
            Instruction::BNE { rs1, rs2, offset } => SBType::encode(0b1100011, 0b001, *rs1, *rs2, *offset),
            Instruction::BLT { rs1, rs2, offset } => SBType::encode(0b1100011, 0b100, *rs1, *rs2, *offset),
            Instruction::BGE { rs1, rs2, offset } => SBType::encode(0b1100011, 0b101, *rs1, *rs2, *offset),
            Instruction::BLTU { rs1, rs2, offset } => SBType::encode(0b1100011, 0b110, *rs1, *rs2, *offset),
            Instruction::BGEU { rs1, rs2, offset } => SBType::encode(0b1100011, 0b111, *rs1, *rs2, *offset),
            Instruction::AUIPC { rd, offset } => UType::encode(0b0010111, *rd, *offset),
            Instruction::LUI { rd, imm } => UType::encode(0b0110111, *rd, *imm),
            Instruction::JAL { rd, offset } => UJType::encode(0b1101111, *rd, *offset),
            Instruction::FENCE { pred, succ } => FenceType::encode(0b0001111, 0b000, *pred, *succ),
            Instruction::FENCEI {} => IType::encode(0b0001111, 0b001,0,0,0),
            Instruction::ECALL {} => IType::encode(0b1110011, 0b000,0,0,0),
            Instruction::EBREAK {} => IType::encode(0b1110011, 0b000,0,0,1),
            Instruction::CSRRW { rs, rd, csr } => IType::encode(0b1110011, 0b001, *rd, *rs, *csr),
            Instruction::CSRRS { rs, rd, csr } => IType::encode(0b1110011, 0b010, *rd, *rs, *csr),
            Instruction::CSRRC { rs, rd, csr } => IType::encode(0b1110011, 0b011, *rd, *rs, *csr),
            Instruction::CSRRWI { zimm, rd, csr } => IType::encode(0b1110011, 0b101, *rd, *zimm as usize, *csr),
            Instruction::CSRRSI { zimm, rd, csr } => IType::encode(0b1110011, 0b110, *rd, *zimm as usize, *csr),
            Instruction::CSRRCI { zimm, rd, csr } => IType::encode(0b1110011, 0b111, *rd, *zimm as usize, *csr),
            Instruction::UNKNOWN { instruction } => *instruction,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
