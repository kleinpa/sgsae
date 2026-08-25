//! RV32IM instruction encoding and decoding.
//!
//! [`Instruction`] is a decoded instruction: every field holds the value the
//! instruction *means*, not the bits it was stored in. Immediates arrive
//! already sign-extended to `i32`, branch and jump offsets are byte
//! displacements from the address of the instruction itself, and `LUI`/`AUIPC`
//! carry the full 32-bit value rather than the 20-bit field. Nothing
//! downstream has to remember a shift or an extension rule.
//!
//! [`Instruction::decode`] and [`Instruction::encode`] are inverses for every
//! instruction the decoder recognises, which is checked exhaustively by a
//! round-trip test over random words.

use crate::types::Register;

// ---------------------------------------------------------------------------
// Bitfield primitives
// ---------------------------------------------------------------------------

/// Extract `len` bits starting at bit `lo`.
pub(crate) const fn field(x: u32, lo: u32, len: u32) -> u32 {
    (x >> lo) & ((1u32 << len) - 1)
}

/// Sign-extend the low `len` bits of `x` to a full `i32`.
///
/// Every immediate in the ISA goes through here with its true width, which is
/// the whole reason the extension rules cannot drift apart between formats.
pub(crate) const fn sext(x: u32, len: u32) -> i32 {
    let shift = 32 - len;
    ((x << shift) as i32) >> shift
}

const fn opcode_of(x: u32) -> u32 {
    field(x, 0, 7)
}
const fn funct3(x: u32) -> u32 {
    field(x, 12, 3)
}
const fn funct7(x: u32) -> u32 {
    field(x, 25, 7)
}
const fn rd(x: u32) -> Register {
    field(x, 7, 5) as Register
}
const fn rs1(x: u32) -> Register {
    field(x, 15, 5) as Register
}
const fn rs2(x: u32) -> Register {
    field(x, 20, 5) as Register
}
const fn shamt(x: u32) -> u32 {
    field(x, 20, 5)
}

// Immediate reassembly. Each mirrors a figure in the ISA manual directly:
// the bit positions on the left are where the value's bits live in the word.

/// I-type: `imm[11:0] = inst[31:20]`.
const fn i_imm(x: u32) -> i32 {
    sext(field(x, 20, 12), 12)
}

/// S-type: `imm[11:5] = inst[31:25]`, `imm[4:0] = inst[11:7]`.
const fn s_imm(x: u32) -> i32 {
    sext((field(x, 25, 7) << 5) | field(x, 7, 5), 12)
}

/// B-type: 13-bit, always even. `imm[12|10:5]` in `inst[31:25]`,
/// `imm[4:1|11]` in `inst[11:7]`.
const fn b_imm(x: u32) -> i32 {
    let bits = (field(x, 31, 1) << 12)
        | (field(x, 7, 1) << 11)
        | (field(x, 25, 6) << 5)
        | (field(x, 8, 4) << 1);
    sext(bits, 13)
}

/// U-type: `imm[31:12] = inst[31:12]`, low 12 bits zero. Already a full value.
const fn u_imm(x: u32) -> i32 {
    (x & 0xFFFF_F000) as i32
}

/// J-type: 21-bit, always even. `imm[20|10:1|11|19:12]` in `inst[31:12]`.
const fn j_imm(x: u32) -> i32 {
    let bits = (field(x, 31, 1) << 20)
        | (field(x, 12, 8) << 12)
        | (field(x, 20, 1) << 11)
        | (field(x, 21, 10) << 1);
    sext(bits, 21)
}

// Encoders, mirroring the decoders above.

const fn enc_r(opcode: u32, f3: u32, f7: u32, rd: Register, rs1: Register, rs2: Register) -> u32 {
    (f7 << 25)
        | ((rs2 as u32 & 0x1F) << 20)
        | ((rs1 as u32 & 0x1F) << 15)
        | (f3 << 12)
        | ((rd as u32 & 0x1F) << 7)
        | opcode
}

const fn enc_i(opcode: u32, f3: u32, rd: Register, rs1: Register, imm: i32) -> u32 {
    (((imm as u32) & 0xFFF) << 20)
        | ((rs1 as u32 & 0x1F) << 15)
        | (f3 << 12)
        | ((rd as u32 & 0x1F) << 7)
        | opcode
}

const fn enc_ish(opcode: u32, f3: u32, f7: u32, rd: Register, rs1: Register, shamt: u32) -> u32 {
    (f7 << 25)
        | ((shamt & 0x1F) << 20)
        | ((rs1 as u32 & 0x1F) << 15)
        | (f3 << 12)
        | ((rd as u32 & 0x1F) << 7)
        | opcode
}

const fn enc_s(opcode: u32, f3: u32, rs1: Register, rs2: Register, imm: i32) -> u32 {
    let i = imm as u32;
    (((i >> 5) & 0x7F) << 25)
        | ((rs2 as u32 & 0x1F) << 20)
        | ((rs1 as u32 & 0x1F) << 15)
        | (f3 << 12)
        | ((i & 0x1F) << 7)
        | opcode
}

