extern crate sgsae;

use sgsae::instructions::Instruction;
use sgsae::machine::Machine;

fn main() {
    #[rustfmt::skip]
    Machine::new_with_instructions(&[
        Instruction::ADDW {rd: 1, rs1: 1, rs2: 2},
        Instruction::ADDIW { rd: 1, rs: 1, imm: 88},
        Instruction::EBREAK{},
    ])
    .run();
}
