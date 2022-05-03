extern crate sgsae;

use std::fs::File;

use sgsae::instructions::Instruction;
use sgsae::machine::Machine;

fn main() -> Result<(), std::io::Error> {
    let program = [
        // main:
        Instruction::ADDI { rd: 10, rs: 0, imm: 7 }, // a = 7
        Instruction::ADDI { rd: 11, rs: 0, imm: 1000 }, // b = 8
        Instruction::AUIPC { rd: 6, offset: 0 },     //
        Instruction::JALR { rd: 1, rs: 6, offset: 2 * 4 }, // a = mul(a,b)
        Instruction::EBREAK {},                      // exit
        Instruction::ADD { rd: 12, rs1: 10, rs2: 0 }, // c = a
        Instruction::ADDI { rd: 10, rs: 0, imm: 0 }, // a = 0
        Instruction::ADDI { rd: 12, rs: 12, imm: -1 }, // loop: c = c - 1
        Instruction::ADD { rd: 10, rs1: 10, rs2: 11 }, // a = a + b
        Instruction::BNE { rs1: 12, rs2: 0, offset: -3 * 4 }, // if c != 0 then loop
        Instruction::JALR { rd: 0, rs: 1, offset: 0 }, // return
    ];

    let default_bin = {
        let mut default_bin = Vec::new();
        for instruction in program {
            default_bin.extend(instruction.encode().to_le_bytes());
        }
        default_bin
    };

    let memory_bytes = {
        let mut memory_bytes = [0; 65536];

        memory_bytes[..default_bin.len()].copy_from_slice(default_bin.as_slice());
        memory_bytes
    };

    let mut m = Machine::new_with_memory(memory_bytes);
    if let Err(e) = m.run() {
        eprintln!("error: {}", e)
    }
    println!("registers: {:#018X?}", m.get_registers());

    Ok(())
}
