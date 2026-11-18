use crate::{
    chunk::{Code, Opcode},
    heap::{Heap, ObjKind},
};

const TRUE: u64 = 1;
const FALSE: u64 = 0;

struct CallFrame {
    ip: *const u8,
    sp: usize,
}

impl CallFrame {
    fn new(code: &[u8], function: usize, sp: usize) -> Self {
        Self {
            ip: unsafe { code.as_ptr().add(function) },
            sp,
        }
    }

    fn read_instr(&mut self) -> Opcode {
        unsafe { std::mem::transmute(self.read_u8()) }
    }

    fn read_u8(&mut self) -> u8 {
        unsafe {
            let x = *self.ip;
            self.ip = self.ip.add(1);
            x
        }
    }

    fn read_u16(&mut self) -> u16 {
        unsafe {
            let bytes = *(self.ip as *const [u8; 2]);
            let value = u16::from_ne_bytes(bytes);
            self.ip = self.ip.add(2);
            value
        }
    }

    fn read_u64(&mut self) -> u64 {
        unsafe {
            let bytes = *(self.ip as *const [u8; 8]);
            let value = u64::from_ne_bytes(bytes);
            self.ip = self.ip.add(8);
            value
        }
    }

    fn jump(&mut self, offset: usize) {
        unsafe {
            self.ip = self.ip.add(offset);
        }
    }

    fn jump_back(&mut self, offset: usize) {
        unsafe {
            self.ip = self.ip.sub(offset);
        }
    }
}

macro_rules! cast {
    ($expr:expr, $t:ident) => {
        $t::from_ne_bytes($expr.to_ne_bytes())
    };
}

pub struct VM {
    code: Vec<u8>,
    stack: Vec<u64>,
    heap: Heap,
    frame: CallFrame,
    frames: Vec<CallFrame>,
}

impl VM {
    pub fn new(code: Code, heap: Heap) -> Self {
        let code = code.into_inner();
        Self {
            stack: Vec::new(),
            frame: CallFrame::new(&code, 0, 0),
            frames: Vec::new(),
            code,
            heap,
        }
    }

