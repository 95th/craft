use rustc_hash::{FxHashMap, FxHashSet};

use crate::{ir, ir_opt::IrOptimizationPass};

pub struct ConstPropagation {
    preds: FxHashMap<ir::BasicBlockId, FxHashSet<ir::BasicBlockId>>,
    outputs: FxHashMap<ir::BasicBlockId, FxHashMap<ir::RegId, ir::Constant>>,
    current: FxHashMap<ir::RegId, ir::Constant>,
    changed: bool,
}

impl IrOptimizationPass for ConstPropagation {
    fn run(&mut self, ir: &mut ir::Ir, function: ir::BasicBlockId) -> bool {
        let blocks = ir.reachable_reverse_postorder(function);
        for &block in blocks.iter() {
            self.collect_predecessors(ir, block);
        }

        loop {
            self.changed = false;
            for &block in blocks.iter() {
                self.collect_block_output(ir, block);
            }
            if !self.changed {
                break;
            }
        }

        for block in blocks {
            self.rewrite_block(ir, block);
        }
        self.changed
    }
}

impl ConstPropagation {
    pub fn new() -> Self {
        Self {
            preds: FxHashMap::default(),
            outputs: FxHashMap::default(),
            current: FxHashMap::default(),
            changed: false,
        }
    }

    fn collect_predecessors(&mut self, ir: &ir::Ir, block: ir::BasicBlockId) {
        match ir.basic_blocks[block].terminator {
            ir::Terminator::Jump { target } => {
                self.preds.entry(target).or_default().insert(block);
            }
            ir::Terminator::Branch {
                then_block,
                else_block,
                ..
            } => {
                self.preds.entry(then_block).or_default().insert(block);
                self.preds.entry(else_block).or_default().insert(block);
            }
            ir::Terminator::Return => {}
        }
    }

    fn collect_block_output(&mut self, ir: &ir::Ir, block: ir::BasicBlockId) {
        self.collect_block_input(block);
        for stmt in ir.basic_blocks[block].statements.iter() {
            let ir::Statement::Assign { reg, value } = stmt else {
                continue;
            };
            if let Some(value) = self.try_fold_rvalue(value) {
                self.current.insert(*reg, value);
            } else {
                self.current.remove(reg);
            }
        }
        if let Some(prev_output) = self.outputs.get(&block) {
            if *prev_output != self.current {
                self.changed = true;
            }
        } else {
            self.changed = true;
        }
        self.outputs
            .insert(block, std::mem::take(&mut self.current));
    }

    fn collect_block_input(&mut self, block: ir::BasicBlockId) {
        self.current.clear();
        if let Some(preds) = self.preds.get(&block) {
            let mut first = true;
            for pred in preds {
                let Some(output) = self.outputs.get(pred) else {
                    continue;
                };
                if first {
                    first = false;
                    self.current = output.clone();
                } else {
                    self.current
                        .retain(|k, v| output.get(k).is_some_and(|p_value| v == p_value));
                }
            }
        }
    }

    fn rewrite_block(&mut self, ir: &mut ir::Ir, block: ir::BasicBlockId) {
        self.collect_block_input(block);
        for stmt in ir.basic_blocks[block].statements.iter_mut() {
            self.rewrite_stmt(stmt);
        }
        self.rewrite_terminator(&mut ir.basic_blocks[block].terminator);
    }

    fn rewrite_stmt(&mut self, stmt: &mut ir::Statement) {
        match stmt {
            ir::Statement::Assign { reg, value } => {
                self.rewrite_rvalue(value);
                if let Some(constant) = self.try_fold_rvalue(value) {
                    *value = ir::RValue::Use(ir::Operand::Constant(constant));
                    self.current.insert(*reg, constant);
                }
            }
            ir::Statement::SetField { base, value, .. } => {
                self.rewrite_operand(base);
                self.rewrite_operand(value);
            }
            ir::Statement::SetIndex { base, index, value } => {
                self.rewrite_operand(base);
                self.rewrite_operand(index);
                self.rewrite_operand(value);
            }
            ir::Statement::Param { .. } => {}
            ir::Statement::Print(operand, ..) => self.rewrite_operand(operand),
            ir::Statement::Call { callee, .. } => self.rewrite_operand(callee),
            ir::Statement::Assert(operand) => self.rewrite_operand(operand),
        }
    }

