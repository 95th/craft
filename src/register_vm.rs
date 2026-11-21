use rustc_hash::FxHashMap;

use crate::{
    heap::Heap,
    register_chunk::{Code, Opcode},
};

const TRUE: u64 = 1;
const FALSE: u64 = 0;
const MAX_REGISTER_PER_FRAME: usize = 16 * 1024;

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

    fn read_array<const N: usize>(&mut self) -> [u8; N] {
        unsafe {
            let bytes = *(self.ip as *const [u8; N]);
            self.ip = self.ip.add(N);
            bytes
        }
    }

    fn read_u16(&mut self) -> u16 {
        u16::from_ne_bytes(self.read_array())
    }

    fn read_i32(&mut self) -> i32 {
        i32::from_ne_bytes(self.read_array())
    }

    fn read_u64(&mut self) -> u64 {
        u64::from_ne_bytes(self.read_array())
    }

    fn jump(&mut self, offset: i32) {
        unsafe {
            self.ip = self.ip.offset(offset as isize);
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
    string_pool: FxHashMap<usize, u64>,
}

impl VM {
    pub fn new(code: Code) -> Self {
        let code = code.into_inner();
        Self {
            stack: vec![0; MAX_REGISTER_PER_FRAME],
            frame: CallFrame::new(&code, 0, 0),
            frames: Vec::new(),
            code,
            heap: Heap::new(),
            string_pool: FxHashMap::default(),
        }
    }

    pub fn run(&mut self) {
        loop {
            let instr = self.frame.read_instr();
            match instr {
                Opcode::Const => {
                    let dest = self.frame.read_u16();
                    let value = self.frame.read_u64();
                    self.store(dest, value);
                }
                Opcode::String => {
                    let dest = self.frame.read_u16();
                    let index = self.frame.read_u64() as usize;
                    let obj = *self.string_pool.entry(index).or_insert_with(|| {
                        let len = self.code[index..].first_chunk().unwrap();
                        let len = u32::from_ne_bytes(*len) as usize;
                        let index = index + 4;
                        let data = &self.code[index..index + len];
                        let obj = self.heap.alloc_string(data);
                        obj as u64
                    });
                    self.store(dest, obj);
                }
                Opcode::Call => {
                    let callee = self.frame.read_u16();
                    let function = self.load(callee);
                    self.call(function, callee);
                }
                Opcode::JumpIfTrue => {
                    let cond = self.frame.read_u16();
                    let offset = self.frame.read_i32();
                    let cond = self.load(cond);
                    if cond == TRUE {
                        self.frame.jump(offset);
                    }
                }
                Opcode::JumpIfFalse => {
                    let cond = self.frame.read_u16();
                    let offset = self.frame.read_i32();
                    let cond = self.load(cond);
                    if cond == FALSE {
                        self.frame.jump(offset);
                    }
                }
                Opcode::Jump => {
                    let offset = self.frame.read_i32();
                    self.frame.jump(offset);
                }
                Opcode::Struct => {
                    let dest = self.frame.read_u16();
                    let start = self.frame.read_u16();
                    let count = self.frame.read_u8();
                    let i = self.frame.sp + (start as usize);
                    let fields = &self.stack[i..i + count as usize];
                    let obj = self.heap.alloc_struct(fields);
                    self.store(dest, obj as u64);
                }
                Opcode::RepeatStruct => {
                    let dest = self.frame.read_u16();
                    let value = self.frame.read_u16();
                    let count = self.frame.read_u16();
                    let value = self.load(value);
                    let count = self.load(count) as usize;
                    let data = vec![value; count];
                    let obj = self.heap.alloc_struct(&data);
                    self.store(dest, obj as u64);
                }
                Opcode::GetStructProperty => {
                    let dest = self.frame.read_u16();
                    let obj = self.frame.read_u16();
                    let index = self.frame.read_u8();
                    let obj = self.load(obj) as usize;
                    let value = self.heap.get_field(obj, index as usize);
                    self.store(dest, value);
                }
                Opcode::SetStructProperty => {
                    let obj = self.frame.read_u16();
                    let index = self.frame.read_u8();
                    let value = self.frame.read_u16();
                    let obj = self.load(obj) as usize;
                    let value = self.load(value);
                    self.heap.set_field(obj, index as usize, value);
                }
                Opcode::GetStructIndex => {
                    let dest = self.frame.read_u16();
                    let obj = self.frame.read_u16();
                    let index = self.frame.read_u16();
                    let obj = self.load(obj) as usize;
                    let index = self.load(index) as usize;
                    let value = self.heap.get_field(obj, index);
                    self.store(dest, value);
                }
                Opcode::SetStructIndex => {
                    let obj = self.frame.read_u16();
                    let index = self.frame.read_u16();
                    let value = self.frame.read_u16();
                    let obj = self.load(obj) as usize;
                    let index = self.load(index) as usize;
                    let value = self.load(value);
                    self.heap.set_field(obj, index as usize, value);
                }
                Opcode::Return => {
                    if let Some(frame) = self.frames.pop() {
                        self.frame = frame;
                    } else {
                        return;
                    }
                }
                Opcode::PrintF64 => {
                    let reg = self.frame.read_u16();
                    let value = self.load(reg);
                    let value = f64::from_bits(value);
                    println!("{value}");
                }
                Opcode::PrintI64 => {
                    let reg = self.frame.read_u16();
                    let value = self.load(reg);
                    let value = cast!(value, i64);
                    println!("{value}");
                }
                Opcode::PrintU64 => {
                    let reg = self.frame.read_u16();
                    let value = self.load(reg);
                    println!("{value}");
                }
                Opcode::PrintString => {
                    let reg = self.frame.read_u16();
                    let value = self.load(reg);
                    let bytes = self.heap.data(value as usize);
                    let value = std::str::from_utf8(bytes).unwrap();
                    println!("{value}");
                }
                Opcode::PrintBool => {
                    let reg = self.frame.read_u16();
                    let value = self.load(reg);
                    println!("{}", value == TRUE);
                }
                Opcode::PrintObj => {
                    self.frame.read_u16();
                    println!("<object>");
                }
                Opcode::Copy => {
                    let dest = self.frame.read_u16();
                    let src = self.frame.read_u16();
                    let value = self.load(src);
                    self.store(dest, value);
                }
                Opcode::Not => {
                    let dest = self.frame.read_u16();
                    let src = self.frame.read_u16();
                    let value = self.load(src);
                    self.store(dest, (value == FALSE) as u64);
                }
                Opcode::F64Neg => {
                    let dest = self.frame.read_u16();
                    let src = self.frame.read_u16();
                    let value = self.load(src);
                    let value = -f64::from_bits(value);
                    self.store(dest, value.to_bits());
                }
                Opcode::I64Neg => {
                    let dest = self.frame.read_u16();
                    let src = self.frame.read_u16();
                    let value = self.load(src);
                    let value = cast!(value, i64).wrapping_mul(-1);
                    self.store(dest, cast!(value, u64));
                }
                Opcode::Equal => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let lhs = self.load(lhs);
                    let rhs = self.load(rhs);
                    let value = lhs == rhs;
                    self.store(dest, value as u64);
                }
                Opcode::NotEqual => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let lhs = self.load(lhs);
                    let rhs = self.load(rhs);
                    let value = lhs != rhs;
                    self.store(dest, value as u64);
                }

                Opcode::F64Add => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = f64::from_bits(a) + f64::from_bits(b);
                    self.store(dest, value.to_bits());
                }
                Opcode::F64Sub => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = f64::from_bits(a) - f64::from_bits(b);
                    self.store(dest, value.to_bits());
                }
                Opcode::F64Mul => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = f64::from_bits(a) * f64::from_bits(b);
                    self.store(dest, value.to_bits());
                }
                Opcode::F64Div => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = f64::from_bits(a) / f64::from_bits(b);
                    self.store(dest, value.to_bits());
                }
                Opcode::F64Greater => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = f64::from_bits(a) > f64::from_bits(b);
                    self.store(dest, value as u64);
                }
                Opcode::F64GreaterEqual => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = f64::from_bits(a) >= f64::from_bits(b);
                    self.store(dest, value as u64);
                }
                Opcode::F64Less => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = f64::from_bits(a) < f64::from_bits(b);
                    self.store(dest, value as u64);
                }
                Opcode::F64LessEqual => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = f64::from_bits(a) <= f64::from_bits(b);
                    self.store(dest, value as u64);
                }
                Opcode::F64Equal => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = f64::from_bits(a) == f64::from_bits(b);
                    self.store(dest, value as u64);
                }
                Opcode::F64NotEqual => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = f64::from_bits(a) != f64::from_bits(b);
                    self.store(dest, value as u64);
                }
                Opcode::I64Add => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = cast!(a, i64).wrapping_add(cast!(b, i64));
                    self.store(dest, cast!(value, u64));
                }
                Opcode::I64Sub => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = cast!(a, i64).wrapping_sub(cast!(b, i64));
                    self.store(dest, cast!(value, u64));
                }
                Opcode::I64Mul => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = cast!(a, i64).wrapping_mul(cast!(b, i64));
                    self.store(dest, cast!(value, u64));
                }
                Opcode::I64Div => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = cast!(a, i64).wrapping_div(cast!(b, i64));
                    self.store(dest, cast!(value, u64));
                }
                Opcode::I64Greater => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = cast!(a, i64) > cast!(b, i64);
                    self.store(dest, value as u64);
                }
                Opcode::I64GreaterEqual => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = cast!(a, i64) >= cast!(b, i64);
                    self.store(dest, value as u64);
                }
                Opcode::I64Less => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = cast!(a, i64) < cast!(b, i64);
                    self.store(dest, value as u64);
                }
                Opcode::I64LessEqual => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = cast!(a, i64) <= cast!(b, i64);
                    self.store(dest, value as u64);
                }
                Opcode::I64Or => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = cast!(a, i64) | cast!(b, i64);
                    self.store(dest, cast!(value, u64));
                }
                Opcode::I64And => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = cast!(a, i64) & cast!(b, i64);
                    self.store(dest, cast!(value, u64));
                }
                Opcode::U64Add => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = a.wrapping_add(b);
                    self.store(dest, value);
                }
                Opcode::U64Sub => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = a.wrapping_sub(b);
                    self.store(dest, value);
                }
                Opcode::U64Mul => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = a.wrapping_mul(b);
                    self.store(dest, value);
                }
                Opcode::U64Div => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = a.wrapping_div(b);
                    self.store(dest, value);
                }
                Opcode::U64Greater => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = a > b;
                    self.store(dest, value as u64);
                }
                Opcode::U64GreaterEqual => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = a >= b;
                    self.store(dest, value as u64);
                }
                Opcode::U64Less => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = a < b;
                    self.store(dest, value as u64);
                }
                Opcode::U64LessEqual => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = a <= b;
                    self.store(dest, value as u64);
                }
                Opcode::U64Or => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = a | b;
                    self.store(dest, value);
                }
                Opcode::U64And => {
                    let dest = self.frame.read_u16();
                    let lhs = self.frame.read_u16();
                    let rhs = self.frame.read_u16();
                    let a = self.load(lhs);
                    let b = self.load(rhs);
                    let value = a & b;
                    self.store(dest, value);
                }
            }
        }
    }

    fn load(&mut self, reg: u16) -> u64 {
        self.stack[self.frame.sp + reg as usize]
    }

    fn store(&mut self, reg: u16, value: u64) {
        self.stack[self.frame.sp + reg as usize] = value;
    }

    fn call(&mut self, function: u64, callee_reg: u16) {
        let base = self.frame.sp + callee_reg as usize;
        if self.stack.len() < base + MAX_REGISTER_PER_FRAME {
            self.stack.resize(base + MAX_REGISTER_PER_FRAME, 0);
        }
        self.frames.push(std::mem::replace(
            &mut self.frame,
            CallFrame::new(&self.code, function as usize, base),
        ))
    }
}

#[test]
fn test_run() {
    use std::time::Instant;

    let mut code = Code::new();
    code.op_const(0, 0);
    code.op_const(1, 35);
    code.op_call(0);
    code.op_print_u64(0);
    code.op_return();

    let function = code.len() as u64;

    code.op_const(2, 2);
    code.op_binary(Opcode::U64Less, 3, 1, 2);

    let false_offset = code.op_jump_if_false(3);
    code.op_copy(0, 1);
    code.op_return();

    code.patch_jump(false_offset, code.len());
    code.op_const(4, function);
    code.op_const(5, 1);
    code.op_binary(Opcode::U64Sub, 5, 1, 5);
    code.op_call(4);

    code.op_const(5, function);
    code.op_const(6, 2);
    code.op_binary(Opcode::U64Sub, 6, 1, 6);
    code.op_call(5);

    code.op_binary(Opcode::U64Add, 0, 4, 5);
    code.op_return();

    code.write_u64_at(3, function);

    let mut vm = VM::new(code);
    let start = Instant::now();
    vm.run();
    println!("took: {:?}", start.elapsed());
}
