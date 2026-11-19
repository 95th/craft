use std::collections::VecDeque;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    ast::BinaryOp,
    bind::SymbolId,
    chunk::{Code, Opcode::*},
    interner::{StrId, StrInterner},
    ir,
    typechecker::TyCtxt,
    types::{Ty, TyId},
};

pub struct Codegen2<'a> {
    ir: &'a ir::Ir,
    tcx: &'a mut TyCtxt,
    interner: &'a StrInterner,
    code: Code,
    current_block: ir::BasicBlockId,
    offsets: FxHashMap<ir::BasicBlockId, usize>,
    jump_worklist: Vec<(ir::BasicBlockId, usize)>,
    function_worklist: Vec<(SymbolId, usize)>,
    string_worklist: Vec<(StrId, usize)>,
}

impl<'a> Codegen2<'a> {
    pub fn new(ir: &'a ir::Ir, tcx: &'a mut TyCtxt, interner: &'a StrInterner) -> Self {
        Self {
            ir,
            tcx,
            interner,
            code: Code::new(),
            current_block: ir::BasicBlockId::MAX,
            offsets: FxHashMap::default(),
            jump_worklist: Vec::new(),
            function_worklist: Vec::new(),
            string_worklist: Vec::new(),
        }
    }

    pub fn generate(mut self) -> Code {
        self.gen_function(self.ir.main);

        let mut functions = FxHashMap::default();
        while let Some((function, patch_offset)) = self.function_worklist.pop() {
            let function = *functions.entry(function).or_insert_with(|| {
                let function_offset = self.code.len();
                self.gen_function(self.ir.functions[&function]);
                function_offset
            });
            self.code.write_u64_at(patch_offset, function as u64);
        }

        let code_end = self.code.len();

        let mut strings = FxHashMap::default();
        for (string, patch_offset) in self.string_worklist.drain(..) {
            let string_offset = *strings.entry(string).or_insert_with(|| {
                let string_offset = self.code.len();
                self.code.write_string(self.interner.lookup(string));
                self.code.write_u64_at(patch_offset, string_offset as u64);
                string_offset
            });
            self.code.write_u64_at(patch_offset, string_offset as u64);
        }

        self.code.disassemble(code_end);
        self.code
    }

    fn gen_function(&mut self, function: ir::BasicBlockId) {
        let blocks = self.collect_blocks(function);
        for block in blocks {
            self.gen_block(block);
        }

        for (block, patch_offset) in self.jump_worklist.drain(..) {
            let destination = self.offsets[&block];
            self.code.patch_jump_with(patch_offset, destination);
        }
    }

    fn gen_block(&mut self, block: ir::BasicBlockId) {
        self.offsets.insert(block, self.code.len());
        self.current_block = block;
        for stmt in self.ir.basic_blocks[block].statements.iter() {
            self.gen_stmt(stmt);
        }
        match self.ir.basic_blocks[block].terminator {
            Some(ir::Terminator::Jump { target }) => {
                let start = self.code.write_jump(Jump);
                self.jump_worklist.push((target, start));
            }
            Some(ir::Terminator::Branch {
                cond,
                then_block,
                else_block,
            }) => {
                self.gen_operand(cond);
                let false_jump = self.code.write_jump(JumpIfFalse);
                let true_jump = self.code.write_jump(Jump);
                self.jump_worklist.push((then_block, true_jump));
                self.jump_worklist.push((else_block, false_jump));
            }
            None => match self.ir.basic_blocks[block].successors[..] {
                [] => self.code.op_return(),
                [block] => {
                    let jump = self.code.write_jump(Jump);
                    self.jump_worklist.push((block, jump));
                }
                _ => unreachable!("Normal blocks must have zero or one successor."),
            },
        }
    }

    fn gen_stmt(&mut self, stmt: &ir::Statement) {
        match stmt {
            ir::Statement::Assign { place, value } => match place.projection {
                ir::Projection::Field(index) => {
                    self.code.op_get_local(place.reg);
                    self.gen_rvalue(value);
                    self.code.op_set_struct_property(index);
                }
                ir::Projection::Index(index) => {
                    self.code.op_get_local(place.reg);
                    self.code.op_get_local(index);
                    self.gen_rvalue(value);
                    self.code.op_set_struct_index();
                }
                ir::Projection::None => {
                    self.gen_rvalue(value);
                    self.code.op_set_local(place.reg);
                }
            },
            ir::Statement::StorageLive {
                implicit,
                reg: _reg,
            } => {
                if !implicit {
                    self.code.op_unit();
                }
            }
            ir::Statement::Print(operand, ty) => {
                self.gen_operand(*operand);
                match self.tcx.types.probe_value(*ty) {
                    Ty::F64 => self.code.op(PrintF64),
                    Ty::I64 => self.code.op(PrintI64),
                    Ty::U64 => self.code.op(PrintU64),
                    Ty::String => self.code.op(PrintString),
                    Ty::Bool => self.code.op(PrintBool),
                    _ => self.code.op(PrintObj),
                }
            }
        }
    }

