mod branch_forward;
mod const_propagation;
mod copy_propagation;
mod dead_store_elimination;
mod simplify_branch;

use crate::{
    ir,
    ir_opt::{
        branch_forward::BranchForwarding, const_propagation::ConstPropagation,
        copy_propagation::CopyPropagation, dead_store_elimination::DeadStoreElimination,
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
        changed |= ConstPropagation::new().run(ir, function);
        changed |= BranchForwarding::new().run(ir, function);
        changed |= SimplifyBranches.run(ir, function);
        changed |= CopyPropagation::new().run(ir, function);
        changed |= DeadStoreElimination.run(ir, function);

        if cfg!(feature = "trace") {
            println!("\nOptimization pass completed, changed = {changed}");
            for block in ir.reachable_reverse_postorder(function) {
                println!("Block: {block:?}");
                for stmt in ir.basic_blocks[block].statements.iter() {
                    println!("{stmt:?}")
                }
                println!("{:?}", ir.basic_blocks[block].terminator);
            }
        }
        if !changed {
            break;
        }
    }
}
