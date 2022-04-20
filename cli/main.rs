extern crate sgsae;

fn addw(rs1: sgsae::Register, rs2: sgsae::Register, rd: sgsae::Register) -> sgsae::Instruction {
    sgsae::Instruction::ADDW {
        rs2: rs2,
        rs1: rs1,
        rd: rd,
    }
}

// 12345678 90123456 78901234 56789012
// [0b00000000, 0b00010000, 0b00000000, 0b00111011]
fn main() {
    let bytes = addw(0, 1, 0).encode().to_be_bytes();

    let mut m = sgsae::Machine::new();
    m.test_init(&[bytes, bytes, bytes].concat());
    m.run();
}
