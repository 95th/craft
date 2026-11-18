use crate::chunk::{Code, Opcode};

const TRUE: u64 = 1;
const FALSE: u64 = 0;

struct CallFrame {
    ip: usize,
    sp: usize,
}

pub struct VM {
    code: Vec<u8>,
    stack: Vec<u64>,
    frame: CallFrame,
    frames: Vec<CallFrame>,
}

impl VM {
    pub fn new(code: Code) -> Self {
        Self {
            stack: Vec::new(),
            frame: CallFrame {
                ip: code.main(),
                sp: 0,
            },
            frames: Vec::new(),
            code: code.into_inner(),
        }
    }

    pub fn run(&mut self) {
        loop {
            let instr: Opcode = unsafe { std::mem::transmute(self.code[self.frame.ip]) };
            self.frame.ip += 1;
            match instr {
                Opcode::True => self.stack.push(TRUE),
                Opcode::False => self.stack.push(FALSE),
                Opcode::Not => {
                    let value = self.stack.pop().unwrap();
                    self.stack.push((value == FALSE) as u64)
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
                    let index = self.code[self.frame.ip] as usize;
                    self.frame.ip += 1;
                    self.stack.push(self.stack[self.frame.sp + index]);
                }
                Opcode::GetLocalLong => {
                    let bytes =
                        unsafe { *(self.code.as_ptr().add(self.frame.ip) as *const [u8; 2]) };
                    let index = u16::from_be_bytes(bytes) as usize;
                    self.frame.ip += 2;
                    self.stack.push(self.stack[self.frame.sp + index]);
                }
                Opcode::SetLocal => {
                    let index = self.code[self.frame.ip] as usize;
                    self.frame.ip += 1;
                    let value = self.stack.pop().unwrap();
                    self.stack[self.frame.sp + index] = value;
                }
                Opcode::SetLocalLong => {
                    let index = self.read_u16() as usize;
                    let value = self.stack.pop().unwrap();
                    self.stack[self.frame.sp + index] = value;
                }
                Opcode::Call => {
                    let arg_count = self.code[self.frame.ip] as usize;
                    self.frame.ip += 1;
                    let callee = self.stack.pop().unwrap();
                    self.call(callee, arg_count);
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
                    self.frame.ip += offset;
                }
                Opcode::JumpIfFalse => {
                    let value = self.stack.pop().unwrap();
                    let offset = self.read_u16() as usize;
                    if value == FALSE {
                        self.frame.ip += offset;
                    }
                }
                Opcode::Loop => {
                    let offset = self.read_u16() as usize;
                    self.frame.ip -= offset;
                }
                Opcode::Return => {
                    if let Some(frame) = self.frames.pop() {
                        let value = self.stack.pop().unwrap();
                        self.stack.truncate(self.frame.sp);
                        self.stack.push(value);
                        self.frame = frame;
                    } else {
                        return;
                    }
                }
            }
        }
    }

    fn call(&mut self, function: u64, arg_count: usize) {
        self.frames.push(std::mem::replace(
            &mut self.frame,
            CallFrame {
                ip: function as usize,
                sp: self.stack.len() - arg_count,
            },
        ))
    }

    fn read_u16(&mut self) -> u16 {
        let bytes = unsafe { *(self.code.as_ptr().add(self.frame.ip) as *const [u8; 2]) };
        let value = u16::from_be_bytes(bytes);
        self.frame.ip += 2;
        value
    }

    fn read_u64(&mut self) -> u64 {
        let bytes = unsafe { *(self.code.as_ptr().add(self.frame.ip) as *const [u8; 8]) };
        let value = u64::from_be_bytes(bytes);
        self.frame.ip += 8;
        value
    }
}
