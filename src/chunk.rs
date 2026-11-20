#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Opcode {
    Return,
    Const,
    String,
    True,
    False,
    Unit,
    Not,
    GetLocal,
    GetLocalLong,
    SetLocal,
    SetLocalLong,

    PrintF64,
    PrintI64,
    PrintU64,
    PrintString,
    PrintBool,
    PrintObj,

    Call,
    Pop,
    PopN,
    PopScope,
    JumpIfFalse,
    Jump,
    Loop,
    Struct,
    RepeatStruct,
    GetStructProperty,
    SetStructProperty,
    GetStructIndex,
    SetStructIndex,

    Equal,
    NotEqual,

    F64Neg,
    F64Add,
    F64Sub,
    F64Mul,
    F64Div,
    F64Greater,
    F64GreaterEqual,
    F64Less,
    F64LessEqual,
    F64Equal,
    F64NotEqual,

    I64Neg,
    I64Add,
    I64Sub,
    I64Mul,
    I64Div,
    I64Greater,
    I64GreaterEqual,
    I64Less,
    I64LessEqual,
    I64Or,
    I64And,

    U64Add,
    U64Sub,
    U64Mul,
    U64Div,
    U64Greater,
    U64GreaterEqual,
    U64Less,
    U64LessEqual,
    U64Or,
    U64And,
}

#[derive(Debug)]
pub struct Code {
    code: Vec<u8>,
}

impl Code {
    pub fn new() -> Self {
        Self { code: Vec::new() }
    }

    pub fn disassemble(&self, end: usize) {
        if !cfg!(feature = "trace") {
            return;
        }

        println!("-- disassembly --");
        let mut i = 0;
        while i < end {
            let instr: Opcode = unsafe { std::mem::transmute(self.code[i]) };
            print!("{i:<05} {instr:?}");
            i += 1;
            match instr {
                Opcode::Const => {
                    let n = self.code[i..].first_chunk().unwrap();
                    let n = u64::from_ne_bytes(*n);
                    println!(" {n}");
                    i += 8;
                }
                Opcode::GetLocal | Opcode::SetLocal => {
                    let n = self.code[i];
                    println!(" {n}");
                    i += 1;
                }
                Opcode::GetLocalLong
                | Opcode::SetLocalLong
                | Opcode::PopN
                | Opcode::PopScope
                | Opcode::Struct
                | Opcode::GetStructProperty
                | Opcode::SetStructProperty => {
                    let n = self.code[i..].first_chunk().unwrap();
                    let n = u16::from_ne_bytes(*n);
                    println!(" {n}");
                    i += 2;
                }
                Opcode::JumpIfFalse | Opcode::Jump => {
                    let n = self.code[i..].first_chunk().unwrap();
                    let n = u16::from_ne_bytes(*n) as usize;
                    println!(" {} (target: {})", n, i + 2 + n);
                    i += 2;
                }
                Opcode::Loop => {
                    let n = self.code[i..].first_chunk().unwrap();
                    let n = u16::from_ne_bytes(*n) as usize;
                    println!(" {} (target: {})", n, i + 2 - n);
                    i += 2;
                }
                Opcode::Call => {
                    let n = self.code[i];
                    println!(" ({n} args)");
                    i += 1;
                }
                Opcode::Equal
                | Opcode::NotEqual
                | Opcode::F64Neg
                | Opcode::F64Add
                | Opcode::F64Sub
                | Opcode::F64Mul
                | Opcode::F64Div
                | Opcode::F64Greater
                | Opcode::F64GreaterEqual
                | Opcode::F64Less
                | Opcode::F64LessEqual
                | Opcode::F64Equal
                | Opcode::F64NotEqual
                | Opcode::I64Neg
                | Opcode::I64Add
                | Opcode::I64Sub
                | Opcode::I64Mul
                | Opcode::I64Div
                | Opcode::I64Greater
                | Opcode::I64GreaterEqual
                | Opcode::I64Less
                | Opcode::I64LessEqual
                | Opcode::I64Or
                | Opcode::I64And
                | Opcode::U64Add
                | Opcode::U64Sub
                | Opcode::U64Mul
                | Opcode::U64Div
                | Opcode::U64Greater
                | Opcode::U64GreaterEqual
                | Opcode::U64Less
                | Opcode::U64LessEqual
                | Opcode::U64Or
                | Opcode::U64And
                | Opcode::True
                | Opcode::False
                | Opcode::Unit
                | Opcode::Not
                | Opcode::Pop
                | Opcode::PrintBool
                | Opcode::PrintF64
                | Opcode::PrintI64
                | Opcode::PrintObj
                | Opcode::PrintString
                | Opcode::PrintU64
                | Opcode::RepeatStruct
                | Opcode::GetStructIndex
                | Opcode::SetStructIndex
                | Opcode::String
                | Opcode::Return => println!(),
            }
        }
        println!("----");
    }

