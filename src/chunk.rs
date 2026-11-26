use Opcode::*;
use bitvec::vec::BitVec;

use crate::heap::{Layout, LayoutBuilder, Layouts};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Opcode {
    // Operand_Dest + u64 constant
    Const,
    String,

    // Operand_Callee
    Call,
    // Operand_Dest + u32 FnAddress
    CallI,

    // Operand_Cond + i32 offset
    JumpIfTrue,
    JumpIfFalse,

    // i32 offset
    Jump,

    // u16_Layout + Operand_Dest + Operand_First + u8_Count
    Struct,

    // u16_Layout + Operand_Dest + Operand_Value + Operand_Count
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
    Assert,

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

    // Operand_Dest + Operand_Lhs + Constant_Rhs
    EqualI,
    NotEqualI,

    F64AddI,
    F64SubI,
    F64MulI,
    F64DivI,
    F64GreaterI,
    F64GreaterEqualI,
    F64LessI,
    F64LessEqualI,
    F64EqualI,
    F64NotEqualI,

    I64AddI,
    I64SubI,
    I64MulI,
    I64DivI,
    I64GreaterI,
    I64GreaterEqualI,
    I64LessI,
    I64LessEqualI,
    I64OrI,
    I64AndI,

    U64AddI,
    U64SubI,
    U64MulI,
    U64DivI,
    U64GreaterI,
    U64GreaterEqualI,
    U64LessI,
    U64LessEqualI,
    U64OrI,
    U64AndI,

    FnStart = u8::MAX,
}

pub type Reg = u16;

pub struct Code {
    code: Vec<u8>,
    layouts: LayoutBuilder,
    safepoints: Vec<(usize, BitVec)>,
}

impl Code {
    pub fn new() -> Self {
        Self {
            code: Vec::new(),
            layouts: LayoutBuilder::new(),
            safepoints: Vec::new(),
        }
    }

    pub fn into_inner(self) -> (Vec<u8>, Layouts) {
        (self.code, self.layouts.build())
    }

    pub fn len(&self) -> usize {
        self.code.len()
    }

    pub fn disassemble(&self, end: usize) {
        if !cfg!(feature = "trace") {
            return;
        }

        Disassembler::new(&self.code, &self.layouts, &self.safepoints).disassemble(end);
    }

    pub fn start_function(&mut self) -> usize {
        self.op(FnStart);
        self.write_u16(0);
        self.code.len() - 3
    }

