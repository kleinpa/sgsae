//! The RV32C compressed instruction extension.
//!
//! Every 16-bit encoding is shorthand for one 32-bit instruction, so this
//! module is a pure expander: [`decode`] turns a halfword into the
//! [`Instruction`] it stands for, and the interpreter never knows the
//! difference. Expansion is not reversible — re-encoding the result yields the
//! 32-bit form — which is fine, because nothing needs to emit compressed code.
//!
//! Supporting this is not optional in practice. Rust's bare-metal RISC-V
//! targets are `riscv32imc` and `riscv32imac`; both bake `c` into the target
//! spec, and `-Ctarget-feature=-c` does not remove it. A toolchain-built
//! cartridge is roughly two-thirds compressed instructions.
//!
//! The float encodings (`C.FLD`, `C.FSW`, and friends) decode as illegal, since
//! the machine has no floating point.

use crate::instructions::{field, sext, Instruction};
use crate::types::Register;

/// Does this halfword begin a 16-bit instruction?
///
/// Instructions whose low two bits are both set are 32 bits wide; every other
/// pattern introduces a compressed one.
pub const fn is_compressed(half: u16) -> bool {
    half & 0b11 != 0b11
}

/// The 3-bit register fields select x8..x15 — the registers a compiler reaches
/// for most often, which is what makes the short encodings pay off.
const fn creg(bits: u32) -> Register {
    (8 + (bits & 0b111)) as Register
}

/// Full 5-bit register field.
const fn reg(x: u32, lo: u32) -> Register {
    field(x, lo, 5) as Register
}

/// `C.LW` / `C.SW` offset: `off[5:3]` at 12:10, `off[2]` at 6, `off[6]` at 5.
const fn lw_offset(x: u32) -> i32 {
    ((field(x, 10, 3) << 3) | (field(x, 6, 1) << 2) | (field(x, 5, 1) << 6)) as i32
}

/// `C.J` / `C.JAL` target: an 12-bit signed, even displacement.
const fn j_offset(x: u32) -> i32 {
    let v = (field(x, 12, 1) << 11)
        | (field(x, 11, 1) << 4)
        | (field(x, 9, 2) << 8)
        | (field(x, 8, 1) << 10)
        | (field(x, 7, 1) << 6)
        | (field(x, 6, 1) << 7)
        | (field(x, 3, 3) << 1)
        | (field(x, 2, 1) << 5);
    sext(v, 12)
}

/// `C.BEQZ` / `C.BNEZ` target: a 9-bit signed, even displacement.
const fn b_offset(x: u32) -> i32 {
    let v = (field(x, 12, 1) << 8)
        | (field(x, 10, 2) << 3)
        | (field(x, 5, 2) << 6)
        | (field(x, 3, 2) << 1)
        | (field(x, 2, 1) << 5);
    sext(v, 9)
}

/// The 6-bit immediate shared by `C.ADDI`, `C.LI`, and `C.ANDI`.
const fn imm6(x: u32) -> i32 {
    sext((field(x, 12, 1) << 5) | field(x, 2, 5), 6)
}

/// Shift amount for `C.SLLI` / `C.SRLI` / `C.SRAI`.
const fn shamt(x: u32) -> u32 {
    (field(x, 12, 1) << 5) | field(x, 2, 5)
}

