extern crate sgsae;

use sgsae::instructions::Instruction;
use sgsae::machine::Machine;

fn main() {
    let mut m = Machine::new_with_instructions(&[
        Instruction::ADDIW { rd: 28, rs: 0, imm: 1 },
        Instruction::ADD { rd: 28, rs1: 28, rs2: 28 },
        Instruction::ADD { rd: 28, rs1: 28, rs2: 28 },
        Instruction::ADD { rd: 28, rs1: 28, rs2: 28 },
        Instruction::ADD { rd: 28, rs1: 28, rs2: 28 },
        Instruction::EBREAK {},
    ]);
    if let Err(e) = m.run() {
        eprintln!("error: {}", e)
    }
    println!("registers: {:#018X?}", m.get_registers())
}