    pub fn update_frame_size(&mut self, function: usize, size: u16) {
        self.write_u16_at(function + 1, size);
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

    pub fn op_call_imm(&mut self, dest: Reg, callee: u32) {
        self.op(CallI);
        self.write_u16(dest);
        self.write_u32(callee);
    }

    pub fn op_jump_if_true(&mut self, cond: Reg) -> usize {
        self.op(JumpIfTrue);
        self.write_u16(cond);
        self.write_i32(0);
        self.code.len() - 4
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

    pub fn op_struct(&mut self, dest: Reg, first: Reg, count: u8, layout: Layout) {
        let layout = self.layouts.alloc(layout);
        self.op(Struct);
        self.write_u16(layout);
        self.write_u16(dest);
        self.write_u16(first);
        self.write_u8(count);
    }

    pub fn op_repeat_struct(&mut self, dest: Reg, value: Reg, count: Reg, layout: Layout) {
        let layout = self.layouts.alloc(layout);
        self.op(RepeatStruct);
        self.write_u16(layout);
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

    pub fn op_assert(&mut self, value: Reg) {
        self.op(Assert);
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

    pub fn op_binary_imm(&mut self, op: Opcode, dest: Reg, lhs: Reg, rhs_value: u64) {
        self.op(op);
        self.write_u16(dest);
        self.write_u16(lhs);
        self.write_u64(rhs_value);
    }

    pub fn write_u32_at(&mut self, i: usize, n: u32) {
        self.code[i..i + 4].copy_from_slice(&n.to_ne_bytes());
    }

    pub fn write_u64_at(&mut self, i: usize, n: u64) {
        self.code[i..i + 8].copy_from_slice(&n.to_ne_bytes());
    }

    pub fn write_string(&mut self, s: &str) {
        let len = s.len() as u32;
        self.code.extend_from_slice(&len.to_ne_bytes());
        self.code.extend_from_slice(s.as_bytes());
    }

    pub fn add_safepoint(&mut self, offset: usize, live: BitVec) {
        self.safepoints.push((offset, live));
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

    fn write_u32(&mut self, n: u32) {
        self.code.extend(n.to_ne_bytes());
    }

    fn write_u64(&mut self, n: u64) {
        self.code.extend(n.to_ne_bytes());
    }

    fn write_u16_at(&mut self, i: usize, n: u16) {
        self.code[i..i + 2].copy_from_slice(&n.to_ne_bytes());
    }

    fn write_i32_at(&mut self, i: usize, n: i32) {
        self.code[i..i + 4].copy_from_slice(&n.to_ne_bytes());
    }
}

struct Disassembler<'a> {
    code: &'a [u8],
    layouts: &'a LayoutBuilder,
    safepoints: &'a Vec<(usize, BitVec)>,
    i: usize,
}

impl<'a> Disassembler<'a> {
    fn new(
        code: &'a [u8],
        layouts: &'a LayoutBuilder,
        safepoints: &'a Vec<(usize, BitVec)>,
    ) -> Self {
        Self {
            code,
            layouts,
            safepoints,
            i: 0,
        }
    }

    pub fn disassemble(&mut self, end: usize) {
        println!("-- disassembly --");
        while self.i < end {
            let instr: Opcode = unsafe { std::mem::transmute(self.read_u8()) };
            print!("{:<05} {instr:?}", self.i - 1);
            match instr {
                FnStart => {
                    let frame_size = self.read_u16();
                    println!(" Frame size={frame_size}");
                }
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
                CallI => {
                    let dest = self.read_u16();
                    let callee = self.read_u32();
                    println!(" r{dest} = @{callee}");
                }
                JumpIfTrue | JumpIfFalse => {
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
                    let layout = self.read_u16();
                    let layout = self.layouts.of(layout);
                    let dest = self.read_u16();
                    let first = self.read_u16();
                    let count = self.read_u8();
                    println!(" r{dest} = r{first} x {count} ({layout:?})");
                }
                RepeatStruct => {
                    let layout = self.read_u16();
                    let layout = self.layouts.of(layout);
                    let dest = self.read_u16();
                    let start = self.read_u16();
                    let count = self.read_u16();
                    println!(" r{dest} = r{start} x r{count} ({layout:?})");
                }
                GetStructProperty => {
                    let dest = self.read_u16();
                    let obj = self.read_u16();
                    let index = self.read_u8();
                    println!(" r{dest} = r{obj}.{index}");
                }
                SetStructProperty => {
                    let obj = self.read_u16();
                    let index = self.read_u8();
                    let value = self.read_u16();
                    println!(" r{obj}.{index} = r{value}");
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
                PrintF64 | PrintI64 | PrintU64 | PrintString | PrintBool | PrintObj | Assert => {
                    let value = self.read_u16();
                    println!(" r{value}");
                }
                Copy | Not | F64Neg | I64Neg => {
                    let dest = self.read_u16();
                    let src = self.read_u16();
                    println!(" r{dest} = r{src}");
                }
                EqualI | NotEqualI | F64AddI | F64SubI | F64MulI | F64DivI | F64GreaterI
                | F64GreaterEqualI | F64LessI | F64LessEqualI | F64EqualI | F64NotEqualI
                | I64AddI | I64SubI | I64MulI | I64DivI | I64GreaterI | I64GreaterEqualI
                | I64LessI | I64LessEqualI | I64OrI | I64AndI | U64AddI | U64SubI | U64MulI
                | U64DivI | U64GreaterI | U64GreaterEqualI | U64LessI | U64LessEqualI | U64OrI
                | U64AndI => {
                    let dest = self.read_u16();
                    let lhs = self.read_u16();
                    let rhs = self.read_u64();
                    println!(" r{dest} = r{lhs}, {rhs}");
                }
                Equal | NotEqual | F64Add | F64Sub | F64Mul | F64Div | F64Greater
                | F64GreaterEqual | F64Less | F64LessEqual | F64Equal | F64NotEqual | I64Add
                | I64Sub | I64Mul | I64Div | I64Greater | I64GreaterEqual | I64Less
                | I64LessEqual | I64Or | I64And | U64Add | U64Sub | U64Mul | U64Div
                | U64Greater | U64GreaterEqual | U64Less | U64LessEqual | U64Or | U64And => {
                    let dest = self.read_u16();
                    let lhs = self.read_u16();
                    let rhs = self.read_u16();
                    println!(" r{dest} = r{lhs}, r{rhs}");
                }
            }
        }
        println!("----");

        println!("-- safepoints --");
        for (i, live) in self.safepoints {
            println!("{i} => {live}");
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

    fn read_u32(&mut self) -> u32 {
        u32::from_ne_bytes(self.read_array())
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
