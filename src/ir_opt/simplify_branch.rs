use crate::{ir, ir_opt::IrOptimizationPass};

pub struct SimplifyBranches;

impl IrOptimizationPass for SimplifyBranches {
    fn run(&mut self, ir: &mut ir::Ir, function: ir::BasicBlockId) -> bool {
        let mut changed = false;
        for block in ir.reachable_postorder(function) {
            match ir.basic_blocks[block].terminator {
                ir::Terminator::Jump { .. } => {}
                ir::Terminator::Branch {
                    cond,
                    then_block,
                    else_block,
                } => {
                    if then_block == else_block {
                        ir.basic_blocks[block].terminator =
                            ir::Terminator::Jump { target: then_block };
                        changed = true;
                    } else {
                        match cond {
                            ir::Operand::Reg(_) => {}
                            ir::Operand::Constant(ir::Constant::Bool(true)) => {
                                ir.basic_blocks[block].terminator =
                                    ir::Terminator::Jump { target: then_block };
                                changed = true;
                            }
                            ir::Operand::Constant(ir::Constant::Bool(false)) => {
                                ir.basic_blocks[block].terminator =
                                    ir::Terminator::Jump { target: else_block };
                                changed = true;
                            }
                            ir::Operand::Constant(..) => unreachable!(),
                        }
                    }
                }
                ir::Terminator::Return => {}
            }
        }
        changed
    }
}