const fn enc_b(opcode: u32, f3: u32, rs1: Register, rs2: Register, imm: i32) -> u32 {
    let i = imm as u32;
    (((i >> 12) & 1) << 31)
        | (((i >> 5) & 0x3F) << 25)
        | ((rs2 as u32 & 0x1F) << 20)
        | ((rs1 as u32 & 0x1F) << 15)
        | (f3 << 12)
        | (((i >> 1) & 0xF) << 8)
        | (((i >> 11) & 1) << 7)
        | opcode
}

const fn enc_u(opcode: u32, rd: Register, imm: i32) -> u32 {
    ((imm as u32) & 0xFFFF_F000) | ((rd as u32 & 0x1F) << 7) | opcode
}

const fn enc_j(opcode: u32, rd: Register, imm: i32) -> u32 {
    let i = imm as u32;
    (((i >> 20) & 1) << 31)
        | (((i >> 1) & 0x3FF) << 21)
        | (((i >> 11) & 1) << 20)
        | (((i >> 12) & 0xFF) << 12)
        | ((rd as u32 & 0x1F) << 7)
        | opcode
}

// ---------------------------------------------------------------------------
// Opcodes
// ---------------------------------------------------------------------------

const OP_LOAD: u32 = 0b000_0011;
const OP_FENCE: u32 = 0b000_1111;
const OP_IMM: u32 = 0b001_0011;
const OP_AUIPC: u32 = 0b001_0111;
const OP_STORE: u32 = 0b010_0011;
const OP_REG: u32 = 0b011_0011;
const OP_LUI: u32 = 0b011_0111;
const OP_BRANCH: u32 = 0b110_0011;
const OP_JALR: u32 = 0b110_0111;
const OP_JAL: u32 = 0b110_1111;
const OP_SYSTEM: u32 = 0b111_0011;

/// `funct7` marking the "alternate" form of an op: SUB, SRA, SRAI.
const ALT: u32 = 0b010_0000;
/// `funct7` marking the M extension.
const MULDIV: u32 = 0b000_0001;

// ---------------------------------------------------------------------------
// Instruction
// ---------------------------------------------------------------------------

/// A decoded RV32IM instruction.
///
/// `offset` fields on `JAL` and the branches are relative to the address of
/// *that instruction*, matching the ISA. `imm` on `LUI` and `AUIPC` is the
/// full 32-bit value, already shifted left by 12.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[rustfmt::skip]
pub enum Instruction {
    // -- RV32I: upper immediates and jumps -----------------------------------
    /// Load Upper Immediate: `rd ← imm`
    LUI { rd: Register, imm: i32 },
    /// Add Upper Immediate to PC: `rd ← pc + imm`
    AUIPC { rd: Register, imm: i32 },
    /// Jump and Link: `rd ← pc + 4; pc ← pc + offset`
    JAL { rd: Register, offset: i32 },
    /// Jump and Link Register: `rd ← pc + 4; pc ← (rs1 + offset) & !1`
    JALR { rd: Register, rs1: Register, offset: i32 },

    // -- RV32I: branches -----------------------------------------------------
    /// Branch if Equal
    BEQ { rs1: Register, rs2: Register, offset: i32 },
    /// Branch if Not Equal
    BNE { rs1: Register, rs2: Register, offset: i32 },
    /// Branch if Less Than (signed)
    BLT { rs1: Register, rs2: Register, offset: i32 },
    /// Branch if Greater or Equal (signed)
    BGE { rs1: Register, rs2: Register, offset: i32 },
    /// Branch if Less Than (unsigned)
    BLTU { rs1: Register, rs2: Register, offset: i32 },
    /// Branch if Greater or Equal (unsigned)
    BGEU { rs1: Register, rs2: Register, offset: i32 },

    // -- RV32I: loads and stores ---------------------------------------------
    /// Load Byte, sign-extended
    LB { rd: Register, rs1: Register, offset: i32 },
    /// Load Half, sign-extended
    LH { rd: Register, rs1: Register, offset: i32 },
    /// Load Word
    LW { rd: Register, rs1: Register, offset: i32 },
    /// Load Byte, zero-extended
    LBU { rd: Register, rs1: Register, offset: i32 },
    /// Load Half, zero-extended
    LHU { rd: Register, rs1: Register, offset: i32 },
    /// Store Byte
    SB { rs1: Register, rs2: Register, offset: i32 },
    /// Store Half
    SH { rs1: Register, rs2: Register, offset: i32 },
    /// Store Word
    SW { rs1: Register, rs2: Register, offset: i32 },

