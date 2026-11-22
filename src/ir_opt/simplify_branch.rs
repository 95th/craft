use crate::{ir, ir_opt::IrOptimizationPass};

pub struct SimplifyBranches;

impl IrOptimizationPass for SimplifyBranches {
    fn run(&mut self, ir: &mut ir::Ir, function: ir::BasicBlockId) -> bool {
        let mut changed = false;
        for block in ir.reachable_postorder(function) {
            match ir.basic_blocks[block].terminator {
                ir::Terminator::Jump { .. } => {}
                ir::Terminator::Branch {
                    then_block,
                    else_block,
                    ..
                } => {
                    if then_block == else_block {
                        ir.basic_blocks[block].terminator =
                            ir::Terminator::Jump { target: then_block };
                        changed = true;
                    }
                }
                ir::Terminator::Return => {}
            }
        }
        changed
    }
}