    fn gen_rvalue(&mut self, value: &ir::RValue) {
        match *value {
            ir::RValue::Use(operand) => self.gen_operand(operand),
            ir::RValue::Binary { op, lhs, rhs, ty } => self.gen_binary(op, lhs, rhs, ty),
            ir::RValue::Not(operand) => {
                self.gen_operand(operand);
                self.code.op(Not);
            }
            ir::RValue::Neg(operand, ty) => {
                self.gen_operand(operand);
                match self.tcx.types.probe_value(ty) {
                    Ty::F64 => self.code.op(F64Neg),
                    Ty::I64 => self.code.op(I64Neg),
                    _ => unreachable!(),
                }
            }
            ir::RValue::Struct { ref fields } => {
                for f in fields {
                    self.gen_operand(*f);
                }
                self.code.op_struct(fields.len() as u16);
            }
            ir::RValue::StructRepeat { value, count } => {
                self.gen_operand(value);
                self.gen_operand(count);
                self.code.op_repeat_struct();
            }
            ir::RValue::Call { callee, ref args } => {
                self.code.op_unit();
                for arg in args {
                    self.gen_operand(*arg);
                }
                self.gen_operand(callee);
                self.code.op_call(args.len() as u8);
            }
        }
    }

    fn gen_binary(&mut self, op: BinaryOp, lhs: ir::Operand, rhs: ir::Operand, ty: TyId) {
        self.gen_operand(lhs);
        self.gen_operand(rhs);
        match self.tcx.types.probe_value(ty) {
            Ty::F64 => match op {
                BinaryOp::Add => self.code.op(F64Add),
                BinaryOp::Sub => self.code.op(F64Sub),
                BinaryOp::Mul => self.code.op(F64Mul),
                BinaryOp::Div => self.code.op(F64Div),
                BinaryOp::Greater => self.code.op(F64Greater),
                BinaryOp::GreaterEqual => self.code.op(F64GreaterEqual),
                BinaryOp::Less => self.code.op(F64Less),
                BinaryOp::LessEqual => self.code.op(F64LessEqual),
                BinaryOp::Equal => self.code.op(F64Equal),
                BinaryOp::NotEqual => self.code.op(F64NotEqual),
                _ => {}
            },
            Ty::I64 => match op {
                BinaryOp::Add => self.code.op(I64Add),
                BinaryOp::Sub => self.code.op(I64Sub),
                BinaryOp::Mul => self.code.op(I64Mul),
                BinaryOp::Div => self.code.op(I64Div),
                BinaryOp::Greater => self.code.op(I64Greater),
                BinaryOp::GreaterEqual => self.code.op(I64GreaterEqual),
                BinaryOp::Less => self.code.op(I64Less),
                BinaryOp::LessEqual => self.code.op(I64LessEqual),
                BinaryOp::Equal => self.code.op(Equal),
                BinaryOp::NotEqual => self.code.op(NotEqual),
                BinaryOp::BitOr => self.code.op(I64Or),
                BinaryOp::BitAnd => self.code.op(I64And),
                _ => {}
            },
            Ty::U64 => match op {
                BinaryOp::Add => self.code.op(U64Add),
                BinaryOp::Sub => self.code.op(U64Sub),
                BinaryOp::Mul => self.code.op(U64Mul),
                BinaryOp::Div => self.code.op(U64Div),
                BinaryOp::Greater => self.code.op(U64Greater),
                BinaryOp::GreaterEqual => self.code.op(U64GreaterEqual),
                BinaryOp::Less => self.code.op(U64Less),
                BinaryOp::LessEqual => self.code.op(U64LessEqual),
                BinaryOp::Equal => self.code.op(Equal),
                BinaryOp::NotEqual => self.code.op(NotEqual),
                BinaryOp::BitOr => self.code.op(U64Or),
                BinaryOp::BitAnd => self.code.op(U64And),
                _ => {}
            },
            _ => match op {
                BinaryOp::Equal => self.code.op(Equal),
                BinaryOp::NotEqual => self.code.op(NotEqual),
                _ => {}
            },
        }
    }

    fn gen_operand(&mut self, operand: ir::Operand) {
        match operand {
            ir::Operand::Place(place) => {
                self.code.op_get_local(place.reg);
                match place.projection {
                    ir::Projection::Field(index) => {
                        self.code.op_get_struct_property(index);
                    }
                    ir::Projection::Index(index) => {
                        self.code.op_get_local(index);
                        self.code.op_get_struct_index();
                    }
                    ir::Projection::None => {}
                }
            }
            ir::Operand::Constant(constant) => self.gen_constant(constant),
        }
    }

    fn gen_constant(&mut self, constant: ir::Constant) {
        match constant {
            ir::Constant::Unit => self.code.op_unit(),
            ir::Constant::Bool(x) => self.code.op(if x { True } else { False }),
            ir::Constant::I64(x) => {
                let x = u64::from_ne_bytes(x.to_ne_bytes());
                self.code.op_const(x);
            }
            ir::Constant::U64(x) => self.code.op_const(x),
            ir::Constant::F64(x) => {
                let x = x.to_bits();
                self.code.op_const(x);
            }
            ir::Constant::String(s) => {
                let offset = self.code.len() + 1;
                self.code.op_const(0);
                self.code.op(String);
                self.string_worklist.push((s, offset));
            }
            ir::Constant::Function(id) => {
                let offset = self.code.len() + 1;
                self.code.op_const(0);
                self.function_worklist.push((id, offset));
            }
        }
    }

    fn collect_blocks(&self, entry: ir::BasicBlockId) -> Vec<ir::BasicBlockId> {
        let mut list = Vec::new();
        let mut seen = FxHashSet::<ir::BasicBlockId>::default();
        let mut worklist = VecDeque::new();
        worklist.push_back(entry);

        while let Some(block) = worklist.pop_back() {
            list.push(block);
            for &successor in self.ir.basic_blocks[block].successors.iter() {
                if seen.insert(successor) {
                    worklist.push_back(successor);
                }
            }
        }

        list
    }
}
