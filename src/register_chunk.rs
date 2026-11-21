use Opcode::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Opcode {
    // Operand_Dest + u64 constant
    Const,
    String,

    // Operand_Fn
    Call,

    // Operand_Cond + i32 offset
    JumpIfFalse,

    // i32 offset
    Jump,

    // Operand_Dest + Operand_Start + u8 Count
    Struct,

    // Operand_Dest + Operand_Value + Operand_Count
    RepeatStruct,

    // Operand_Dest + Operand_Obj + u8 Index
    GetStructProperty,

    // Operand_Obj + u8 Index + Operand_Value
    SetStructProperty,

    // Operand_Dest + Operand_Array + Operand_Index
    GetStructIndex,

    // Operand_Array + Operand_Index + Operand_Value
    SetStructIndex,

    // No operands
    Return,

    // Single operand
    PrintF64,
    PrintI64,
    PrintU64,
    PrintString,
    PrintBool,
    PrintObj,

    // Two operands
    Copy,
    Not,
    F64Neg,
    I64Neg,

    // Three operands
    Equal,
    NotEqual,

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

pub type Reg = u16;

#[derive(Debug)]
pub struct Code {
    code: Vec<u8>,
}

impl Code {
    pub fn new() -> Self {
        Self { code: Vec::new() }
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.code
    }

    pub fn len(&self) -> usize {
        self.code.len()
    }

    pub fn disassemble(&self, end: usize) {
        if !cfg!(feature = "trace") {
            return;
        }

        Disassembler::new(&self.code).disassemble(end);
    }

    pub fn op_const(&mut self, dest: Reg, value: u64) {
        self.op(Const);
        self.write_u16(dest);
        self.write_u64(value);
    }

    pub fn op_string(&mut self, dest: Reg, value: u64) {
        self.op(String);
        self.write_u16(dest);
        self.write_u64(value);
    }

    pub fn op_call(&mut self, callee: Reg) {
        self.op(Call);
        self.write_u16(callee);
    }

    pub fn op_jump_if_false(&mut self, cond: Reg) -> usize {
        self.op(JumpIfFalse);
        self.write_u16(cond);
        self.write_i32(0);
        self.code.len() - 4
    }

    pub fn op_jump(&mut self) -> usize {
        self.op(Jump);
        self.write_i32(0);
        self.code.len() - 4
    }

    pub fn patch_jump(&mut self, from: usize, to: usize) {
        let offset = to as isize - (from as isize + 4);
        self.write_i32_at(from, offset as i32);
    }

    pub fn op_struct(&mut self, dest: Reg, start: Reg, count: u8) {
        self.op(Struct);
        self.write_u16(dest);
        self.write_u16(start);
        self.write_u8(count);
    }

    pub fn op_repeat_struct(&mut self, dest: Reg, value: Reg, count: Reg) {
        self.op(RepeatStruct);
        self.write_u16(dest);
        self.write_u16(value);
        self.write_u16(count);
    }

    pub fn op_get_struct_property(&mut self, dest: Reg, obj: Reg, index: u8) {
        self.op(GetStructProperty);
        self.write_u16(dest);
        self.write_u16(obj);
        self.write_u8(index);
    }

    pub fn op_set_struct_property(&mut self, obj: Reg, index: u8, value: Reg) {
        self.op(SetStructProperty);
        self.write_u16(obj);
        self.write_u8(index);
        self.write_u16(value);
    }

    pub fn op_get_struct_index(&mut self, dest: Reg, obj: Reg, index: Reg) {
        self.op(GetStructIndex);
        self.write_u16(dest);
        self.write_u16(obj);
        self.write_u16(index);
    }

    pub fn op_set_struct_index(&mut self, obj: Reg, index: Reg, value: Reg) {
        self.op(SetStructIndex);
        self.write_u16(obj);
        self.write_u16(index);
        self.write_u16(value);
    }

    pub fn op_return(&mut self) {
        self.op(Return);
    }

    pub fn op_print_f64(&mut self, value: Reg) {
        self.op(PrintF64);
        self.write_u16(value);
    }

    pub fn op_print_i64(&mut self, value: Reg) {
        self.op(PrintI64);
        self.write_u16(value);
    }

    pub fn op_print_u64(&mut self, value: Reg) {
        self.op(PrintU64);
        self.write_u16(value);
    }

    pub fn op_print_string(&mut self, value: Reg) {
        self.op(PrintString);
        self.write_u16(value);
    }

    pub fn op_print_bool(&mut self, value: Reg) {
        self.op(PrintBool);
        self.write_u16(value);
    }

    pub fn op_print_obj(&mut self, value: Reg) {
        self.op(PrintObj);
        self.write_u16(value);
    }

    pub fn op_copy(&mut self, dest: Reg, src: Reg) {
        self.op(Copy);
        self.write_u16(dest);
        self.write_u16(src);
    }

    pub fn op_not(&mut self, dest: Reg, src: Reg) {
        self.op(Not);
        self.write_u16(dest);
        self.write_u16(src);
    }

    pub fn op_neg_f64(&mut self, dest: Reg, src: Reg) {
        self.op(F64Neg);
        self.write_u16(dest);
        self.write_u16(src);
    }

    pub fn op_neg_i64(&mut self, dest: Reg, src: Reg) {
        self.op(I64Neg);
        self.write_u16(dest);
        self.write_u16(src);
    }

    pub fn op_binary(&mut self, op: Opcode, dest: Reg, lhs: Reg, rhs: Reg) {
        self.op(op);
        self.write_u16(dest);
        self.write_u16(lhs);
        self.write_u16(rhs);
    }

    pub fn write_u64_at(&mut self, i: usize, n: u64) {
        self.code[i..i + 8].copy_from_slice(&n.to_ne_bytes());
    }

    pub fn write_string(&mut self, s: &str) {
        let len = s.len() as u32;
        self.code.extend_from_slice(&len.to_ne_bytes());
        self.code.extend_from_slice(s.as_bytes());
    }

    fn op(&mut self, op: Opcode) {
        self.write_u8(op as u8);
    }

    fn write_u8(&mut self, n: u8) {
        self.code.push(n);
    }

    fn write_u16(&mut self, n: u16) {
        self.code.extend(n.to_ne_bytes());
    }

    fn write_i32(&mut self, n: i32) {
        self.code.extend(n.to_ne_bytes());
    }

    fn write_u64(&mut self, n: u64) {
        self.code.extend(n.to_ne_bytes());
    }

    fn write_i32_at(&mut self, i: usize, n: i32) {
        self.code[i..i + 4].copy_from_slice(&n.to_ne_bytes());
    }
}

struct Disassembler<'a> {
    code: &'a [u8],
    i: usize,
}

