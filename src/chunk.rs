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
    Call,
    Pop,
    JumpIfFalse,
    Jump,

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
}

impl Code {
    pub fn new() -> Self {
        Self { code: Vec::new() }
    }

    pub fn write(&mut self, op: Opcode) {
        self.code.push(op as u8);
    }

    pub fn write_u8(&mut self, n: u8) {
        self.code.push(n);
    }

    pub fn write_u16(&mut self, n: u16) {
        self.code.extend(n.to_be_bytes());
    }

    pub fn write_u64(&mut self, n: u64) {
        self.code.extend(n.to_be_bytes());
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.code
    }

    pub fn write_u16_at(&mut self, i: usize, n: u16) {
        let [a, b] = n.to_be_bytes();
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
}
