#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Opcode {
    Return,
    Const,
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
    JumpIfFalse,
    Jump,
    Loop,
    Struct,
    RepeatStruct,
    StructProperty,
    SetStructProperty,
    StructIndex,
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
    main_offset: usize,
}

impl Code {
    pub fn new() -> Self {
        Self {
            code: Vec::new(),
            main_offset: 0,
        }
    }

    pub fn start_main(&mut self) {
        self.main_offset = self.code.len();
    }

    pub fn main(&self) -> usize {
        self.main_offset
    }

    pub fn write(&mut self, op: Opcode) {
        self.code.push(op as u8);
    }

    pub fn pop(&mut self) {
        if self.code.last().copied() == Some(Opcode::Unit as u8) {
            self.code.pop();
        } else {
            self.code.push(Opcode::Pop as u8);
        }
    }

    pub fn write_u8(&mut self, n: u8) {
        self.code.push(n);
    }

    pub fn write_u16(&mut self, n: u16) {
        self.code.extend(n.to_ne_bytes());
    }

    pub fn write_u64(&mut self, n: u64) {
        self.code.extend(n.to_ne_bytes());
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.code
    }

    pub fn len(&self) -> usize {
        self.code.len()
    }

    pub fn write_u16_at(&mut self, i: usize, n: u16) {
        let [a, b] = n.to_ne_bytes();
        self.code[i] = a;
        self.code[i + 1] = b;
    }

    pub fn write_jump(&mut self, op: Opcode) -> usize {
        self.write(op);
        self.write_u16(0);
        self.code.len() - 2
    }

    pub fn patch_jump(&mut self, start: usize) {
        let offset = self.code.len() - start - 2;
        self.write_u16_at(start, offset as u16);
    }

    pub fn write_loop(&mut self, start: usize) {
        let offset = self.code.len() - start + 3;
        self.write(Opcode::Loop);
        self.write_u16(offset as u16);
    }
}
