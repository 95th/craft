mod branch_forward;
mod dead_store_elimination;
mod simplify_branch;

use crate::{
    ir,
    ir_opt::{
        branch_forward::BranchForwarding, dead_store_elimination::DeadStoreElimination,
        simplify_branch::SimplifyBranches,
    },
};

pub trait IrOptimizationPass {
    fn run(&mut self, ir: &mut ir::Ir, function: ir::BasicBlockId) -> bool;
}

pub fn optimize(ir: &mut ir::Ir) {
    optimize_function(ir, ir.main);
    let functions: Vec<_> = ir.functions.values().copied().collect();
    for f in functions {
        optimize_function(ir, f);
    }
}

fn optimize_function(ir: &mut ir::Ir, function: ir::BasicBlockId) {
    loop {
        let mut changed = false;
        changed |= BranchForwarding::new().run(ir, function);
        changed |= SimplifyBranches.run(ir, function);
        changed |= DeadStoreElimination.run(ir, function);
        if !changed {
            break;
        }
    }
}
