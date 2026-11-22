use std::{cmp::Reverse, collections::BinaryHeap};

use rustc_hash::FxHashMap;

use crate::{
    bind::SymbolId,
    interner::{StrId, StrInterner},
    ir,
    liveness::LivenessAnalysis,
    register_chunk::{Code, Opcode::*, Reg},
    typechecker::TyCtxt,
    types::Ty,
};

pub struct Codegen2<'a> {
    ir: &'a ir::Ir,
    tcx: &'a mut TyCtxt,
    interner: &'a StrInterner,
    code: Code,
    jump_worklist: Vec<(ir::BasicBlockId, usize)>,
    function_worklist: Vec<(SymbolId, usize)>,
    string_worklist: Vec<(StrId, usize)>,
    reg_map: FxHashMap<u32, Reg>,
}

impl<'a> Codegen2<'a> {
    pub fn new(ir: &'a ir::Ir, tcx: &'a mut TyCtxt, interner: &'a StrInterner) -> Self {
        Self {
            ir,
            tcx,
            interner,
            code: Code::new(),
            jump_worklist: Vec::new(),
            function_worklist: Vec::new(),
            string_worklist: Vec::new(),
            reg_map: FxHashMap::default(),
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
        let max_regs = self.allocate_registers(function);
        self.code.start_function(max_regs);
        let mut offsets = FxHashMap::default();
        let mut blocks = self.ir.reachable_postorder(function);
        blocks.reverse();
        if cfg!(feature = "trace") {
            println!("Block order for {function:?}: {blocks:?}");
        }
        for i in 0..blocks.len() {
            offsets.insert(blocks[i], self.code.len());
            self.gen_block(blocks[i]);
            self.gen_terminator(blocks[i], blocks.get(i + 1).copied());
        }

        for (block, patch_offset) in self.jump_worklist.drain(..) {
            self.code.patch_jump(patch_offset, offsets[&block]);
        }
    }

    fn allocate_registers(&mut self, function: ir::BasicBlockId) -> u16 {
        let liveness = LivenessAnalysis::new(self.ir).compute(function);

        let mut intervals: Vec<_> = liveness.reg_interval.iter().copied().enumerate().collect();
        intervals.sort_by_key(|(_, i)| i.start);

        self.reg_map.clear();
        let mut active = FxHashMap::<Reg, usize>::default();
        let mut free = BinaryHeap::<Reverse<Reg>>::new();
        let mut next_reg = 0;
        let mut just_freed = Vec::new();

        for (reg_id, interval) in intervals {
            for (reg, end) in active.iter() {
                if *end < interval.start {
                    just_freed.push(*reg);
                }
            }

            while let Some(reg) = just_freed.pop() {
                active.remove(&reg);
                free.push(Reverse(reg));
            }

            if let Some(free) = free.pop() {
                self.reg_map.insert(reg_id as u32, free.0);
                active.insert(free.0, interval.end);
            } else {
                self.reg_map.insert(reg_id as u32, next_reg);
                active.insert(next_reg, interval.end);
                next_reg += 1;
            }
        }
        next_reg
    }

    fn gen_block(&mut self, block: ir::BasicBlockId) {
        for stmt in self.ir.basic_blocks[block].statements.iter() {
            self.gen_stmt(stmt);
        }
    }

    fn gen_terminator(&mut self, block: ir::BasicBlockId, next: Option<ir::BasicBlockId>) {
        match self.ir.basic_blocks[block].terminator {
            ir::Terminator::Jump { target } => {
                if next.is_none_or(|next| next != target) {
                    let start = self.code.op_jump();
                    self.jump_worklist.push((target, start));
                }
            }
            ir::Terminator::Branch {
                cond,
                then_block,
                else_block,
            } => {
                let cond = self.gen_reg(cond);
                if let Some(next) = next {
                    if next == then_block {
                        let false_jump = self.code.op_jump_if_false(cond);
                        self.jump_worklist.push((else_block, false_jump));
                        return;
                    }
                    if next == else_block {
                        let true_jump = self.code.op_jump_if_true(cond);
                        self.jump_worklist.push((then_block, true_jump));
                        return;
                    }
                }
                let false_jump = self.code.op_jump_if_false(cond);
                let true_jump = self.code.op_jump();
                self.jump_worklist.push((then_block, true_jump));
                self.jump_worklist.push((else_block, false_jump));
            }
            ir::Terminator::Return => self.code.op_return(),
        }
    }

    fn gen_stmt(&mut self, stmt: &ir::Statement) {
        match *stmt {
            ir::Statement::Assign { reg, ref value } => {
                let reg = self.gen_reg(reg);
                self.gen_rvalue(value, reg);
            }
            ir::Statement::SetField { reg, index, value } => {
                let reg = self.gen_reg(reg);
                let value = self.gen_reg(value);
                self.code.op_set_struct_property(reg, index as u8, value);
            }
            ir::Statement::SetIndex { reg, index, value } => {
                let reg = self.gen_reg(reg);
                let index = self.gen_reg(index);
                let value = self.gen_reg(value);
                self.code.op_set_struct_index(reg, index, value);
            }
            ir::Statement::ImplicitDef { .. } => {}
            ir::Statement::Print(reg, ty) => {
                let value = self.gen_reg(reg);
                match self.tcx.types.probe_value(ty) {
                    Ty::F64 => self.code.op_print_f64(value),
                    Ty::I64 => self.code.op_print_i64(value),
                    Ty::U64 => self.code.op_print_u64(value),
                    Ty::String => self.code.op_print_string(value),
                    Ty::Bool => self.code.op_print_bool(value),
                    _ => self.code.op_print_obj(value),
                }
            }
        }
    }

    fn gen_rvalue(&mut self, value: &ir::RValue, dest: Reg) {
        match *value {
            ir::RValue::Use(reg) => {
                let reg = self.gen_reg(reg);
                if reg != dest {
                    self.code.op_copy(dest, reg);
                }
            }
            ir::RValue::Constant(constant) => self.gen_constant(constant, dest),
            ir::RValue::Field { reg, index } => {
                let reg = self.gen_reg(reg);
                self.code.op_get_struct_property(dest, reg, index as u8);
            }
            ir::RValue::Index { reg, index } => {
                let reg = self.gen_reg(reg);
                let index = self.gen_reg(index);
                self.code.op_get_struct_index(dest, reg, index);
            }
            ir::RValue::Binary { op, lhs, rhs } => self.gen_binary(op, lhs, rhs, dest),
            ir::RValue::Not(reg) => {
                let reg = self.gen_reg(reg);
                self.code.op_not(dest, reg);
            }
            ir::RValue::F64Neg(reg) => {
                let reg = self.gen_reg(reg);
                self.code.op_neg_f64(dest, reg);
            }
            ir::RValue::I64Neg(reg) => {
                let reg = self.gen_reg(reg);
                self.code.op_neg_i64(dest, reg);
            }
            ir::RValue::Struct { ref fields } => {
                let args = fields.iter().map(|f| self.gen_reg(*f)).collect();
                self.code.op_struct(dest, args);
            }
            ir::RValue::StructRepeat { value, count } => {
                let value = self.gen_reg(value);
                let count = self.gen_reg(count);
                self.code.op_repeat_struct(dest, value, count);
            }
            ir::RValue::Call { callee, ref args } => {
                let callee = self.gen_reg(callee);
                let args = args.iter().map(|a| self.gen_reg(*a)).collect();
                self.code.op_call(dest, callee, args);
            }
        }
    }

    fn gen_binary(&mut self, op: ir::BinaryOp, lhs: ir::RegId, rhs: ir::RegId, dest: Reg) {
        let lhs = self.gen_reg(lhs);
        let rhs = self.gen_reg(rhs);
        match op {
            ir::BinaryOp::F64Add => self.code.op_binary(F64Add, dest, lhs, rhs),
            ir::BinaryOp::F64Sub => self.code.op_binary(F64Sub, dest, lhs, rhs),
            ir::BinaryOp::F64Mul => self.code.op_binary(F64Mul, dest, lhs, rhs),
            ir::BinaryOp::F64Div => self.code.op_binary(F64Div, dest, lhs, rhs),
            ir::BinaryOp::F64Greater => self.code.op_binary(F64Greater, dest, lhs, rhs),
            ir::BinaryOp::F64GreaterEqual => self.code.op_binary(F64GreaterEqual, dest, lhs, rhs),
            ir::BinaryOp::F64Less => self.code.op_binary(F64Less, dest, lhs, rhs),
            ir::BinaryOp::F64LessEqual => self.code.op_binary(F64LessEqual, dest, lhs, rhs),
            ir::BinaryOp::F64Equal => self.code.op_binary(F64Equal, dest, lhs, rhs),
            ir::BinaryOp::F64NotEqual => self.code.op_binary(F64NotEqual, dest, lhs, rhs),
            ir::BinaryOp::I64Add => self.code.op_binary(I64Add, dest, lhs, rhs),
            ir::BinaryOp::I64Sub => self.code.op_binary(I64Sub, dest, lhs, rhs),
            ir::BinaryOp::I64Mul => self.code.op_binary(I64Mul, dest, lhs, rhs),
            ir::BinaryOp::I64Div => self.code.op_binary(I64Div, dest, lhs, rhs),
            ir::BinaryOp::I64Greater => self.code.op_binary(I64Greater, dest, lhs, rhs),
            ir::BinaryOp::I64GreaterEqual => self.code.op_binary(I64GreaterEqual, dest, lhs, rhs),
            ir::BinaryOp::I64Less => self.code.op_binary(I64Less, dest, lhs, rhs),
            ir::BinaryOp::I64LessEqual => self.code.op_binary(I64LessEqual, dest, lhs, rhs),
            ir::BinaryOp::I64Or => self.code.op_binary(I64Or, dest, lhs, rhs),
            ir::BinaryOp::I64And => self.code.op_binary(I64And, dest, lhs, rhs),
            ir::BinaryOp::U64Add => self.code.op_binary(U64Add, dest, lhs, rhs),
            ir::BinaryOp::U64Sub => self.code.op_binary(U64Sub, dest, lhs, rhs),
            ir::BinaryOp::U64Mul => self.code.op_binary(U64Mul, dest, lhs, rhs),
            ir::BinaryOp::U64Div => self.code.op_binary(U64Div, dest, lhs, rhs),
            ir::BinaryOp::U64Greater => self.code.op_binary(U64Greater, dest, lhs, rhs),
            ir::BinaryOp::U64GreaterEqual => self.code.op_binary(U64GreaterEqual, dest, lhs, rhs),
            ir::BinaryOp::U64Less => self.code.op_binary(U64Less, dest, lhs, rhs),
            ir::BinaryOp::U64LessEqual => self.code.op_binary(U64LessEqual, dest, lhs, rhs),
            ir::BinaryOp::U64Or => self.code.op_binary(U64Or, dest, lhs, rhs),
            ir::BinaryOp::U64And => self.code.op_binary(U64And, dest, lhs, rhs),
            ir::BinaryOp::Equal => self.code.op_binary(Equal, dest, lhs, rhs),
            ir::BinaryOp::NotEqual => self.code.op_binary(NotEqual, dest, lhs, rhs),
        }
    }

    fn gen_constant(&mut self, constant: ir::Constant, dest: Reg) {
        match constant {
            ir::Constant::Unit => self.code.op_const(dest, 0),
            ir::Constant::Bool(x) => self.code.op_const(dest, if x { 1 } else { 0 }),
            ir::Constant::I64(x) => {
                let x = u64::from_ne_bytes(x.to_ne_bytes());
                self.code.op_const(dest, x);
            }
            ir::Constant::U64(x) => self.code.op_const(dest, x),
            ir::Constant::F64(x) => {
                let x = x.to_bits();
                self.code.op_const(dest, x);
            }
            ir::Constant::String(s) => {
                let offset = self.code.len() + 1 + std::mem::size_of::<Reg>();
                self.code.op_string(dest, 0);
                self.string_worklist.push((s, offset));
            }
            ir::Constant::Function(id) => {
                let offset = self.code.len() + 1 + std::mem::size_of::<Reg>();
                self.code.op_const(dest, 0);
                self.function_worklist.push((id, offset));
            }
        }
    }

    fn gen_reg(&mut self, reg_id: ir::RegId) -> Reg {
        self.reg_map[&reg_id]
    }
}