    fn rewrite_terminator(&mut self, terminator: &mut ir::Terminator) {
        match terminator {
            ir::Terminator::Jump { .. } => {}
            ir::Terminator::Branch { cond, .. } => self.rewrite_operand(cond),
            ir::Terminator::Return => {}
        }
    }

    fn rewrite_rvalue(&mut self, value: &mut ir::RValue) {
        match value {
            ir::RValue::Use(operand) => self.rewrite_operand(operand),
            ir::RValue::Field { base, .. } => self.rewrite_operand(base),
            ir::RValue::Index { base, index } => {
                self.rewrite_operand(base);
                self.rewrite_operand(index);
            }
            ir::RValue::Binary { lhs, rhs, .. } => {
                self.rewrite_operand(lhs);
                self.rewrite_operand(rhs);
            }
            ir::RValue::Not(operand) => {
                self.rewrite_operand(operand);
            }
            ir::RValue::F64Neg(operand) => {
                self.rewrite_operand(operand);
            }
            ir::RValue::I64Neg(operand) => {
                self.rewrite_operand(operand);
            }
            ir::RValue::String(..) | ir::RValue::Struct { .. } | ir::RValue::Array { .. } => {}
            ir::RValue::ArrayRepeat { value, count, .. } => {
                self.rewrite_operand(value);
                self.rewrite_operand(count);
            }
        }
    }

    fn rewrite_operand(&mut self, operand: &mut ir::Operand) {
        match operand {
            ir::Operand::Constant(..) => {}
            ir::Operand::Reg(reg) => {
                if let Some(value) = self.current.get(reg) {
                    *operand = ir::Operand::Constant(*value);
                    self.changed = true;
                }
            }
        }
    }

