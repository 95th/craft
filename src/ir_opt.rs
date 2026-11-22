mod branch_forward;
mod dead_store_elimination;

use crate::{
    ir,
    ir_opt::{branch_forward::BranchForwarding, dead_store_elimination::DeadStoreElimination},
};

pub trait IrOptimizationPass {
    fn run(&mut self, ir: &mut ir::Ir, function: ir::BasicBlockId);
}

pub fn optimize(ir: &mut ir::Ir) {
    run_pass(BranchForwarding, ir);
    run_pass(DeadStoreElimination, ir);
}

fn run_pass(mut pass: impl IrOptimizationPass, ir: &mut ir::Ir) {
    pass.run(ir, ir.main);
    let functions: Vec<_> = ir.functions.values().copied().collect();
    for f in functions {
        pass.run(ir, f);
    }
}
