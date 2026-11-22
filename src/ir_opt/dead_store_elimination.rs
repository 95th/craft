use bitvec::vec::BitVec;

use crate::{ir, ir_opt::IrOptimizationPass, liveness::LivenessAnalysis};

pub struct DeadStoreElimination;

impl IrOptimizationPass for DeadStoreElimination {
    fn run(&mut self, ir: &mut ir::Ir, function: ir::BasicBlockId) {
        loop {
            let liveness = LivenessAnalysis::new(ir).compute(function);
            let mut dead = Vec::new();

            let regs = ir.regs(function);
            for block in ir.reachable_postorder(function) {
                let mut tracker = LiveVariableTracker {
                    live: liveness.live_out[&block].clone(),
                };
                regs.visit_terminator(block, &mut tracker);
                for i in (0..ir.basic_blocks[block].statements.len()).rev() {
                    if cfg!(feature = "lower") {
                        println!("Live: {}", tracker.live);
                    }
                    let stmt = &ir.basic_blocks[block].statements[i];
                    if let ir::Statement::Assign { reg, value } = stmt {
                        if is_pure(value) && !tracker.live[*reg as usize] {
                            dead.push((block, i));
                        } else {
                            tracker.live.set(*reg as usize, false);
                            regs.visit_rvalue(value, &mut tracker);
                        }
                    } else {
                        regs.visit_stmt(stmt, &mut tracker);
                    }
                }
            }

            if cfg!(feature = "lower") {
                for &(block, i) in dead.iter() {
                    println!(
                        "Dead {block:?} => {:?}",
                        ir.basic_blocks[block].statements[i]
                    );
                }
            }

            if dead.is_empty() {
                break;
            }

            for (block, index) in dead {
                ir.basic_blocks[block].statements.remove(index);
            }
        }
    }
}

fn is_pure(value: &ir::RValue) -> bool {
    match value {
        ir::RValue::Constant(..) => true,
        ir::RValue::Use(_) => true,
        ir::RValue::Field { .. } => true,
        ir::RValue::Index { .. } => false,
        ir::RValue::Binary { op, .. } => !matches!(op, ir::BinaryOp::I64Div | ir::BinaryOp::U64Div),
        ir::RValue::Not(_) => true,
        ir::RValue::F64Neg(_) => true,
        ir::RValue::I64Neg(_) => true,
        ir::RValue::Struct { .. } => false,
        ir::RValue::StructRepeat { .. } => false,
        ir::RValue::Call { .. } => false,
    }
}

struct LiveVariableTracker {
    live: BitVec,
}

impl ir::RegVisitor for LiveVariableTracker {
    fn on_def(&mut self, _reg: ir::RegId) {
        //
    }

    fn on_use(&mut self, reg: ir::RegId) {
        self.live.set(reg as usize, true);
    }
}