    fn try_fold_rvalue(&self, value: &ir::RValue) -> Option<ir::Constant> {
        match *value {
            ir::RValue::Use(operand) => self.try_fold_operand(operand),
            ir::RValue::Binary { op, lhs, rhs } => {
                let lhs = self.try_fold_operand(lhs)?;
                let rhs = self.try_fold_operand(rhs)?;
                self.fold_binary(op, lhs, rhs)
            }
            ir::RValue::Not(operand) => {
                let value = self.try_fold_operand(operand)?;
                if let ir::Constant::Bool(x) = value {
                    Some(ir::Constant::Bool(!x))
                } else {
                    None
                }
            }
            ir::RValue::F64Neg(operand) => {
                let value = self.try_fold_operand(operand)?;
                if let ir::Constant::F64(x) = value {
                    Some(ir::Constant::F64(-x))
                } else {
                    None
                }
            }
            ir::RValue::I64Neg(operand) => {
                let value = self.try_fold_operand(operand)?;
                if let ir::Constant::I64(x) = value {
                    Some(ir::Constant::I64(x.wrapping_neg()))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn try_fold_operand(&self, operand: ir::Operand) -> Option<ir::Constant> {
        match operand {
            ir::Operand::Constant(constant) => Some(constant),
            ir::Operand::Reg(r) => self.current.get(&r).copied(),
        }
    }

    fn fold_binary(
        &self,
        op: ir::BinaryOp,
        lhs: ir::Constant,
        rhs: ir::Constant,
    ) -> Option<ir::Constant> {
        macro_rules! cast {
            ($x:expr, $type:ident) => {
                match $x {
                    ir::Constant::$type(x) => x,
                    _ => unreachable!(),
                }
            };
        }
        Some(match op {
            ir::BinaryOp::F64Add => ir::Constant::F64(cast!(lhs, F64) + cast!(rhs, F64)),
            ir::BinaryOp::F64Sub => ir::Constant::F64(cast!(lhs, F64) - cast!(rhs, F64)),
            ir::BinaryOp::F64Mul => ir::Constant::F64(cast!(lhs, F64) * cast!(rhs, F64)),
            ir::BinaryOp::F64Div => ir::Constant::F64(cast!(lhs, F64) / cast!(rhs, F64)),
            ir::BinaryOp::F64Greater => ir::Constant::Bool(cast!(lhs, F64) > cast!(rhs, F64)),
            ir::BinaryOp::F64GreaterEqual => ir::Constant::Bool(cast!(lhs, F64) >= cast!(rhs, F64)),
            ir::BinaryOp::F64Less => ir::Constant::Bool(cast!(lhs, F64) < cast!(rhs, F64)),
            ir::BinaryOp::F64LessEqual => ir::Constant::Bool(cast!(lhs, F64) <= cast!(rhs, F64)),
            ir::BinaryOp::F64Equal => ir::Constant::Bool(cast!(lhs, F64) == cast!(rhs, F64)),
            ir::BinaryOp::F64NotEqual => ir::Constant::Bool(cast!(lhs, F64) != cast!(rhs, F64)),
            ir::BinaryOp::I64Add => {
                ir::Constant::I64(cast!(lhs, I64).wrapping_add(cast!(rhs, I64)))
            }
            ir::BinaryOp::I64Sub => {
                ir::Constant::I64(cast!(lhs, I64).wrapping_sub(cast!(rhs, I64)))
            }
            ir::BinaryOp::I64Mul => {
                ir::Constant::I64(cast!(lhs, I64).wrapping_mul(cast!(rhs, I64)))
            }
            ir::BinaryOp::I64Div => {
                let rhs = cast!(rhs, I64);
                if rhs == 0 {
                    return None;
                }
                ir::Constant::I64(cast!(lhs, I64).wrapping_div(rhs))
            }
            ir::BinaryOp::I64Greater => ir::Constant::Bool(cast!(lhs, I64) > cast!(rhs, I64)),
            ir::BinaryOp::I64GreaterEqual => ir::Constant::Bool(cast!(lhs, I64) >= cast!(rhs, I64)),
            ir::BinaryOp::I64Less => ir::Constant::Bool(cast!(lhs, I64) < cast!(rhs, I64)),
            ir::BinaryOp::I64LessEqual => ir::Constant::Bool(cast!(lhs, I64) <= cast!(rhs, I64)),
            ir::BinaryOp::I64Or => ir::Constant::I64(cast!(lhs, I64) | cast!(rhs, I64)),
            ir::BinaryOp::I64And => ir::Constant::I64(cast!(lhs, I64) & cast!(rhs, I64)),
            ir::BinaryOp::U64Add => {
                ir::Constant::U64(cast!(lhs, U64).wrapping_add(cast!(rhs, U64)))
            }
            ir::BinaryOp::U64Sub => {
                ir::Constant::U64(cast!(lhs, U64).wrapping_sub(cast!(rhs, U64)))
            }
            ir::BinaryOp::U64Mul => {
                ir::Constant::U64(cast!(lhs, U64).wrapping_mul(cast!(rhs, U64)))
            }
            ir::BinaryOp::U64Div => {
                let rhs = cast!(rhs, U64);
                if rhs == 0 {
                    return None;
                }
                ir::Constant::U64(cast!(lhs, U64).wrapping_div(rhs))
            }
            ir::BinaryOp::U64Greater => ir::Constant::Bool(cast!(lhs, U64) > cast!(rhs, U64)),
            ir::BinaryOp::U64GreaterEqual => ir::Constant::Bool(cast!(lhs, U64) >= cast!(rhs, U64)),
            ir::BinaryOp::U64Less => ir::Constant::Bool(cast!(lhs, U64) < cast!(rhs, U64)),
            ir::BinaryOp::U64LessEqual => ir::Constant::Bool(cast!(lhs, U64) <= cast!(rhs, U64)),
            ir::BinaryOp::U64Or => ir::Constant::U64(cast!(lhs, U64) | cast!(rhs, U64)),
            ir::BinaryOp::U64And => ir::Constant::U64(cast!(lhs, U64) & cast!(rhs, U64)),
            ir::BinaryOp::Equal => ir::Constant::Bool(lhs == rhs),
            ir::BinaryOp::NotEqual => ir::Constant::Bool(lhs != rhs),
        })
    }
}
