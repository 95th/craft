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

pub type Reg = u8;

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

        println!("-- disassembly --");
        let mut i = 0;
        while i < end {
            let instr: Opcode = unsafe { std::mem::transmute(self.code[i]) };
            print!("{i:<05} {instr:?}");
            i += 1;
            match instr {
                Opcode::Const => {
                    let [dest, n @ ..] = self.code[i..].first_chunk::<9>().unwrap();
                    i += 9;
                    let n = u64::from_ne_bytes(*n);
                    println!(" r{dest} = {n}");
                }
                Opcode::String => {
                    let [dest, n @ ..] = self.code[i..].first_chunk::<9>().unwrap();
                    i += 9;
                    let n = u64::from_ne_bytes(*n) as usize;
                    let len = self.code[n..].first_chunk().unwrap();
                    let len = u32::from_ne_bytes(*len) as usize;
                    let s = &self.code[n + 4..][..len];
                    let s = std::str::from_utf8(s).unwrap();
                    println!(" r{dest} = {n} ({s:?})");
                }
                Opcode::Call => {
                    let callee = self.code[i];
                    i += 1;
                    println!(" r{callee}");
                }
                Opcode::JumpIfFalse => {
                    let [cond, offset @ ..] = self.code[i..].first_chunk::<5>().unwrap();
                    i += 5;
                    let offset = i32::from_ne_bytes(*offset) as isize;
                    let target = i as isize + offset;
                    println!(" r{cond} {offset} (target: {target})");
                }
                Opcode::Jump => {
                    let offset = self.code[i..].first_chunk().unwrap();
                    i += 4;
                    let offset = i32::from_ne_bytes(*offset) as isize;
                    let target = i as isize + offset;
                    println!(" {offset} (target: {target})");
                }
                Opcode::Struct => {
                    let [dest, start, count] = self.code[i..].first_chunk().unwrap();
                    println!(" r{dest} = r{start} x {count}");
                    i += 3;
                }
                Opcode::RepeatStruct => {
                    let [dest, start, count] = self.code[i..].first_chunk().unwrap();
                    println!(" r{dest} = r{start} x r{count}");
                    i += 3;
                }
                Opcode::GetStructProperty => {
                    let [dest, obj, index] = self.code[i..].first_chunk().unwrap();
                    println!(" r{dest} = r{obj} '.' {index}");
                    i += 3;
                }
                Opcode::SetStructProperty => {
                    let [obj, index, value] = self.code[i..].first_chunk().unwrap();
                    println!(" r{obj} '.' {index} = r{value}");
                    i += 3;
                }
                Opcode::GetStructIndex => {
                    let [dest, obj, index] = self.code[i..].first_chunk().unwrap();
                    println!(" r{dest} = r{obj} [ r{index} ]");
                    i += 3;
                }
                Opcode::SetStructIndex => {
                    let [obj, index, value] = self.code[i..].first_chunk().unwrap();
                    println!(" r{obj} [ r{index} ] = r{value}");
                    i += 3;
                }
                Opcode::Return => {
                    println!();
                }
                Opcode::PrintF64
                | Opcode::PrintI64
                | Opcode::PrintU64
                | Opcode::PrintString
                | Opcode::PrintBool
                | Opcode::PrintObj => {
                    let value = self.code[i];
                    i += 1;
                    println!(" r{value}");
                }
                Opcode::Copy | Opcode::Not | Opcode::F64Neg | Opcode::I64Neg => {
                    let [dest, src] = self.code[i..].first_chunk().unwrap();
                    println!(" r{dest} = r{src}");
                    i += 2;
                }
                _ => {
                    let [dest, lhs, rhs] = self.code[i..].first_chunk().unwrap();
                    println!(" r{dest} = r{lhs}, r{rhs}");
                    i += 3;
                }
            }
        }
        println!("----");
    }

    pub fn op_const(&mut self, dest: Reg, value: u64) {
        self.op(Const);
        self.write_u8(dest);
        self.write_u64(value);
    }

    pub fn op_string(&mut self, dest: Reg, value: u64) {
        self.op(String);
        self.write_u8(dest);
        self.write_u64(value);
    }

    pub fn op_call(&mut self, callee: Reg) {
        self.op(Call);
        self.write_u8(callee);
    }

    pub fn op_jump_if_false(&mut self, cond: Reg) -> usize {
        self.op(JumpIfFalse);
        self.write_u8(cond);
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
        self.write_u8(dest);
        self.write_u8(start);
        self.write_u8(count);
    }

    pub fn op_repeat_struct(&mut self, dest: Reg, value: Reg, count: Reg) {
        self.op(RepeatStruct);
        self.write_u8(dest);
        self.write_u8(value);
        self.write_u8(count);
    }

    pub fn op_get_struct_property(&mut self, dest: Reg, obj: Reg, index: u8) {
        self.op(GetStructProperty);
        self.write_u8(dest);
        self.write_u8(obj);
        self.write_u8(index);
    }

    pub fn op_set_struct_property(&mut self, obj: Reg, index: u8, value: Reg) {
        self.op(SetStructProperty);
        self.write_u8(obj);
        self.write_u8(index);
        self.write_u8(value);
    }

    pub fn op_get_struct_index(&mut self, dest: Reg, obj: Reg, index: Reg) {
        self.op(GetStructIndex);
        self.write_u8(dest);
        self.write_u8(obj);
        self.write_u8(index);
    }

    pub fn op_set_struct_index(&mut self, obj: Reg, index: Reg, value: Reg) {
        self.op(SetStructIndex);
        self.write_u8(obj);
        self.write_u8(index);
        self.write_u8(value);
    }

    pub fn op_return(&mut self) {
        self.op(Return);
    }

    pub fn op_print_f64(&mut self, value: Reg) {
        self.op(PrintF64);
        self.write_u8(value);
    }

    pub fn op_print_i64(&mut self, value: Reg) {
        self.op(PrintI64);
        self.write_u8(value);
    }

    pub fn op_print_u64(&mut self, value: Reg) {
        self.op(PrintU64);
        self.write_u8(value);
    }

    pub fn op_print_string(&mut self, value: Reg) {
        self.op(PrintString);
        self.write_u8(value);
    }

    pub fn op_print_bool(&mut self, value: Reg) {
        self.op(PrintBool);
        self.write_u8(value);
    }

    pub fn op_print_obj(&mut self, value: Reg) {
        self.op(PrintObj);
        self.write_u8(value);
    }

    pub fn op_copy(&mut self, dest: Reg, src: Reg) {
        self.op(Copy);
        self.write_u8(dest);
        self.write_u8(src);
    }

    pub fn op_not(&mut self, dest: Reg, src: Reg) {
        self.op(Not);
        self.write_u8(dest);
        self.write_u8(src);
    }

    pub fn op_neg_f64(&mut self, dest: Reg, src: Reg) {
        self.op(F64Neg);
        self.write_u8(dest);
        self.write_u8(src);
    }

    pub fn op_neg_i64(&mut self, dest: Reg, src: Reg) {
        self.op(I64Neg);
        self.write_u8(dest);
        self.write_u8(src);
    }

    pub fn op_binary(&mut self, op: Opcode, dest: Reg, lhs: Reg, rhs: Reg) {
        self.op(op);
        self.write_u8(dest);
        self.write_u8(lhs);
        self.write_u8(rhs);
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