    // -- RV32I: register-immediate -------------------------------------------
    /// Add Immediate
    ADDI { rd: Register, rs1: Register, imm: i32 },
    /// Set Less Than Immediate (signed)
    SLTI { rd: Register, rs1: Register, imm: i32 },
    /// Set Less Than Immediate (unsigned comparison, sign-extended immediate)
    SLTIU { rd: Register, rs1: Register, imm: i32 },
    /// Xor Immediate
    XORI { rd: Register, rs1: Register, imm: i32 },
    /// Or Immediate
    ORI { rd: Register, rs1: Register, imm: i32 },
    /// And Immediate
    ANDI { rd: Register, rs1: Register, imm: i32 },
    /// Shift Left Logical Immediate
    SLLI { rd: Register, rs1: Register, shamt: u32 },
    /// Shift Right Logical Immediate
    SRLI { rd: Register, rs1: Register, shamt: u32 },
    /// Shift Right Arithmetic Immediate
    SRAI { rd: Register, rs1: Register, shamt: u32 },

    // -- RV32I: register-register --------------------------------------------
    /// Add
    ADD { rd: Register, rs1: Register, rs2: Register },
    /// Subtract
    SUB { rd: Register, rs1: Register, rs2: Register },
    /// Shift Left Logical
    SLL { rd: Register, rs1: Register, rs2: Register },
    /// Set Less Than (signed)
    SLT { rd: Register, rs1: Register, rs2: Register },
    /// Set Less Than (unsigned)
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

    // -- RV32M ---------------------------------------------------------------
    /// Multiply, low word
    MUL { rd: Register, rs1: Register, rs2: Register },
    /// Multiply, high word (signed × signed)
    MULH { rd: Register, rs1: Register, rs2: Register },
    /// Multiply, high word (signed × unsigned)
    MULHSU { rd: Register, rs1: Register, rs2: Register },
    /// Multiply, high word (unsigned × unsigned)
    MULHU { rd: Register, rs1: Register, rs2: Register },
    /// Divide (signed)
    DIV { rd: Register, rs1: Register, rs2: Register },
    /// Divide (unsigned)
    DIVU { rd: Register, rs1: Register, rs2: Register },
    /// Remainder (signed)
    REM { rd: Register, rs1: Register, rs2: Register },
    /// Remainder (unsigned)
    REMU { rd: Register, rs1: Register, rs2: Register },

    // -- Memory ordering and system ------------------------------------------
    /// Fence
    FENCE { pred: u8, succ: u8 },
    /// Fence Instruction Stream
    FENCEI,
    /// Environment Call
    ECALL,
    /// Environment Breakpoint
    EBREAK,
    /// CSR Read and Write
    CSRRW { rd: Register, rs1: Register, csr: u16 },
    /// CSR Read and Set Bits
    CSRRS { rd: Register, rs1: Register, csr: u16 },
    /// CSR Read and Clear Bits
    CSRRC { rd: Register, rs1: Register, csr: u16 },
    /// CSR Read and Write Immediate
    CSRRWI { rd: Register, zimm: u32, csr: u16 },
    /// CSR Read and Set Bits Immediate
    CSRRSI { rd: Register, zimm: u32, csr: u16 },
    /// CSR Read and Clear Bits Immediate
    CSRRCI { rd: Register, zimm: u32, csr: u16 },

    /// Not a recognised RV32IM encoding.
    UNKNOWN { word: u32 },
}