    pub fn op_const(&mut self, value: u64) {
        self.op(Opcode::Const);
        self.write_u64(value);
    }

    pub fn op_return(&mut self) {
        self.op(Opcode::Return);
    }

    pub fn op_struct(&mut self, args: u16) {
        self.op(Opcode::Struct);
        self.write_u16(args);
    }

    pub fn op_repeat_struct(&mut self) {
        self.op(Opcode::RepeatStruct);
    }

    pub fn op_get_struct_property(&mut self, index: u16) {
        self.op(Opcode::GetStructProperty);
        self.write_u16(index);
    }

    pub fn op_set_struct_property(&mut self, index: u16) {
        self.op(Opcode::SetStructProperty);
        self.write_u16(index);
    }

    pub fn op_get_struct_index(&mut self) {
        self.op(Opcode::GetStructIndex);
    }

    pub fn op_set_struct_index(&mut self) {
        self.op(Opcode::SetStructIndex);
    }

    pub fn op_unit(&mut self) {
        self.op(Opcode::Unit);
    }

    pub fn op_call(&mut self, args: u8) {
        self.op(Opcode::Call);
        self.write_u8(args);
    }

    pub fn op_get_local(&mut self, index: u32) {
        if index <= u8::MAX as u32 {
            self.op(Opcode::GetLocal);
            self.write_u8(index as u8);
        } else {
            self.op(Opcode::GetLocalLong);
            self.write_u16(index as u16);
        }
    }

    pub fn op_set_local(&mut self, index: u32) {
        if index <= u8::MAX as u32 {
            self.op(Opcode::SetLocal);
            self.write_u8(index as u8);
        } else {
            self.op(Opcode::SetLocalLong);
            self.write_u16(index as u16);
        }
    }

    pub fn pop(&mut self) {
        if self.code.last().copied() == Some(Opcode::Unit as u8) {
            self.code.pop();
        } else {
            self.code.push(Opcode::Pop as u8);
        }
    }

    pub fn op_pop_scope(&mut self, locals: usize) {
        if locals == 0 {
            return;
        }
        self.op(Opcode::PopScope);
        self.write_u16(locals as u16);
    }

    pub fn op_pop_n(&mut self, n: usize) {
        match n {
            0 => {}
            1 => self.pop(),
            2 => {
                self.pop();
                self.pop();
            }
            _ => {
                self.op(Opcode::PopN);
                self.write_u16(n as u16);
            }
        }
    }

    pub fn write_jump(&mut self, op: Opcode) -> usize {
        self.op(op);
        self.write_u16(0);
        self.code.len() - 2
    }

    pub fn patch_jump(&mut self, start: usize) {
        let offset = self.code.len() - start - 2;
        self.write_u16_at(start, offset as u16);
    }

    pub fn write_loop(&mut self, start: usize) {
        let offset = self.code.len() - start + 3;
        self.op(Opcode::Loop);
        self.write_u16(offset as u16);
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.code
    }

    pub fn len(&self) -> usize {
        self.code.len()
    }

    pub fn op(&mut self, op: Opcode) {
        self.write_u8(op as u8);
    }

    pub fn write_u64_at(&mut self, i: usize, n: u64) {
        self.code[i..i + 8].copy_from_slice(&n.to_ne_bytes());
    }

    pub fn write_string(&mut self, s: &str) {
        let len = s.len() as u32;
        self.code.extend_from_slice(&len.to_ne_bytes());
        self.code.extend_from_slice(s.as_bytes());
    }

    fn write_u8(&mut self, n: u8) {
        self.code.push(n);
    }

    fn write_u16(&mut self, n: u16) {
        self.code.extend(n.to_ne_bytes());
    }

    fn write_u64(&mut self, n: u64) {
        self.code.extend(n.to_ne_bytes());
    }

    fn write_u16_at(&mut self, i: usize, n: u16) {
        self.code[i..i + 2].copy_from_slice(&n.to_ne_bytes());
    }
}
