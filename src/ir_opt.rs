pub mod branch_forward;

use crate::{ir, ir_opt::branch_forward::BranchForwarding};

pub trait IrOptimizationPass {
    fn run(&mut self, ir: &mut ir::Ir, function: ir::BasicBlockId);
}

pub fn optimize(ir: &mut ir::Ir) {
    let mut pass = BranchForwarding;
    pass.run(ir, ir.main);
    let functions: Vec<_> = ir.functions.values().copied().collect();
    for f in functions {
        pass.run(ir, f);
    }
}
