use rustc_hash::FxHashMap;

use crate::{
    ir::{self, RValue},
    ir_opt::IrOptimizationPass,
};

pub struct CopyPropagation {
    map: FxHashMap<ir::RegId, ir::RegId>,
    changed: bool,
}

impl IrOptimizationPass for CopyPropagation {
    fn run(&mut self, ir: &mut ir::Ir, function: ir::BasicBlockId) -> bool {
        for block in ir.reachable_postorder(function) {
            self.map.clear();
            for stmt in ir.basic_blocks[block].statements.iter_mut() {
                self.rewrite_stmt(stmt);
            }
        }
        self.changed
    }
}

impl CopyPropagation {
    pub fn new() -> Self {
        Self {
            map: FxHashMap::default(),
            changed: false,
        }
    }

    fn rewrite_stmt(&mut self, stmt: &mut ir::Statement) {
        match stmt {
            ir::Statement::Assign { reg, value } => {
                self.rewrite_rvalue(value);

                self.map.remove(reg);
                self.map.retain(|_, v| v != reg);

                if let ir::RValue::Use(ir::Operand::Reg(src)) = value
                    && src != reg
                {
                    self.map.insert(*reg, *src);
                }
            }
            ir::Statement::Call { callee, .. } => {
                self.rewrite_operand(callee);
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
            ir::Statement::ImplicitDef { .. } => {}
            ir::Statement::Print(operand, ..) => self.rewrite_operand(operand),
            ir::Statement::Assert(operand) => self.rewrite_operand(operand),
        }
    }

    fn rewrite_rvalue(&mut self, value: &mut ir::RValue) {
        match value {
            RValue::Use(operand) => self.rewrite_operand(operand),
            RValue::Field { base, .. } => {
                self.rewrite_operand(base);
            }
            RValue::Index { base, index } => {
                self.rewrite_operand(base);
                self.rewrite_operand(index);
            }
            RValue::Binary { lhs, rhs, .. } => {
                self.rewrite_operand(lhs);
                self.rewrite_operand(rhs);
            }
            RValue::Not(operand) => self.rewrite_operand(operand),
            RValue::F64Neg(operand) => self.rewrite_operand(operand),
            RValue::I64Neg(operand) => self.rewrite_operand(operand),
            RValue::Struct { .. } | RValue::Array { .. } => {}
            RValue::ArrayRepeat { value, count, .. } => {
                self.rewrite_operand(value);
                self.rewrite_operand(count);
            }
        }
    }

    fn rewrite_operand(&mut self, operand: &mut ir::Operand) {
        match operand {
            ir::Operand::Constant(..) => {}
            ir::Operand::Reg(reg) => {
                let mut new_reg = *reg;
                while let Some(&x) = self.map.get(&new_reg) {
                    new_reg = x;
                }
                if new_reg != *reg {
                    *reg = new_reg;
                    self.changed = true;
                }
            }
        }
    }
}