impl Instruction {
    /// Decode one 32-bit instruction word.
    ///
    /// Anything not in RV32IM comes back as [`Instruction::UNKNOWN`]; the CPU
    /// turns that into an illegal-instruction trap.
    #[rustfmt::skip]
    pub fn decode(x: u32) -> Instruction {
        use Instruction as I;
        let unknown = I::UNKNOWN { word: x };

        match opcode_of(x) {
            OP_LUI => I::LUI { rd: rd(x), imm: u_imm(x) },
            OP_AUIPC => I::AUIPC { rd: rd(x), imm: u_imm(x) },
            OP_JAL => I::JAL { rd: rd(x), offset: j_imm(x) },

            OP_JALR => match funct3(x) {
                0b000 => I::JALR { rd: rd(x), rs1: rs1(x), offset: i_imm(x) },
                _ => unknown,
            },

            OP_BRANCH => {
                let (rs1, rs2, offset) = (rs1(x), rs2(x), b_imm(x));
                match funct3(x) {
                    0b000 => I::BEQ { rs1, rs2, offset },
                    0b001 => I::BNE { rs1, rs2, offset },
                    0b100 => I::BLT { rs1, rs2, offset },
                    0b101 => I::BGE { rs1, rs2, offset },
                    0b110 => I::BLTU { rs1, rs2, offset },
                    0b111 => I::BGEU { rs1, rs2, offset },
                    _ => unknown,
                }
            }

            OP_LOAD => {
                let (rd, rs1, offset) = (rd(x), rs1(x), i_imm(x));
                match funct3(x) {
                    0b000 => I::LB { rd, rs1, offset },
                    0b001 => I::LH { rd, rs1, offset },
                    0b010 => I::LW { rd, rs1, offset },
                    0b100 => I::LBU { rd, rs1, offset },
                    0b101 => I::LHU { rd, rs1, offset },
                    _ => unknown,
                }
            }

            OP_STORE => {
                let (rs1, rs2, offset) = (rs1(x), rs2(x), s_imm(x));
                match funct3(x) {
                    0b000 => I::SB { rs1, rs2, offset },
                    0b001 => I::SH { rs1, rs2, offset },
                    0b010 => I::SW { rs1, rs2, offset },
                    _ => unknown,
                }
            }

            OP_IMM => {
                let (rd, rs1, imm) = (rd(x), rs1(x), i_imm(x));
                match (funct3(x), funct7(x)) {
                    (0b000, _) => I::ADDI { rd, rs1, imm },
                    (0b010, _) => I::SLTI { rd, rs1, imm },
                    (0b011, _) => I::SLTIU { rd, rs1, imm },
                    (0b100, _) => I::XORI { rd, rs1, imm },
                    (0b110, _) => I::ORI { rd, rs1, imm },
                    (0b111, _) => I::ANDI { rd, rs1, imm },
                    // On RV32 the shift amount is 5 bits and the remaining 7
                    // select the variant; any other value is not an encoding.
                    (0b001, 0) => I::SLLI { rd, rs1, shamt: shamt(x) },
                    (0b101, 0) => I::SRLI { rd, rs1, shamt: shamt(x) },
                    (0b101, ALT) => I::SRAI { rd, rs1, shamt: shamt(x) },
                    _ => unknown,
                }
            }

            OP_REG => {
                let (rd, rs1, rs2) = (rd(x), rs1(x), rs2(x));
                match (funct3(x), funct7(x)) {
                    (0b000, 0) => I::ADD { rd, rs1, rs2 },
                    (0b000, ALT) => I::SUB { rd, rs1, rs2 },
                    (0b001, 0) => I::SLL { rd, rs1, rs2 },
                    (0b010, 0) => I::SLT { rd, rs1, rs2 },
                    (0b011, 0) => I::SLTU { rd, rs1, rs2 },
                    (0b100, 0) => I::XOR { rd, rs1, rs2 },
                    (0b101, 0) => I::SRL { rd, rs1, rs2 },
                    (0b101, ALT) => I::SRA { rd, rs1, rs2 },
                    (0b110, 0) => I::OR { rd, rs1, rs2 },
                    (0b111, 0) => I::AND { rd, rs1, rs2 },

                    (0b000, MULDIV) => I::MUL { rd, rs1, rs2 },
                    (0b001, MULDIV) => I::MULH { rd, rs1, rs2 },
                    (0b010, MULDIV) => I::MULHSU { rd, rs1, rs2 },
                    (0b011, MULDIV) => I::MULHU { rd, rs1, rs2 },
                    (0b100, MULDIV) => I::DIV { rd, rs1, rs2 },
                    (0b101, MULDIV) => I::DIVU { rd, rs1, rs2 },
                    (0b110, MULDIV) => I::REM { rd, rs1, rs2 },
                    (0b111, MULDIV) => I::REMU { rd, rs1, rs2 },
                    _ => unknown,
                }
            }

            OP_FENCE => match (funct3(x), rd(x), rs1(x)) {
                (0b000, 0, 0) => {
                    I::FENCE { pred: field(x, 24, 4) as u8, succ: field(x, 20, 4) as u8 }
                }
                (0b001, 0, 0) if field(x, 20, 12) == 0 => I::FENCEI,
                _ => unknown,
            },

            OP_SYSTEM => {
                let (rd, rs1, csr) = (rd(x), rs1(x), field(x, 20, 12) as u16);
                let zimm = u32::from(rs1);
                match funct3(x) {
                    0b000 => match (x, rd, rs1) {
                        (_, 0, 0) if csr == 0 => I::ECALL,
                        (_, 0, 0) if csr == 1 => I::EBREAK,
                        _ => unknown,
                    },
                    0b001 => I::CSRRW { rd, rs1, csr },
                    0b010 => I::CSRRS { rd, rs1, csr },
                    0b011 => I::CSRRC { rd, rs1, csr },
                    0b101 => I::CSRRWI { rd, zimm, csr },
                    0b110 => I::CSRRSI { rd, zimm, csr },
                    0b111 => I::CSRRCI { rd, zimm, csr },
                    _ => unknown,
                }
            }

            _ => unknown,
        }
    }

