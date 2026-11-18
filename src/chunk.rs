#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Opcode {
    Return,
    Const,
    True,
    False,
    Unit,
    Not,
    Negate,
    GetLocal,
    GetLocalLong,
    SetLocal,
    SetLocalLong,
    PrintNumber,
    PrintString,
    Call,
    Pop,
    JumpIfFalse,
    Jump,
    Loop,
    Struct,
    StructProperty,

    Add,
    Sub,
    Mul,
    Div,
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
    Equal,
    NotEqual,
    BitOr,
    BitAnd,
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
