use crate::instructions::Instruction;
use crate::types::ProgramCounter;

pub struct Machine {
    program_counter: ProgramCounter,
    registers: [u64; 32],
    memory: [u8; 65536],
}

impl Machine {
    pub fn new() -> Machine {
        Machine {
            program_counter: 0,
            registers: [0; 32],
            memory: [0; 65536],
        }
    }

    // Constructor for tests and experiments. The provided instructions will be copied into memory and executed.
    pub fn new_with_instructions(instructions: &[Instruction]) -> Machine {
        let mut m = Machine::new();
        let mut i = 0; // current address

        for instruction in instructions {
            let bytes = instruction.encode().to_le_bytes();
            m.memory[i..i + 4].copy_from_slice(&bytes);
            i += bytes.len();
        }
        return m;
    }

    //
    /// ```
    /// let x = 5;
    /// ```
    pub fn run(&mut self) {
        loop {
            // fetch
            let data = u32::from_le_bytes([
                self.memory[(self.program_counter) + 0],
                self.memory[(self.program_counter) + 1],
                self.memory[(self.program_counter) + 2],
                self.memory[(self.program_counter) + 3],
            ]);

            self.program_counter += 4;

            // decode
            let instruction = Instruction::decode(data);
            println!("{:?}", instruction);

            // execute
            match instruction {
                Instruction::ADDW { rs1, rs2, rd } => {
                    self.registers[rd] = self.registers[rs1] + self.registers[rs2];
                }
                Instruction::ADDIW { imm, rs, rd } => {
                    self.registers[rd] = self.registers[rs] + imm as u64
                }
                Instruction::EBREAK {} => return,
                _ => {
                    panic!("unknown instruction");
                }
            }

            println!("{:?}", self.registers);
        }
    }

    pub fn init(&mut self, data: &[u8]) {
        self.memory[..data.len()].copy_from_slice(data);
        self.registers[1] = 1; // remove once addi implemented
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        // let mut m = crate::machine::Machine::new_with_instructions(&[
        //     crate::instructions::Instruction::ADDW {
        //         rd: 0,
        //         rs1: 1,
        //         rs2: 2,
        //     },
        // ]);
        // m.run();
    }
}
