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
}