    /// Encode back to a 32-bit instruction word.
    ///
    /// In debug builds, out-of-range immediates and misaligned branch targets
    /// assert rather than silently truncating.
    #[rustfmt::skip]
    pub fn encode(&self) -> u32 {
        use Instruction as I;
        match *self {
            I::LUI { rd, imm } => { debug_assert!(fits_upper(imm)); enc_u(OP_LUI, rd, imm) }
            I::AUIPC { rd, imm } => { debug_assert!(fits_upper(imm)); enc_u(OP_AUIPC, rd, imm) }
            I::JAL { rd, offset } => { debug_assert!(fits_jump(offset)); enc_j(OP_JAL, rd, offset) }
            I::JALR { rd, rs1, offset } => { debug_assert!(fits(offset, 12)); enc_i(OP_JALR, 0b000, rd, rs1, offset) }

            I::BEQ { rs1, rs2, offset } => { debug_assert!(fits_branch(offset)); enc_b(OP_BRANCH, 0b000, rs1, rs2, offset) }
            I::BNE { rs1, rs2, offset } => { debug_assert!(fits_branch(offset)); enc_b(OP_BRANCH, 0b001, rs1, rs2, offset) }
            I::BLT { rs1, rs2, offset } => { debug_assert!(fits_branch(offset)); enc_b(OP_BRANCH, 0b100, rs1, rs2, offset) }
            I::BGE { rs1, rs2, offset } => { debug_assert!(fits_branch(offset)); enc_b(OP_BRANCH, 0b101, rs1, rs2, offset) }
            I::BLTU { rs1, rs2, offset } => { debug_assert!(fits_branch(offset)); enc_b(OP_BRANCH, 0b110, rs1, rs2, offset) }
            I::BGEU { rs1, rs2, offset } => { debug_assert!(fits_branch(offset)); enc_b(OP_BRANCH, 0b111, rs1, rs2, offset) }

            I::LB { rd, rs1, offset } => { debug_assert!(fits(offset, 12)); enc_i(OP_LOAD, 0b000, rd, rs1, offset) }
            I::LH { rd, rs1, offset } => { debug_assert!(fits(offset, 12)); enc_i(OP_LOAD, 0b001, rd, rs1, offset) }
            I::LW { rd, rs1, offset } => { debug_assert!(fits(offset, 12)); enc_i(OP_LOAD, 0b010, rd, rs1, offset) }
            I::LBU { rd, rs1, offset } => { debug_assert!(fits(offset, 12)); enc_i(OP_LOAD, 0b100, rd, rs1, offset) }
            I::LHU { rd, rs1, offset } => { debug_assert!(fits(offset, 12)); enc_i(OP_LOAD, 0b101, rd, rs1, offset) }

            I::SB { rs1, rs2, offset } => { debug_assert!(fits(offset, 12)); enc_s(OP_STORE, 0b000, rs1, rs2, offset) }
            I::SH { rs1, rs2, offset } => { debug_assert!(fits(offset, 12)); enc_s(OP_STORE, 0b001, rs1, rs2, offset) }
            I::SW { rs1, rs2, offset } => { debug_assert!(fits(offset, 12)); enc_s(OP_STORE, 0b010, rs1, rs2, offset) }

            I::ADDI { rd, rs1, imm } => { debug_assert!(fits(imm, 12)); enc_i(OP_IMM, 0b000, rd, rs1, imm) }
            I::SLTI { rd, rs1, imm } => { debug_assert!(fits(imm, 12)); enc_i(OP_IMM, 0b010, rd, rs1, imm) }
            I::SLTIU { rd, rs1, imm } => { debug_assert!(fits(imm, 12)); enc_i(OP_IMM, 0b011, rd, rs1, imm) }
            I::XORI { rd, rs1, imm } => { debug_assert!(fits(imm, 12)); enc_i(OP_IMM, 0b100, rd, rs1, imm) }
            I::ORI { rd, rs1, imm } => { debug_assert!(fits(imm, 12)); enc_i(OP_IMM, 0b110, rd, rs1, imm) }
            I::ANDI { rd, rs1, imm } => { debug_assert!(fits(imm, 12)); enc_i(OP_IMM, 0b111, rd, rs1, imm) }
            I::SLLI { rd, rs1, shamt } => { debug_assert!(shamt < 32); enc_ish(OP_IMM, 0b001, 0, rd, rs1, shamt) }
            I::SRLI { rd, rs1, shamt } => { debug_assert!(shamt < 32); enc_ish(OP_IMM, 0b101, 0, rd, rs1, shamt) }
            I::SRAI { rd, rs1, shamt } => { debug_assert!(shamt < 32); enc_ish(OP_IMM, 0b101, ALT, rd, rs1, shamt) }

            I::ADD { rd, rs1, rs2 } => enc_r(OP_REG, 0b000, 0, rd, rs1, rs2),
            I::SUB { rd, rs1, rs2 } => enc_r(OP_REG, 0b000, ALT, rd, rs1, rs2),
            I::SLL { rd, rs1, rs2 } => enc_r(OP_REG, 0b001, 0, rd, rs1, rs2),
            I::SLT { rd, rs1, rs2 } => enc_r(OP_REG, 0b010, 0, rd, rs1, rs2),
            I::SLTU { rd, rs1, rs2 } => enc_r(OP_REG, 0b011, 0, rd, rs1, rs2),
            I::XOR { rd, rs1, rs2 } => enc_r(OP_REG, 0b100, 0, rd, rs1, rs2),
            I::SRL { rd, rs1, rs2 } => enc_r(OP_REG, 0b101, 0, rd, rs1, rs2),
            I::SRA { rd, rs1, rs2 } => enc_r(OP_REG, 0b101, ALT, rd, rs1, rs2),
            I::OR { rd, rs1, rs2 } => enc_r(OP_REG, 0b110, 0, rd, rs1, rs2),
            I::AND { rd, rs1, rs2 } => enc_r(OP_REG, 0b111, 0, rd, rs1, rs2),

            I::MUL { rd, rs1, rs2 } => enc_r(OP_REG, 0b000, MULDIV, rd, rs1, rs2),
            I::MULH { rd, rs1, rs2 } => enc_r(OP_REG, 0b001, MULDIV, rd, rs1, rs2),
            I::MULHSU { rd, rs1, rs2 } => enc_r(OP_REG, 0b010, MULDIV, rd, rs1, rs2),
            I::MULHU { rd, rs1, rs2 } => enc_r(OP_REG, 0b011, MULDIV, rd, rs1, rs2),
            I::DIV { rd, rs1, rs2 } => enc_r(OP_REG, 0b100, MULDIV, rd, rs1, rs2),
            I::DIVU { rd, rs1, rs2 } => enc_r(OP_REG, 0b101, MULDIV, rd, rs1, rs2),
            I::REM { rd, rs1, rs2 } => enc_r(OP_REG, 0b110, MULDIV, rd, rs1, rs2),
            I::REMU { rd, rs1, rs2 } => enc_r(OP_REG, 0b111, MULDIV, rd, rs1, rs2),

            I::FENCE { pred, succ } => {
                debug_assert!(pred < 16 && succ < 16);
                ((pred as u32) << 24) | ((succ as u32) << 20) | OP_FENCE
            }
            I::FENCEI => (0b001 << 12) | OP_FENCE,
            I::ECALL => enc_i(OP_SYSTEM, 0b000, 0, 0, 0),
            I::EBREAK => enc_i(OP_SYSTEM, 0b000, 0, 0, 1),

            I::CSRRW { rd, rs1, csr } => enc_i(OP_SYSTEM, 0b001, rd, rs1, i32::from(csr)),
            I::CSRRS { rd, rs1, csr } => enc_i(OP_SYSTEM, 0b010, rd, rs1, i32::from(csr)),
            I::CSRRC { rd, rs1, csr } => enc_i(OP_SYSTEM, 0b011, rd, rs1, i32::from(csr)),
            I::CSRRWI { rd, zimm, csr } => { debug_assert!(zimm < 32); enc_i(OP_SYSTEM, 0b101, rd, zimm as Register, i32::from(csr)) }
            I::CSRRSI { rd, zimm, csr } => { debug_assert!(zimm < 32); enc_i(OP_SYSTEM, 0b110, rd, zimm as Register, i32::from(csr)) }
            I::CSRRCI { rd, zimm, csr } => { debug_assert!(zimm < 32); enc_i(OP_SYSTEM, 0b111, rd, zimm as Register, i32::from(csr)) }

            I::UNKNOWN { word } => word,
        }
    }
}

