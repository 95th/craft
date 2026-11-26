use bitvec::vec::BitVec;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    bind::SymbolId,
    chunk::{Code, Opcode::*, Reg},
    heap::Layout,
    interner::{StrId, StrInterner},
    ir,
    liveness::{Interval, Liveness, LivenessAnalysis},
    typechecker::TyCtxt,
    types::Ty,
};

pub struct Codegen<'a> {
    ir: &'a ir::Ir,
    tcx: &'a mut TyCtxt,
    interner: &'a StrInterner,
    code: Code,
    jump_worklist: Vec<(ir::BasicBlockId, usize)>,
    function_worklist: Vec<(SymbolId, usize, bool)>,
    string_worklist: Vec<(StrId, usize)>,
    reg_map: FxHashMap<u32, Reg>,
    next_reg: Reg,
    next_scratch_reg: Reg,
    max_scratch_reg: Reg,
    stmt_idx: usize,
    safepoints: Vec<(usize, usize)>,
}

impl<'a> Codegen<'a> {
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
            next_reg: 0,
            next_scratch_reg: 0,
            max_scratch_reg: 0,
            stmt_idx: 0,
            safepoints: Vec::new(),
        }
    }

    pub fn generate(mut self) -> Code {
        if cfg!(feature = "trace") {
            println!("Running codegen");
        }
        self.gen_function(self.ir.main);

        let mut functions = FxHashMap::default();
        while let Some((function, patch_offset, immediate)) = self.function_worklist.pop() {
            let function = *functions.entry(function).or_insert_with(|| {
                let function_offset = self.code.len();
                self.gen_function(self.ir.functions[&function]);
                function_offset
            });
            if immediate {
                self.code.write_u32_at(patch_offset, function as u32);
            } else {
                self.code.write_u64_at(patch_offset, function as u64);
            }
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
        let liveness = LivenessAnalysis::new(self.ir).compute(function);
        self.allocate_registers(&liveness, function);
        self.stmt_idx = 0;
        self.safepoints.clear();
        let offset = self.code.start_function();
        let mut offsets = FxHashMap::default();
        let blocks = self.ir.reachable_reverse_postorder(function);
        if cfg!(feature = "trace") {
            println!("Block order for {function:?}: {blocks:?}");
        }
        for i in 0..blocks.len() {
            offsets.insert(blocks[i], self.code.len());
            self.gen_block(blocks[i], blocks.get(i + 1).copied());
        }

        for (block, patch_offset) in self.jump_worklist.drain(..) {
            self.code.patch_jump(patch_offset, offsets[&block]);
        }
        self.code.update_frame_size(offset, self.max_scratch_reg);
        self.gen_safepoints(&liveness, function);
    }

    fn allocate_registers(&mut self, liveness: &Liveness, function: ir::BasicBlockId) {
        let mut intervals: Vec<_> = liveness
            .reg_interval
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(reg, interval)| {
                if interval == Interval::UNINIT {
                    None
                } else {
                    Some((reg as ir::RegId, interval))
                }
            })
            .collect();
        intervals.sort_by_key(|(_, i)| i.start);

        let reg_groups = self.collect_reg_groups(function);

        self.reg_map.clear();
        let mut active_ir_regs = FxHashSet::default();
        let mut active = FxHashMap::<Reg, (ir::RegId, usize)>::default();
        let mut live = BitVec::<usize>::new();
        let mut just_freed = Vec::new();

        for (ir_reg, interval) in intervals {
            if active_ir_regs.contains(&ir_reg) {
                continue;
            }

            for (reg, (ir_reg, end)) in active.iter() {
                if *end <= interval.start {
                    just_freed.push(*reg);
                    active_ir_regs.remove(ir_reg);
                }
            }

            while let Some(reg) = just_freed.pop() {
                active.remove(&reg);
                live.set(reg as usize, false);
            }

            if let Some(&(ir_start, ir_end)) = reg_groups.get(&ir_reg) {
                let len = (ir_end - ir_start + 1) as usize;

                let start_reg = if let Some(highest_used) = live.iter_ones().rev().next() {
                    highest_used + 1
                } else {
                    live.len()
                };

                for j in 0..len {
                    let reg = start_reg + j;
                    let ir_reg = ir_start + j as u32;
                    if reg < live.len() {
                        live.set(reg, true);
                    } else {
                        live.push(true);
                    }
                    self.reg_map.insert(ir_reg, reg as Reg);
                    let interval = liveness.reg_interval[ir_reg as usize];
                    active.insert(reg as Reg, (ir_reg, interval.end));
                    active_ir_regs.insert(ir_reg);
                }
            } else {
                let reg = if let Some(reg) = live.iter_zeros().next() {
                    live.set(reg, true);
                    reg as Reg
                } else {
                    let reg = live.len();
                    live.push(true);
                    reg as Reg
                };
                self.reg_map.insert(ir_reg, reg);
                active.insert(reg, (ir_reg, interval.end));
                active_ir_regs.insert(ir_reg);
            }
        }
        self.next_reg = live.len() as Reg;
        self.next_scratch_reg = self.next_reg;
        self.max_scratch_reg = self.next_reg;
    }

    fn collect_reg_groups(
        &self,
        entry: ir::BasicBlockId,
    ) -> FxHashMap<ir::RegId, (ir::RegId, ir::RegId)> {
        let mut out = FxHashMap::default();
        for block in self.ir.reachable_postorder(entry) {
            for stmt in self.ir.basic_blocks[block].statements.iter() {
                match *stmt {
                    ir::Statement::Assign { ref value, .. } => match value {
                        ir::RValue::Struct { fields, .. } => {
                            if let Some(start) = fields.first().copied() {
                                let end = fields.last().copied().unwrap_or(start);
                                for reg in start..=end {
                                    out.insert(reg, (start, end));
                                }
                            }
                        }
                        _ => {}
                    },
                    ir::Statement::Call {
                        return_reg,
                        ref args,
                        ..
                    } => {
                        let start = return_reg;
                        let end = args.last().copied().unwrap_or(return_reg);
                        for reg in start..=end {
                            out.insert(reg, (start, end));
                        }
                    }
                    _ => {}
                }
            }
        }
        out
    }

    fn gen_block(&mut self, block: ir::BasicBlockId, next_block: Option<ir::BasicBlockId>) {
        for stmt in self.ir.basic_blocks[block].statements.iter() {
            self.reset_scratch_regs();
            self.gen_stmt(stmt);
            self.stmt_idx += 1;
        }
        self.reset_scratch_regs();
        self.gen_terminator(block, next_block);
        self.stmt_idx += 1;
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
                let cond = match cond {
                    ir::Operand::Reg(reg) => self.gen_reg(reg),
                    ir::Operand::Constant(..) => {
                        unreachable!("simplify branch opt pass should handle this");
                    }
                };
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
            ir::Statement::SetField { base, index, value } => {
                let reg = self.gen_operand(base);
                let value = self.gen_operand(value);
                self.code.op_set_struct_property(reg, index as u8, value);
            }
            ir::Statement::SetIndex { base, index, value } => {
                let reg = self.gen_operand(base);
                let index = self.gen_operand(index);
                let value = self.gen_operand(value);
                self.code.op_set_struct_index(reg, index, value);
            }
            ir::Statement::ImplicitDef { .. } => {}
            ir::Statement::Print(operand, ty) => {
                let value = self.gen_operand(operand);
                match self.tcx.types.probe_value(ty) {
                    Ty::F64 => self.code.op_print_f64(value),
                    Ty::I64 => self.code.op_print_i64(value),
                    Ty::U64 => self.code.op_print_u64(value),
                    Ty::String => self.code.op_print_string(value),
                    Ty::Bool => self.code.op_print_bool(value),
                    _ => self.code.op_print_obj(value),
                }
            }
            ir::Statement::Assert(operand) => {
                let value = self.gen_operand(operand);
                self.code.op_assert(value);
            }
            ir::Statement::Call {
                callee, return_reg, ..
            } => {
                self.record_safepoint();
                match callee {
                    ir::Operand::Constant(constant) => {
                        let ir::Constant::Function(id) = constant else {
                            unreachable!()
                        };
                        let return_reg = self.gen_reg(return_reg);
                        let offset = self.code.len() + 1 + std::mem::size_of::<Reg>();
                        self.code.op_call_imm(return_reg, 0);
                        self.function_worklist.push((id, offset, true));
                    }
                    ir::Operand::Reg(reg) => {
                        let callee = self.gen_reg(reg);
                        let return_reg = self.gen_reg(return_reg);
                        self.code.op_copy(return_reg, callee);
                        self.code.op_call(return_reg);
                    }
                }
            }
        }
    }

    fn gen_rvalue(&mut self, value: &ir::RValue, dest: Reg) {
        match *value {
            ir::RValue::Use(operand) => self.gen_operand_into(operand, dest),
            ir::RValue::Field { base, index } => {
                let reg = self.gen_operand(base);
                self.code.op_get_struct_property(dest, reg, index as u8);
            }
            ir::RValue::Index { base, index } => {
                let reg = self.gen_operand(base);
                let index = self.gen_operand(index);
                self.code.op_get_struct_index(dest, reg, index);
            }
            ir::RValue::Binary { op, lhs, rhs } => self.gen_binary(op, lhs, rhs, dest),
            ir::RValue::Not(operand) => {
                let reg = self.gen_operand(operand);
                self.code.op_not(dest, reg);
            }
            ir::RValue::F64Neg(operand) => {
                let reg = self.gen_operand(operand);
                self.code.op_neg_f64(dest, reg);
            }
            ir::RValue::I64Neg(operand) => {
                let reg = self.gen_operand(operand);
                self.code.op_neg_i64(dest, reg);
            }
            ir::RValue::Struct {
                ref fields,
                ref ref_map,
            } => {
                self.record_safepoint();
                let first = fields.first().map_or(dest, |f| self.gen_reg(*f));
                self.code.op_struct(
                    dest,
                    first,
                    fields.len() as u8,
                    Layout::Struct(ref_map.clone()),
                );
            }
            ir::RValue::Array {
                ref elements,
                is_ref,
            } => {
                self.record_safepoint();
                let first = elements.first().map_or(dest, |f| self.gen_reg(*f));
                self.code
                    .op_struct(dest, first, elements.len() as u8, Layout::Array(is_ref));
            }
            ir::RValue::ArrayRepeat {
                value,
                count,
                is_ref,
            } => {
                self.record_safepoint();
                let value = self.gen_operand(value);
                let count = self.gen_operand(count);
                self.code
                    .op_repeat_struct(dest, value, count, Layout::Array(is_ref));
            }
        }
    }

    fn gen_binary(&mut self, op: ir::BinaryOp, lhs: ir::Operand, rhs: ir::Operand, dest: Reg) {
        match rhs {
            ir::Operand::Constant(ir::Constant::U64(rhs)) => {
                self.gen_binary_u64(op, lhs, rhs, dest)
            }
            ir::Operand::Constant(ir::Constant::I64(rhs)) => {
                self.gen_binary_i64(op, lhs, rhs, dest)
            }
            ir::Operand::Constant(ir::Constant::F64(rhs)) => {
                self.gen_binary_f64(op, lhs, rhs, dest)
            }
            ir::Operand::Constant(ir::Constant::Bool(rhs)) => {
                self.gen_binary_other(op, lhs, rhs as u64, dest)
            }
            ir::Operand::Constant(ir::Constant::Unit) => self.gen_binary_other(op, lhs, 0, dest),
            _ => self.gen_binary_reg(op, lhs, rhs, dest),
        }
    }

    fn gen_binary_u64(&mut self, op: ir::BinaryOp, lhs: ir::Operand, rhs: u64, dest: Reg) {
        let lhs = self.gen_operand(lhs);
        match op {
            ir::BinaryOp::U64Add => self.code.op_binary_imm(U64AddI, dest, lhs, rhs),
            ir::BinaryOp::U64Sub => self.code.op_binary_imm(U64SubI, dest, lhs, rhs),
            ir::BinaryOp::U64Mul => self.code.op_binary_imm(U64MulI, dest, lhs, rhs),
            ir::BinaryOp::U64Div => self.code.op_binary_imm(U64DivI, dest, lhs, rhs),
            ir::BinaryOp::U64Greater => self.code.op_binary_imm(U64GreaterI, dest, lhs, rhs),
            ir::BinaryOp::U64GreaterEqual => {
                self.code.op_binary_imm(U64GreaterEqualI, dest, lhs, rhs)
            }
            ir::BinaryOp::U64Less => self.code.op_binary_imm(U64LessI, dest, lhs, rhs),
            ir::BinaryOp::U64LessEqual => self.code.op_binary_imm(U64LessEqualI, dest, lhs, rhs),
            ir::BinaryOp::U64Or => self.code.op_binary_imm(U64OrI, dest, lhs, rhs),
            ir::BinaryOp::U64And => self.code.op_binary_imm(U64AndI, dest, lhs, rhs),
            ir::BinaryOp::Equal => self.code.op_binary_imm(EqualI, dest, lhs, rhs),
            ir::BinaryOp::NotEqual => self.code.op_binary_imm(NotEqualI, dest, lhs, rhs),
            _ => unreachable!(),
        }
    }

    fn gen_binary_other(&mut self, op: ir::BinaryOp, lhs: ir::Operand, rhs: u64, dest: Reg) {
        let lhs = self.gen_operand(lhs);
        match op {
            ir::BinaryOp::Equal => self.code.op_binary_imm(EqualI, dest, lhs, rhs),
            ir::BinaryOp::NotEqual => self.code.op_binary_imm(NotEqualI, dest, lhs, rhs),
            _ => unreachable!(),
        }
    }

    fn gen_binary_i64(&mut self, op: ir::BinaryOp, lhs: ir::Operand, rhs: i64, dest: Reg) {
        let lhs = self.gen_operand(lhs);
        let rhs = u64::from_ne_bytes(rhs.to_ne_bytes());
        match op {
            ir::BinaryOp::I64Add => self.code.op_binary_imm(I64AddI, dest, lhs, rhs),
            ir::BinaryOp::I64Sub => self.code.op_binary_imm(I64SubI, dest, lhs, rhs),
            ir::BinaryOp::I64Mul => self.code.op_binary_imm(I64MulI, dest, lhs, rhs),
            ir::BinaryOp::I64Div => self.code.op_binary_imm(I64DivI, dest, lhs, rhs),
            ir::BinaryOp::I64Greater => self.code.op_binary_imm(I64GreaterI, dest, lhs, rhs),
            ir::BinaryOp::I64GreaterEqual => {
                self.code.op_binary_imm(I64GreaterEqualI, dest, lhs, rhs)
            }
            ir::BinaryOp::I64Less => self.code.op_binary_imm(I64LessI, dest, lhs, rhs),
            ir::BinaryOp::I64LessEqual => self.code.op_binary_imm(I64LessEqualI, dest, lhs, rhs),
            ir::BinaryOp::I64Or => self.code.op_binary_imm(I64OrI, dest, lhs, rhs),
            ir::BinaryOp::I64And => self.code.op_binary_imm(I64AndI, dest, lhs, rhs),
            ir::BinaryOp::Equal => self.code.op_binary_imm(EqualI, dest, lhs, rhs),
            ir::BinaryOp::NotEqual => self.code.op_binary_imm(NotEqualI, dest, lhs, rhs),
            _ => unreachable!(),
        }
    }

    fn gen_binary_f64(&mut self, op: ir::BinaryOp, lhs: ir::Operand, rhs: f64, dest: Reg) {
        let lhs = self.gen_operand(lhs);
        let rhs = rhs.to_bits();
        match op {
            ir::BinaryOp::F64Add => self.code.op_binary_imm(F64AddI, dest, lhs, rhs),
            ir::BinaryOp::F64Sub => self.code.op_binary_imm(F64SubI, dest, lhs, rhs),
            ir::BinaryOp::F64Mul => self.code.op_binary_imm(F64MulI, dest, lhs, rhs),
            ir::BinaryOp::F64Div => self.code.op_binary_imm(F64DivI, dest, lhs, rhs),
            ir::BinaryOp::F64Greater => self.code.op_binary_imm(F64GreaterI, dest, lhs, rhs),
            ir::BinaryOp::F64GreaterEqual => {
                self.code.op_binary_imm(F64GreaterEqualI, dest, lhs, rhs)
            }
            ir::BinaryOp::F64Less => self.code.op_binary_imm(F64LessI, dest, lhs, rhs),
            ir::BinaryOp::F64LessEqual => self.code.op_binary_imm(F64LessEqualI, dest, lhs, rhs),
            ir::BinaryOp::F64Equal => self.code.op_binary_imm(F64EqualI, dest, lhs, rhs),
            ir::BinaryOp::F64NotEqual => self.code.op_binary_imm(F64NotEqualI, dest, lhs, rhs),
            _ => unreachable!(),
        }
    }

    fn gen_binary_reg(&mut self, op: ir::BinaryOp, lhs: ir::Operand, rhs: ir::Operand, dest: Reg) {
        let lhs = self.gen_operand(lhs);
        let rhs = self.gen_operand(rhs);
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

    fn gen_operand_into(&mut self, operand: ir::Operand, dest: Reg) {
        match operand {
            ir::Operand::Constant(constant) => {
                self.gen_constant(constant, dest);
            }
            ir::Operand::Reg(reg) => {
                let reg = self.gen_reg(reg);
                if reg != dest {
                    self.code.op_copy(dest, reg);
                }
            }
        }
    }

    fn gen_operand(&mut self, operand: ir::Operand) -> Reg {
        match operand {
            ir::Operand::Constant(constant) => {
                let dest = self.new_scratch_reg();
                self.gen_constant(constant, dest);
                dest
            }
            ir::Operand::Reg(reg) => self.gen_reg(reg),
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
                self.record_safepoint();
                let offset = self.code.len() + 1 + std::mem::size_of::<Reg>();
                self.code.op_string(dest, 0);
                self.string_worklist.push((s, offset));
            }
            ir::Constant::Function(id) => {
                let offset = self.code.len() + 1 + std::mem::size_of::<Reg>();
                self.code.op_const(dest, 0);
                self.function_worklist.push((id, offset, false));
            }
        }
    }

    fn record_safepoint(&mut self) {
        self.safepoints.push((self.stmt_idx, self.code.len()))
    }

    fn gen_safepoints(&mut self, liveness: &Liveness, function: ir::BasicBlockId) {
        let ref_map = &self.ir.reg_maps[&function];
        for &(stmt_idx, offset) in self.safepoints.iter() {
            let mut live = BitVec::repeat(false, self.max_scratch_reg as usize);
            for (i, interval) in liveness.reg_interval.iter().enumerate() {
                if interval.start < stmt_idx && interval.end > stmt_idx {
                    if !ref_map[i] {
                        continue;
                    }
                    let reg = self.gen_reg(i as ir::RegId) as usize;
                    live.set(reg, true);
                }
            }
            self.code.add_safepoint(offset, live);
        }
    }

    fn gen_reg(&self, reg_id: ir::RegId) -> Reg {
        self.reg_map[&reg_id]
    }

    fn new_scratch_reg(&mut self) -> Reg {
        let reg = self.next_scratch_reg;
        self.next_scratch_reg += 1;
        reg
    }

    fn reset_scratch_regs(&mut self) {
        self.max_scratch_reg = self.max_scratch_reg.max(self.next_scratch_reg);
        self.next_scratch_reg = self.next_reg;
    }
}
