pub type Register = usize;

struct InstructionR {
    funct7: u8,
    rs1: Register,
    rs2: Register,
    funct3: u8,
    rd: Register,
}

impl InstructionR {
    fn decode(x: u32) -> InstructionR {
        InstructionR {
            funct7: ((x >> 25) & 0b1111111) as u8,
            rs2: ((x >> 20) & 0b11111) as Register,
            rs1: ((x >> 15) & 0b11111) as Register,
            funct3: ((x >> 12) & 0b111) as u8,
            rd: ((x >> 7) & 0b11111) as Register,
        }
    }
    fn encode(&self, opcode: u8) -> u32 {
        ((self.funct7 as u32 & 0b1111111) << 25)
            + ((self.rs2 as u32 & 0b11111) << 20)
            + ((self.rs1 as u32 & 0b11111) << 15)
            + ((self.funct3 as u32 & 0b111) << 12)
            + ((self.rd as u32 & 0b11111) << 7)
            + ((opcode as u32 & 0b1111111) << 0)
    }
}

#[derive(Debug)]
pub enum Instruction {
    ADDW {
        rs1: Register,
        rs2: Register,
        rd: Register,
    },
    UNKNOWN,
}

impl Instruction {
    pub fn decode(x: u32) -> Instruction {
        let opcode = x & 0b1111111;
        match opcode {
            // addw, subw, sllw, srlw, sraw
            0b0111011 => match InstructionR::decode(x) {
                InstructionR {
                    funct3: 0,
                    funct7: 0,
                    rs1,
                    rs2,
                    rd,
                } => Instruction::ADDW {
                    rs1: rs1,
                    rs2: rs2,
                    rd: rd,
                },
                _ => Instruction::UNKNOWN,
            },
            _ => Instruction::UNKNOWN,
        }
    }
    pub fn encode(self) -> u32 {
        match self {
            Instruction::ADDW { rs1, rs2, rd } => InstructionR {
                funct7: 0,
                rs1: rs1,
                rs2: rs2,
                funct3: 0,
                rd: rd,
            }
            .encode(0b0111011),
            _ => {
                panic!("unknown instruction");
            }
        }
    }
}

pub struct Machine {
    program_counter: u64,
    registers: [u64; 32],
    memory: [u8; 1024],
}

impl Machine {
    pub fn new() -> Machine {
        Machine {
            program_counter: 0,
            registers: [0; 32],
            memory: [0; 1024],
        }
    }

    pub fn run(&mut self) {
        loop {
            // fetch
            let data = u32::from_be_bytes([
                self.memory[(self.program_counter as usize) + 0],
                self.memory[(self.program_counter as usize) + 1],
                self.memory[(self.program_counter as usize) + 2],
                self.memory[(self.program_counter as usize) + 3],
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
                _ => {
                    panic!("unknown instruction");
                }
            }

            println!("{:?}", self.registers);
        }
    }

    pub fn test_init(&mut self, data: &[u8]) {
        self.memory[..data.len()].copy_from_slice(data);
        self.registers[1] = 1;
    }
}
