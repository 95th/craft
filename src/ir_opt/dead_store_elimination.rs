use bitvec::vec::BitVec;

use crate::{ir, ir_opt::IrOptimizationPass, liveness::LivenessAnalysis};

pub struct DeadStoreElimination;

impl IrOptimizationPass for DeadStoreElimination {
    fn run(&mut self, ir: &mut ir::Ir, function: ir::BasicBlockId) -> bool {
        let dead_stores = collect_dead_stores(ir, function);
        if cfg!(feature = "trace") {
            for &(block, i) in dead_stores.iter() {
                println!(
                    "Dead {block:?} => {:?}",
                    ir.basic_blocks[block].statements[i]
                );
            }
        }

        if dead_stores.is_empty() {
            return false;
        }
        for (block, index) in dead_stores {
            ir.basic_blocks[block].statements.remove(index);
        }
        true
    }
}

fn collect_dead_stores(ir: &ir::Ir, function: ir::BasicBlockId) -> Vec<(ir::BasicBlockId, usize)> {
    let liveness = LivenessAnalysis::new(ir).compute(function);
    let mut dead = Vec::new();
    let regs = ir.regs(function);
    for block in ir.reachable_postorder(function) {
        let mut tracker = LiveVariableTracker {
            live: liveness.live_out[&block].clone(),
        };
        regs.visit_terminator(block, &mut tracker);
        for i in (0..ir.basic_blocks[block].statements.len()).rev() {
            if cfg!(feature = "trace") {
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
    dead
}

fn is_pure(value: &ir::RValue) -> bool {
    match value {
        ir::RValue::Use(_) => true,
        ir::RValue::Field { .. } => true,
        ir::RValue::Index { .. } => true,
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
        // no op
    }

    fn on_use(&mut self, reg: ir::RegId) {
        self.live.set(reg as usize, true);
    }
}
