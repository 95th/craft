use crate::chunk::{Code, Opcode};

pub struct VM {
    code: Vec<u8>,
    ip: usize,
    stack: Vec<u64>,
}

impl VM {
    pub fn new(code: Code) -> Self {
        Self {
            code: code.into_inner(),
            ip: 0,
            stack: Vec::new(),
        }
    }

    pub fn run(&mut self) {
        loop {
            let instr: Opcode = unsafe { std::mem::transmute(self.code[self.ip]) };
            self.ip += 1;
            match instr {
                Opcode::True => self.stack.push(1),
                Opcode::False => self.stack.push(0),
                Opcode::Not => {
                    let value = self.stack.pop().unwrap();
                    self.stack.push((value == 0) as u64)
                }
                Opcode::Negate => {
                    let value = self.stack.pop().unwrap();
                    let value = -f64::from_bits(value);
                    self.stack.push(value.to_bits());
                }
                Opcode::Const => {
                    let value = self.read_u64();
                    self.stack.push(value);
                }
                Opcode::Add => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = f64::from_bits(a) + f64::from_bits(b);
                    self.stack.push(value.to_bits());
                }
                Opcode::Sub => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = f64::from_bits(a) - f64::from_bits(b);
                    self.stack.push(value.to_bits());
                }
                Opcode::Mul => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = f64::from_bits(a) * f64::from_bits(b);
                    self.stack.push(value.to_bits());
                }
                Opcode::Div => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = f64::from_bits(a) / f64::from_bits(b);
                    self.stack.push(value.to_bits());
                }
                Opcode::Greater => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = f64::from_bits(a) > f64::from_bits(b);
                    self.stack.push(value as u64);
                }
                Opcode::GreaterEqual => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = f64::from_bits(a) >= f64::from_bits(b);
                    self.stack.push(value as u64);
                }
                Opcode::Less => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = f64::from_bits(a) < f64::from_bits(b);
                    self.stack.push(value as u64);
                }
                Opcode::LessEqual => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = f64::from_bits(a) <= f64::from_bits(b);
                    self.stack.push(value as u64);
                }
                Opcode::Equal => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = a == b;
                    self.stack.push(value as u64);
                }
                Opcode::NotEqual => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = a != b;
                    self.stack.push(value as u64);
                }
                Opcode::BitOr => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = f64::from_bits(a) as usize | f64::from_bits(b) as usize;
                    self.stack.push((value as f64).to_bits());
                }
                Opcode::BitAnd => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = f64::from_bits(a) as u64 & f64::from_bits(b) as u64;
                    self.stack.push((value as f64).to_bits());
                }
                Opcode::GetLocal => {
                    let index = self.code[self.ip] as usize;
                    self.ip += 1;
                    self.stack.push(self.stack[index]);
                }
                Opcode::GetLocalLong => {
                    let bytes = unsafe { *(self.code.as_ptr().add(self.ip) as *const [u8; 2]) };
                    let index = u16::from_be_bytes(bytes) as usize;
                    self.ip += 2;
                    self.stack.push(self.stack[index]);
                }
                Opcode::SetLocal => {
                    let index = self.code[self.ip] as usize;
                    self.ip += 1;
                    let value = self.stack.pop().unwrap();
                    self.stack[index] = value;
                }
                Opcode::SetLocalLong => {
                    let index = self.read_u16() as usize;
                    let value = self.stack.pop().unwrap();
                    self.stack[index] = value;
                }
                Opcode::Call => {
                    let arg_count = self.code[self.ip] as usize;
                    self.ip += 1;
                    let callee = self.stack.pop().unwrap();
                    let mut args = self.stack[self.stack.len() - arg_count..].to_vec();
                    todo!()
                }
                Opcode::PrintNumber => {
                    let value = self.stack.pop().unwrap();
                    let value = f64::from_bits(value);
                    println!("{value}");
                }
                Opcode::Unit => self.stack.push(0),
                Opcode::Pop => {
                    self.stack.pop().unwrap();
                }
                Opcode::Jump => {
                    let offset = self.read_u16() as usize;
                    self.ip += offset;
                }
                Opcode::JumpIfFalse => {
                    let value = self.stack.pop().unwrap();
                    let offset = self.read_u16() as usize;
                    if value == 0 {
                        self.ip += offset;
                    }
                }
                Opcode::Loop => {
                    let offset = self.read_u16() as usize;
                    self.ip -= offset;
                }
                Opcode::Return => return,
            }
        }
    }

    fn read_u16(&mut self) -> u16 {
        let bytes = unsafe { *(self.code.as_ptr().add(self.ip) as *const [u8; 2]) };
        let value = u16::from_be_bytes(bytes);
        self.ip += 2;
        value
    }

    fn read_u64(&mut self) -> u64 {
        let bytes = unsafe { *(self.code.as_ptr().add(self.ip) as *const [u8; 8]) };
        let value = u64::from_be_bytes(bytes);
        self.ip += 8;
        value
    }
}