    pub fn run(&mut self) {
        loop {
            let instr = self.frame.read_instr();
            match instr {
                Opcode::True => self.stack.push(TRUE),
                Opcode::False => self.stack.push(FALSE),
                Opcode::Not => {
                    let value = self.stack.pop().unwrap();
                    self.stack.push((value == FALSE) as u64)
                }
                Opcode::Const => {
                    let value = self.frame.read_u64();
                    self.stack.push(value);
                }
                Opcode::F64Neg => {
                    let value = self.stack.pop().unwrap();
                    let value = -f64::from_bits(value);
                    self.stack.push(value.to_bits());
                }
                Opcode::F64Add => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = f64::from_bits(a) + f64::from_bits(b);
                    self.stack.push(value.to_bits());
                }
                Opcode::F64Sub => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = f64::from_bits(a) - f64::from_bits(b);
                    self.stack.push(value.to_bits());
                }
                Opcode::F64Mul => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = f64::from_bits(a) * f64::from_bits(b);
                    self.stack.push(value.to_bits());
                }
                Opcode::F64Div => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = f64::from_bits(a) / f64::from_bits(b);
                    self.stack.push(value.to_bits());
                }
                Opcode::F64Greater => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = f64::from_bits(a) > f64::from_bits(b);
                    self.stack.push(value as u64);
                }
                Opcode::F64GreaterEqual => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = f64::from_bits(a) >= f64::from_bits(b);
                    self.stack.push(value as u64);
                }
                Opcode::F64Less => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = f64::from_bits(a) < f64::from_bits(b);
                    self.stack.push(value as u64);
                }
                Opcode::F64LessEqual => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = f64::from_bits(a) <= f64::from_bits(b);
                    self.stack.push(value as u64);
                }
                Opcode::F64Equal => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = f64::from_bits(a) == f64::from_bits(b);
                    self.stack.push(value as u64);
                }
                Opcode::F64NotEqual => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = f64::from_bits(a) != f64::from_bits(b);
                    self.stack.push(value as u64);
                }
                Opcode::I64Neg => {
                    let value = self.stack.pop().unwrap();
                    let value = -cast!(value, i64);
                    self.stack.push(cast!(value, u64));
                }
                Opcode::I64Add => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = cast!(a, i64) + cast!(b, i64);
                    self.stack.push(cast!(value, u64));
                }
                Opcode::I64Sub => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = cast!(a, i64) - cast!(b, i64);
                    self.stack.push(cast!(value, u64));
                }
                Opcode::I64Mul => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = cast!(a, i64) * cast!(b, i64);
                    self.stack.push(cast!(value, u64));
                }
                Opcode::I64Div => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = cast!(a, i64) / cast!(b, i64);
                    self.stack.push(cast!(value, u64));
                }
                Opcode::I64Greater => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = cast!(a, i64) > cast!(b, i64);
                    self.stack.push(value as u64);
                }
                Opcode::I64GreaterEqual => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = cast!(a, i64) >= cast!(b, i64);
                    self.stack.push(value as u64);
                }
                Opcode::I64Less => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = cast!(a, i64) < cast!(b, i64);
                    self.stack.push(value as u64);
                }
                Opcode::I64LessEqual => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = cast!(a, i64) <= cast!(b, i64);
                    self.stack.push(value as u64);
                }
                Opcode::I64Or => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = cast!(a, i64) | cast!(b, i64);
                    self.stack.push(cast!(value, u64));
                }
                Opcode::I64And => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = cast!(a, i64) & cast!(b, i64);
                    self.stack.push(cast!(value, u64));
                }
                Opcode::U64Add => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = a + b;
                    self.stack.push(value);
                }
                Opcode::U64Sub => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = a - b;
                    self.stack.push(value);
                }
                Opcode::U64Mul => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = a * b;
                    self.stack.push(value);
                }
                Opcode::U64Div => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = a / b;
                    self.stack.push(value);
                }
                Opcode::U64Greater => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = a > b;
                    self.stack.push(value as u64);
                }
                Opcode::U64GreaterEqual => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = a >= b;
                    self.stack.push(value as u64);
                }
                Opcode::U64Less => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = a < b;
                    self.stack.push(value as u64);
                }
                Opcode::U64LessEqual => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = a <= b;
                    self.stack.push(value as u64);
                }
                Opcode::U64Or => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = a | b;
                    self.stack.push(value);
                }
                Opcode::U64And => {
                    let b = self.stack.pop().unwrap();
                    let a = self.stack.pop().unwrap();
                    let value = a & b;
                    self.stack.push(value);
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
                Opcode::GetLocal => {
                    let index = self.frame.read_u8() as usize;
                    self.stack.push(self.stack[self.frame.sp + index]);
                }
                Opcode::GetLocalLong => {
                    let index = self.frame.read_u16() as usize;
                    self.stack.push(self.stack[self.frame.sp + index]);
                }
                Opcode::SetLocal => {
                    let index = self.frame.read_u8() as usize;
                    let value = self.stack.pop().unwrap();
                    self.stack[self.frame.sp + index] = value;
                }
                Opcode::SetLocalLong => {
                    let index = self.frame.read_u16() as usize;
                    let value = self.stack.pop().unwrap();
                    self.stack[self.frame.sp + index] = value;
                }
                Opcode::Call => {
                    let arg_count = self.frame.read_u8() as usize;
                    let callee = self.stack.pop().unwrap();
                    self.call(callee, arg_count);
                }
                Opcode::PrintF64 => {
                    let value = self.stack.pop().unwrap();
                    let value = f64::from_bits(value);
                    println!("{value}");
                }
                Opcode::PrintI64 => {
                    let value = self.stack.pop().unwrap();
                    let value = cast!(value, i64);
                    println!("{value}");
                }
                Opcode::PrintU64 => {
                    let value = self.stack.pop().unwrap();
                    println!("{value}");
                }
                Opcode::PrintString => {
                    let value = self.stack.pop().unwrap();
                    let bytes = self.heap.data(value as usize);
                    let value = std::str::from_utf8(bytes).unwrap();
                    println!("{value}");
                }
                Opcode::PrintBool => {
                    let value = self.stack.pop().unwrap();
                    println!("{}", value == TRUE);
                }
                Opcode::PrintObj => {
                    self.stack.pop().unwrap();
                    println!("<object>");
                }
                Opcode::Unit => self.stack.push(0),
                Opcode::Pop => {
                    self.stack.pop().unwrap();
                }
                Opcode::Jump => {
                    let offset = self.frame.read_u16() as usize;
                    self.frame.jump(offset);
                }
                Opcode::JumpIfFalse => {
                    let value = self.stack.pop().unwrap();
                    let offset = self.frame.read_u16() as usize;
                    if value == FALSE {
                        self.frame.jump(offset);
                    }
                }
                Opcode::Loop => {
                    let offset = self.frame.read_u16() as usize;
                    self.frame.jump_back(offset);
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
                Opcode::Struct => {
                    let count = self.frame.read_u16() as usize;
                    let data = bits_to_bytes(&self.stack[self.stack.len() - count..]);
                    let obj = self.heap.alloc(ObjKind::Struct, data);
                    for _ in 0..count {
                        self.stack.pop().unwrap();
                    }
                    self.stack.push(obj as u64);
                }
                Opcode::RepeatStruct => {
                    let count = self.stack.pop().unwrap();
                    let value = self.stack.pop().unwrap();
                    let data = vec![value; count as usize];
                    let data = bits_to_bytes(&data[..]);
                    let obj = self.heap.alloc(ObjKind::Struct, data);
                    self.stack.push(obj as u64);
                }
                Opcode::StructProperty => {
                    let index = self.frame.read_u16() as usize;
                    let obj = self.stack.pop().unwrap() as usize;
                    let value = self.heap.get_field(obj, index);
                    self.stack.push(value);
                }
                Opcode::SetStructProperty => {
                    let index = self.frame.read_u16() as usize;
                    let value = self.stack.pop().unwrap();
                    let obj = self.stack.pop().unwrap() as usize;
                    self.heap.set_field(obj, index, value);
                }
                Opcode::StructIndex => {
                    let index = self.stack.pop().unwrap();
                    let obj = self.stack.pop().unwrap() as usize;
                    let value = self.heap.get_field(obj, index as usize);
                    self.stack.push(value);
                }
                Opcode::SetStructIndex => {
                    let value = self.stack.pop().unwrap();
                    let index = self.stack.pop().unwrap();
                    let obj = self.stack.pop().unwrap() as usize;
                    self.heap.set_field(obj, index as usize, value);
                }
            }
        }
    }

    fn call(&mut self, function: u64, arg_count: usize) {
        self.frames.push(std::mem::replace(
            &mut self.frame,
            CallFrame::new(&self.code, function as usize, self.stack.len() - arg_count),
        ))
    }
}

fn bits_to_bytes(data: &[u64]) -> &[u8] {
    let ptr = data.as_ptr() as *const u8;
    let len = data.len() * 8;
    unsafe { std::slice::from_raw_parts(ptr, len) }
}