impl<'a> Disassembler<'a> {
    fn new(code: &'a [u8]) -> Self {
        Self { code, i: 0 }
    }

    pub fn disassemble(&mut self, end: usize) {
        println!("-- disassembly --");
        while self.i < end {
            let instr: Opcode = unsafe { std::mem::transmute(self.read_u8()) };
            print!("{:<05} {instr:?}", self.i - 1);
            match instr {
                Const => {
                    let dest = self.read_u16();
                    let n = self.read_u64();
                    println!(" r{dest} = {n}");
                }
                String => {
                    let dest = self.read_u16();
                    let n = self.read_u64() as usize;
                    let len = self.code[n..].first_chunk().unwrap();
                    let len = u32::from_ne_bytes(*len) as usize;
                    let s = &self.code[n + 4..][..len];
                    let s = std::str::from_utf8(s).unwrap();
                    println!(" r{dest} = {n} ({s:?})");
                }
                Call => {
                    let callee = self.read_u16();
                    println!(" r{callee}");
                }
                JumpIfFalse => {
                    let cond = self.read_u16();
                    let offset = self.read_i32() as isize;
                    let target = self.i as isize + offset;
                    println!(" r{cond} {offset} (target: {target})");
                }
                Jump => {
                    let offset = self.read_i32() as isize;
                    let target = self.i as isize + offset;
                    println!(" {offset} (target: {target})");
                }
                Struct => {
                    let dest = self.read_u16();
                    let start = self.read_u16();
                    let count = self.read_u8();
                    println!(" r{dest} = r{start} x {count}");
                }
                RepeatStruct => {
                    let dest = self.read_u16();
                    let start = self.read_u16();
                    let count = self.read_u16();
                    println!(" r{dest} = r{start} x r{count}");
                }
                GetStructProperty => {
                    let dest = self.read_u16();
                    let obj = self.read_u16();
                    let index = self.read_u8();
                    println!(" r{dest} = r{obj} '.' {index}");
                }
                SetStructProperty => {
                    let obj = self.read_u16();
                    let index = self.read_u8();
                    let value = self.read_u16();
                    println!(" r{obj} '.' {index} = r{value}");
                }
                GetStructIndex => {
                    let dest = self.read_u16();
                    let obj = self.read_u16();
                    let index = self.read_u16();
                    println!(" r{dest} = r{obj} [ r{index} ]");
                }
                SetStructIndex => {
                    let obj = self.read_u16();
                    let index = self.read_u16();
                    let value = self.read_u16();
                    println!(" r{obj} [ r{index} ] = r{value}");
                }
                Return => {
                    println!();
                }
                PrintF64 | PrintI64 | PrintU64 | PrintString | PrintBool | PrintObj => {
                    let value = self.read_u16();
                    println!(" r{value}");
                }
                Copy | Not | F64Neg | I64Neg => {
                    let dest = self.read_u16();
                    let src = self.read_u16();
                    println!(" r{dest} = r{src}");
                }
                _ => {
                    let dest = self.read_u16();
                    let lhs = self.read_u16();
                    let rhs = self.read_u16();
                    println!(" r{dest} = r{lhs}, r{rhs}");
                }
            }
        }
        println!("----");
    }

    fn read_u8(&mut self) -> u8 {
        let n = self.code[self.i];
        self.i += 1;
        n
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

    fn read_array<const N: usize>(&mut self) -> [u8; N] {
        let x = self.code[self.i..].first_chunk().unwrap();
        self.i += N;
        *x
    }
}
