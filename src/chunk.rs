#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Opcode {
    Return,
    Add,
    Sub,
    Mul,
    Div,
    Const,
}

pub struct Code {
    code: Vec<u8>,
}

impl Code {
    pub fn new() -> Self {
        Self { code: Vec::new() }
    }

    pub fn push(&mut self, op: Opcode) {
        self.code.push(op as u8);
    }
}