/// Expand one compressed instruction.
///
/// Reserved patterns, RV64-only encodings, and the float instructions come back
/// as [`Instruction::UNKNOWN`] carrying the original halfword, which the CPU
/// reports as an illegal instruction.
pub fn decode(half: u16) -> Instruction {
    use Instruction as I;
    let unknown = I::UNKNOWN { word: u32::from(half) };
    let half = u32::from(half);
    let funct3 = field(half, 13, 3);

    match half & 0b11 {
        // -- Quadrant 0: stack-relative allocation and base+offset memory ----
        0b00 => match funct3 {
            // C.ADDI4SPN: rd' = sp + zero-extended scaled immediate.
            0b000 => {
                let imm = (field(half, 11, 2) << 4)
                    | (field(half, 7, 4) << 6)
                    | (field(half, 6, 1) << 2)
                    | (field(half, 5, 1) << 3);
                if imm == 0 {
                    // The all-zero encoding is the canonical illegal
                    // instruction, deliberately reserved.
                    return unknown;
                }
                I::ADDI { rd: creg(field(half, 2, 3)), rs1: 2, imm: imm as i32 }
            }
            0b010 => I::LW {
                rd: creg(field(half, 2, 3)),
                rs1: creg(field(half, 7, 3)),
                offset: lw_offset(half),
            },
            0b110 => I::SW {
                rs1: creg(field(half, 7, 3)),
                rs2: creg(field(half, 2, 3)),
                offset: lw_offset(half),
            },
            // 001/011/101/111 are the float loads and stores; 100 is reserved.
            _ => unknown,
        },

        // -- Quadrant 1: immediates, control flow, and register-register -----
        0b01 => match funct3 {
            // C.ADDI, and C.NOP when both fields are zero.
            0b000 => {
                let rd = reg(half, 7);
                I::ADDI { rd, rs1: rd, imm: imm6(half) }
            }
            // C.JAL exists only on RV32, where it links to ra.
            0b001 => I::JAL { rd: 1, offset: j_offset(half) },
            0b010 => I::ADDI { rd: reg(half, 7), rs1: 0, imm: imm6(half) },
            0b011 => {
                let rd = reg(half, 7);
                if rd == 2 {
                    // C.ADDI16SP: adjust the stack pointer by a scaled amount.
                    let imm = (field(half, 12, 1) << 9)
                        | (field(half, 6, 1) << 4)
                        | (field(half, 5, 1) << 6)
                        | (field(half, 3, 2) << 7)
                        | (field(half, 2, 1) << 5);
                    if imm == 0 {
                        return unknown;
                    }
                    I::ADDI { rd: 2, rs1: 2, imm: sext(imm, 10) }
                } else {
                    // C.LUI carries bits 17:12 of the value.
                    let imm = (field(half, 12, 1) << 17) | (field(half, 2, 5) << 12);
                    if imm == 0 || rd == 0 {
                        return unknown;
                    }
                    I::LUI { rd, imm: sext(imm, 18) }
                }
            }
            0b100 => {
                let rd = creg(field(half, 7, 3));
                match field(half, 10, 2) {
                    // On RV32 the sixth shift-amount bit must be zero.
                    0b00 if shamt(half) < 32 => I::SRLI { rd, rs1: rd, shamt: shamt(half) },
                    0b01 if shamt(half) < 32 => I::SRAI { rd, rs1: rd, shamt: shamt(half) },
                    0b00 | 0b01 => unknown,
                    0b10 => I::ANDI { rd, rs1: rd, imm: imm6(half) },
                    _ => {
                        let rs2 = creg(field(half, 2, 3));
                        // Bit 12 selects the RV64-only word forms.
                        if field(half, 12, 1) != 0 {
                            return unknown;
                        }
                        match field(half, 5, 2) {
                            0b00 => I::SUB { rd, rs1: rd, rs2 },
                            0b01 => I::XOR { rd, rs1: rd, rs2 },
                            0b10 => I::OR { rd, rs1: rd, rs2 },
                            _ => I::AND { rd, rs1: rd, rs2 },
                        }
                    }
                }
            }
            0b101 => I::JAL { rd: 0, offset: j_offset(half) },
            0b110 => I::BEQ { rs1: creg(field(half, 7, 3)), rs2: 0, offset: b_offset(half) },
            _ => I::BNE { rs1: creg(field(half, 7, 3)), rs2: 0, offset: b_offset(half) },
        },

        // -- Quadrant 2: stack-pointer-relative memory and register moves ----
        0b10 => match funct3 {
            0b000 => {
                let rd = reg(half, 7);
                // A shift of 32 or more has no RV32 encoding.
                if shamt(half) >= 32 {
                    return unknown;
                }
                I::SLLI { rd, rs1: rd, shamt: shamt(half) }
            }
            // C.LWSP
            0b010 => {
                let rd = reg(half, 7);
                if rd == 0 {
                    return unknown;
                }
                let offset =
                    (field(half, 12, 1) << 5) | (field(half, 4, 3) << 2) | (field(half, 2, 2) << 6);
                I::LW { rd, rs1: 2, offset: offset as i32 }
            }
            0b100 => {
                let rd = reg(half, 7);
                let rs2 = reg(half, 2);
                match (field(half, 12, 1), rd, rs2) {
                    // C.JR
                    (0, rd, 0) if rd != 0 => I::JALR { rd: 0, rs1: rd, offset: 0 },
                    (0, 0, _) => unknown,
                    // C.MV
                    (0, rd, rs2) => I::ADD { rd, rs1: 0, rs2 },
                    (1, 0, 0) => I::EBREAK,
                    // C.JALR
                    (1, rd, 0) => I::JALR { rd: 1, rs1: rd, offset: 0 },
                    // C.ADD
                    (1, rd, rs2) => I::ADD { rd, rs1: rd, rs2 },
                    _ => unknown,
                }
            }
            // C.SWSP
            0b110 => {
                let offset = (field(half, 9, 4) << 2) | (field(half, 7, 2) << 6);
                I::SW { rs1: 2, rs2: reg(half, 2), offset: offset as i32 }
            }
            // 001/011/101/111 are the float forms.
            _ => unknown,
        },

        // Not compressed at all.
        _ => unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Instruction as I;

    /// Reference encodings from
    /// `llvm-mc -triple=riscv32 -mattr=+m,+c -show-encoding`.
    #[rustfmt::skip]
    static VECTORS: &[(u16, Instruction)] = &[
        // Quadrant 0.
        (0x0044, I::ADDI { rd: 9,  rs1: 2, imm: 4 }),      // c.addi4spn s1, sp, 4
        (0x1fe0, I::ADDI { rd: 8,  rs1: 2, imm: 1020 }),   // c.addi4spn s0, sp, 1020
        (0x4398, I::LW   { rd: 14, rs1: 15, offset: 0 }),  // c.lw a4, 0(a5)
        (0x5ff4, I::LW   { rd: 13, rs1: 15, offset: 124 }),// c.lw a3, 124(a5)
        (0xc398, I::SW   { rs1: 15, rs2: 14, offset: 0 }), // c.sw a4, 0(a5)
        (0xdff4, I::SW   { rs1: 15, rs2: 13, offset: 124 }),// c.sw a3, 124(a5)

        // Quadrant 1.
        (0x0001, I::ADDI { rd: 0,  rs1: 0,  imm: 0 }),     // c.nop
        (0x0505, I::ADDI { rd: 10, rs1: 10, imm: 1 }),     // c.addi a0, 1
        (0x157d, I::ADDI { rd: 10, rs1: 10, imm: -1 }),    // c.addi a0, -1
        (0x0541, I::ADDI { rd: 10, rs1: 10, imm: 16 }),    // c.addi a0, 16
        (0x4501, I::ADDI { rd: 10, rs1: 0,  imm: 0 }),     // c.li a0, 0
        (0x557d, I::ADDI { rd: 10, rs1: 0,  imm: -1 }),    // c.li a0, -1
        (0x717d, I::ADDI { rd: 2,  rs1: 2,  imm: -16 }),   // c.addi16sp sp, -16
        (0x6105, I::ADDI { rd: 2,  rs1: 2,  imm: 32 }),    // c.addi16sp sp, 32
        (0x6505, I::LUI  { rd: 10, imm: 0x1000 }),         // c.lui a0, 1
        (0x757d, I::LUI  { rd: 10, imm: -0x1000 }),        // c.lui a0, 0xfffff
        (0x8109, I::SRLI { rd: 10, rs1: 10, shamt: 2 }),   // c.srli a0, 2
        (0x8509, I::SRAI { rd: 10, rs1: 10, shamt: 2 }),   // c.srai a0, 2
        (0x8909, I::ANDI { rd: 10, rs1: 10, imm: 2 }),     // c.andi a0, 2
        (0x8d0d, I::SUB  { rd: 10, rs1: 10, rs2: 11 }),    // c.sub a0, a1
        (0x8d2d, I::XOR  { rd: 10, rs1: 10, rs2: 11 }),    // c.xor a0, a1
        (0x8d4d, I::OR   { rd: 10, rs1: 10, rs2: 11 }),    // c.or a0, a1
        (0x8d6d, I::AND  { rd: 10, rs1: 10, rs2: 11 }),    // c.and a0, a1
        (0xa001, I::JAL  { rd: 0, offset: 0 }),            // c.j .
        (0xbfc5, I::JAL  { rd: 0, offset: -16 }),          // c.j .-16
        (0xaffd, I::JAL  { rd: 0, offset: 2046 }),         // c.j .+2046
        (0xb001, I::JAL  { rd: 0, offset: -2048 }),        // c.j .-2048
        (0x2001, I::JAL  { rd: 1, offset: 0 }),            // c.jal .
        (0x3fc5, I::JAL  { rd: 1, offset: -16 }),          // c.jal .-16
        (0xc111, I::BEQ  { rs1: 10, rs2: 0, offset: 4 }),  // c.beqz a0, .+4
        (0xe111, I::BNE  { rs1: 10, rs2: 0, offset: 4 }),  // c.bnez a0, .+4
        (0xdd65, I::BEQ  { rs1: 10, rs2: 0, offset: -8 }), // c.beqz a0, .-8
        (0xcd7d, I::BEQ  { rs1: 10, rs2: 0, offset: 254 }),// c.beqz a0, .+254
        (0xd101, I::BEQ  { rs1: 10, rs2: 0, offset: -256 }),// c.beqz a0, .-256

        // Quadrant 2.
        (0x0506, I::SLLI { rd: 10, rs1: 10, shamt: 1 }),   // c.slli a0, 1
        (0x057e, I::SLLI { rd: 10, rs1: 10, shamt: 31 }),  // c.slli a0, 31
        (0x4502, I::LW   { rd: 10, rs1: 2, offset: 0 }),   // c.lwsp a0, 0(sp)
        (0x507e, I::LW   { rd: 0,  rs1: 2, offset: 252 }), // (rd=0 form below)
        (0x8582, I::JALR { rd: 0, rs1: 11, offset: 0 }),   // c.jr a1
        (0x852e, I::ADD  { rd: 10, rs1: 0, rs2: 11 }),     // c.mv a0, a1
        (0x9002, I::EBREAK),                               // c.ebreak
        (0x9582, I::JALR { rd: 1, rs1: 11, offset: 0 }),   // c.jalr a1
        (0x952e, I::ADD  { rd: 10, rs1: 10, rs2: 11 }),    // c.add a0, a1
        (0xc02e, I::SW   { rs1: 2, rs2: 11, offset: 0 }),  // c.swsp a1, 0(sp)
        (0xdfae, I::SW   { rs1: 2, rs2: 11, offset: 252 }),// c.swsp a1, 252(sp)
    ];

    #[test]
    fn expands_reference_encodings() {
        for (half, want) in VECTORS {
            // The rd=0 LWSP entry is a placeholder for the illegal case below.
            if *half == 0x507e {
                continue;
            }
            assert_eq!(&decode(*half), want, "expanding {half:#06x}");
        }
    }

    #[test]
    fn detects_the_instruction_width() {
        assert!(is_compressed(0x0505)); // c.addi
        assert!(is_compressed(0x0000)); // illegal, but still 16-bit shaped
        assert!(!is_compressed(0x0513)); // low bits of a 32-bit addi
        for half in 0u16..=0xFFFF {
            assert_eq!(is_compressed(half), half & 3 != 3);
        }
    }

    #[test]
    fn reserved_and_unsupported_encodings_are_illegal() {
        // The all-zero halfword is the canonical illegal instruction.
        assert!(matches!(decode(0x0000), I::UNKNOWN { .. }));
        // C.ADDI4SPN with a zero immediate is reserved.
        assert!(matches!(decode(0x0000 | 0b00), I::UNKNOWN { .. }));
        // C.LWSP with rd = 0 is reserved.
        assert!(matches!(decode(0x507e), I::UNKNOWN { .. }));
        // C.JR with rs1 = 0 is reserved.
        assert!(matches!(decode(0x8002), I::UNKNOWN { .. }));
        // Float loads and stores have no implementation here.
        for funct3 in [0b001u16, 0b011, 0b101, 0b111] {
            assert!(matches!(decode(funct3 << 13), I::UNKNOWN { .. }), "q0 f3={funct3:03b}");
        }
    }

    #[test]
    fn rv64_only_word_forms_are_illegal_here() {
        // C.SUBW / C.ADDW set bit 12 in the MISC-ALU group.
        assert!(matches!(decode(0x9d0d), I::UNKNOWN { .. }));
        assert!(matches!(decode(0x9d2d), I::UNKNOWN { .. }));
    }

    #[test]
    fn nothing_panics_across_the_whole_encoding_space() {
        let mut expanded = 0u32;
        for half in 0u16..=0xFFFF {
            if !is_compressed(half) {
                continue;
            }
            if !matches!(decode(half), I::UNKNOWN { .. }) {
                expanded += 1;
            }
        }
        // Three quarters of the 16-bit space is compressed-shaped, and most of
        // it is a valid instruction.
        assert!(expanded > 25_000, "only {expanded} encodings expanded");
    }

    #[test]
    fn expansions_re_encode_as_their_32_bit_equivalents() {
        // The expansion is what the CPU runs, so it has to be a legal 32-bit
        // instruction in its own right.
        for half in 0u16..=0xFFFF {
            if !is_compressed(half) {
                continue;
            }
            let expanded = decode(half);
            if matches!(expanded, I::UNKNOWN { .. }) {
                continue;
            }
            let word = expanded.encode();
            assert_eq!(
                Instruction::decode(word),
                expanded,
                "{half:#06x} expanded to {expanded:?}, which did not survive re-encoding"
            );
        }
    }
}