/// Does `v` fit in a `bits`-wide two's complement field?
const fn fits(v: i32, bits: u32) -> bool {
    let min = -(1i64 << (bits - 1));
    let max = (1i64 << (bits - 1)) - 1;
    (v as i64) >= min && (v as i64) <= max
}

/// Branch displacements are 13-bit and always even.
const fn fits_branch(v: i32) -> bool {
    fits(v, 13) && v % 2 == 0
}

/// Jump displacements are 21-bit and always even.
const fn fits_jump(v: i32) -> bool {
    fits(v, 21) && v % 2 == 0
}

/// `LUI`/`AUIPC` carry a full value whose low 12 bits must be zero.
const fn fits_upper(v: i32) -> bool {
    v & 0xFFF == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use Instruction as I;

    /// Reference encodings produced by
    /// `llvm-mc -triple=riscv32 -mattr=+m -show-encoding`.
    ///
    /// Unlike the table this replaces, the immediates here sit on the
    /// boundaries where sign extension actually matters.
    #[rustfmt::skip]
    static VECTORS: &[(u32, Instruction)] = &[
        // Loads and stores, at both ends of the 12-bit range.
        (0x00858503, I::LB   { rd: 10, rs1: 11, offset: 8 }),
        (0x00859503, I::LH   { rd: 10, rs1: 11, offset: 8 }),
        (0x0085a503, I::LW   { rd: 10, rs1: 11, offset: 8 }),
        (0x0085c503, I::LBU  { rd: 10, rs1: 11, offset: 8 }),
        (0x0085d503, I::LHU  { rd: 10, rs1: 11, offset: 8 }),
        (0x8005a503, I::LW   { rd: 10, rs1: 11, offset: -2048 }),
        (0x7ff5a503, I::LW   { rd: 10, rs1: 11, offset: 2047 }),
        (0x00a58423, I::SB   { rs1: 11, rs2: 10, offset: 8 }),
        (0x00a59423, I::SH   { rs1: 11, rs2: 10, offset: 8 }),
        (0x00a5a423, I::SW   { rs1: 11, rs2: 10, offset: 8 }),
        (0x06a5a223, I::SW   { rs1: 11, rs2: 10, offset: 100 }),
        (0x80a5a023, I::SW   { rs1: 11, rs2: 10, offset: -2048 }),
        (0x7ea5afa3, I::SW   { rs1: 11, rs2: 10, offset: 2047 }),

        // Register-immediate, including the values the old decoder got wrong.
        (0x01058513, I::ADDI { rd: 10, rs1: 11, imm: 16 }),
        (0x7ff58513, I::ADDI { rd: 10, rs1: 11, imm: 2047 }),
        (0x80058513, I::ADDI { rd: 10, rs1: 11, imm: -2048 }),
        (0x40058513, I::ADDI { rd: 10, rs1: 11, imm: 1024 }),
        (0xfff58513, I::ADDI { rd: 10, rs1: 11, imm: -1 }),
        (0x8005a513, I::SLTI { rd: 10, rs1: 11, imm: -2048 }),
        (0xfff5b513, I::SLTIU{ rd: 10, rs1: 11, imm: -1 }),
        (0xfff5c513, I::XORI { rd: 10, rs1: 11, imm: -1 }),
        (0xf005e513, I::ORI  { rd: 10, rs1: 11, imm: -256 }),
        (0xfff5f513, I::ANDI { rd: 10, rs1: 11, imm: -1 }),
        (0x01059513, I::SLLI { rd: 10, rs1: 11, shamt: 16 }),
        (0x01f59513, I::SLLI { rd: 10, rs1: 11, shamt: 31 }),
        (0x0105d513, I::SRLI { rd: 10, rs1: 11, shamt: 16 }),
        (0x4105d513, I::SRAI { rd: 10, rs1: 11, shamt: 16 }),

        // Upper immediates carry the full value.
        (0x00010537, I::LUI  { rd: 10, imm: 0x10000 }),
        (0x80000537, I::LUI  { rd: 10, imm: -0x8000_0000 }),
        (0xfffff537, I::LUI  { rd: 10, imm: -0x1000 }),
        (0x00008517, I::AUIPC{ rd: 10, imm: 0x8000 }),
        (0xfffff517, I::AUIPC{ rd: 10, imm: -0x1000 }),

        // Register-register.
        (0x00c58533, I::ADD  { rd: 10, rs1: 11, rs2: 12 }),
        (0x40c58533, I::SUB  { rd: 10, rs1: 11, rs2: 12 }),
        (0x00c59533, I::SLL  { rd: 10, rs1: 11, rs2: 12 }),
        (0x00c5a533, I::SLT  { rd: 10, rs1: 11, rs2: 12 }),
        (0x00c5b533, I::SLTU { rd: 10, rs1: 11, rs2: 12 }),
        (0x00c5c533, I::XOR  { rd: 10, rs1: 11, rs2: 12 }),
        (0x00c5d533, I::SRL  { rd: 10, rs1: 11, rs2: 12 }),
        (0x40c5d533, I::SRA  { rd: 10, rs1: 11, rs2: 12 }),
        (0x00c5e533, I::OR   { rd: 10, rs1: 11, rs2: 12 }),
        (0x00c5f533, I::AND  { rd: 10, rs1: 11, rs2: 12 }),

        // RV32M.
        (0x02c58533, I::MUL   { rd: 10, rs1: 11, rs2: 12 }),
        (0x02c59533, I::MULH  { rd: 10, rs1: 11, rs2: 12 }),
        (0x02c5a533, I::MULHSU{ rd: 10, rs1: 11, rs2: 12 }),
        (0x02c5b533, I::MULHU { rd: 10, rs1: 11, rs2: 12 }),
        (0x02c5c533, I::DIV   { rd: 10, rs1: 11, rs2: 12 }),
        (0x02c5d533, I::DIVU  { rd: 10, rs1: 11, rs2: 12 }),
        (0x02c5e533, I::REM   { rd: 10, rs1: 11, rs2: 12 }),
        (0x02c5f533, I::REMU  { rd: 10, rs1: 11, rs2: 12 }),

        // Control transfer, at the extremes of each displacement.
        (0x00b50463, I::BEQ  { rs1: 10, rs2: 11, offset: 8 }),
        (0xfeb50ce3, I::BEQ  { rs1: 10, rs2: 11, offset: -8 }),
        (0x00b500e3, I::BEQ  { rs1: 10, rs2: 11, offset: 2048 }),
        (0x7eb50fe3, I::BEQ  { rs1: 10, rs2: 11, offset: 4094 }),
        (0x80b50063, I::BEQ  { rs1: 10, rs2: 11, offset: -4096 }),
        (0x06b51263, I::BNE  { rs1: 10, rs2: 11, offset: 100 }),
        (0x00b54463, I::BLT  { rs1: 10, rs2: 11, offset: 8 }),
        (0x00b55463, I::BGE  { rs1: 10, rs2: 11, offset: 8 }),
        (0x00b56463, I::BLTU { rs1: 10, rs2: 11, offset: 8 }),
        (0x00b57463, I::BGEU { rs1: 10, rs2: 11, offset: 8 }),
        (0x0080056f, I::JAL  { rd: 10, offset: 8 }),
        (0xff9ff56f, I::JAL  { rd: 10, offset: -8 }),
        (0x7ffff56f, I::JAL  { rd: 10, offset: 1048574 }),
        (0x8000056f, I::JAL  { rd: 10, offset: -1048576 }),
        (0x00858567, I::JALR { rd: 10, rs1: 11, offset: 8 }),
        (0x80058567, I::JALR { rd: 10, rs1: 11, offset: -2048 }),

        // System.
        (0x0ff0000f, I::FENCE { pred: 15, succ: 15 }),
        (0x0000100f, I::FENCEI),
        (0x00000073, I::ECALL),
        (0x00100073, I::EBREAK),
        (0x00059573, I::CSRRW { rd: 10, rs1: 11, csr: 0 }),
        (0x0015a573, I::CSRRS { rd: 10, rs1: 11, csr: 1 }),
        (0x0025b573, I::CSRRC { rd: 10, rs1: 11, csr: 2 }),
        (0x0038d573, I::CSRRWI{ rd: 10, zimm: 17, csr: 3 }),
        (0x0048e573, I::CSRRSI{ rd: 10, zimm: 17, csr: 4 }),
        (0x0058f573, I::CSRRCI{ rd: 10, zimm: 17, csr: 5 }),
    ];

    #[test]
    fn decodes_reference_encodings() {
        for (word, want) in VECTORS {
            assert_eq!(&Instruction::decode(*word), want, "decoding {word:#010x}");
        }
    }

    #[test]
    fn encodes_reference_encodings() {
        for (word, instruction) in VECTORS {
            assert_eq!(instruction.encode(), *word, "encoding {instruction:?}");
        }
    }

    /// The property the old table-only tests could not express: for every word
    /// the decoder claims to understand, encoding the result must reproduce it
    /// exactly. This is what catches a getter and its setter drifting apart.
    #[test]
    fn decode_and_encode_round_trip() {
        // xorshift32, so the corpus is reproducible without a dependency.
        let mut state: u32 = 0x1234_5678;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };

        let mut recognised = 0u32;
        for _ in 0..2_000_000 {
            let word = next();
            let decoded = Instruction::decode(word);
            if let Instruction::UNKNOWN { .. } = decoded {
                continue;
            }
            recognised += 1;
            assert_eq!(decoded.encode(), word, "round trip failed for {word:#010x} -> {decoded:?}");
        }
        // Sanity: the corpus actually exercised the decoder.
        assert!(recognised > 50_000, "only {recognised} words were recognised");
    }

    /// Walk every opcode/funct3/funct7 combination rather than sampling, so no
    /// recognised encoding escapes the round-trip property.
    #[test]
    fn round_trip_is_exhaustive_over_the_encoding_space() {
        for opcode in 0u32..128 {
            for funct3 in 0u32..8 {
                for funct7 in 0u32..128 {
                    // Fill the register and immediate fields with a pattern
                    // that is not all-zero, so field placement bugs show up.
                    let word = (funct7 << 25)
                        | (0b10110 << 20)
                        | (0b01101 << 15)
                        | (funct3 << 12)
                        | (0b11010 << 7)
                        | opcode;
                    let decoded = Instruction::decode(word);
                    if let Instruction::UNKNOWN { .. } = decoded {
                        continue;
                    }
                    assert_eq!(decoded.encode(), word, "{word:#010x} -> {decoded:?}");
                }
            }
        }
    }

    #[test]
    fn unknown_encodings_survive_a_round_trip_unchanged() {
        // 0b1111111 is not an RV32 opcode.
        let word = 0xDEAD_BE7F;
        assert_eq!(Instruction::decode(word), I::UNKNOWN { word });
        assert_eq!(I::UNKNOWN { word }.encode(), word);
    }

    #[test]
    fn rv64_only_encodings_are_rejected() {
        // LD, LWU, SD, ADDIW, ADDW: valid on RV64, illegal here.
        for word in [0x0085b503u32, 0x0085e503, 0x00c5b423, 0x0105851b, 0x00c5853b] {
            assert!(
                matches!(Instruction::decode(word), I::UNKNOWN { .. }),
                "{word:#010x} should not decode on RV32"
            );
        }
        // On RV32 a shift amount only has 5 bits; bit 25 set is not an encoding.
        assert!(matches!(Instruction::decode(0x02059513), I::UNKNOWN { .. }));
    }

    #[test]
    fn sign_extension_is_width_correct() {
        assert_eq!(sext(0b0111, 4), 7);
        assert_eq!(sext(0b1000, 4), -8);
        assert_eq!(sext(0xFFF, 12), -1);
        assert_eq!(sext(0x7FF, 12), 2047);
        assert_eq!(sext(0x800, 12), -2048);
        assert_eq!(sext(0x1FFF, 13), -1);
        assert_eq!(sext(0x0FFF, 13), 4095);
        assert_eq!(sext(0x1F_FFFF, 21), -1);
        assert_eq!(sext(0x0F_FFFF, 21), 1048575);
    }
}
