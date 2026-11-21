use rustc_hash::{FxHashMap, FxHashSet};

use crate::{ir, ir_opt::IrOptimizationPass};

pub struct BranchForwarding;

impl IrOptimizationPass for BranchForwarding {
    fn run(&mut self, ir: &mut ir::Ir, function: ir::BasicBlockId) {
        let reachable = ir.reachable_postorder(function);
        let empty_blocks = collect_empty_blocks(ir, &reachable);
        optimize(ir, &reachable, &empty_blocks);
    }
}

fn collect_empty_blocks(
    ir: &ir::Ir,
    blocks: &[ir::BasicBlockId],
) -> FxHashMap<ir::BasicBlockId, ir::BasicBlockId> {
    let mut empty = FxHashMap::default();
    for &block in blocks {
        if ir.basic_blocks[block].statements.is_empty()
            && let ir::Terminator::Jump { target } = ir.basic_blocks[block].terminator
            && block != target
        {
            empty.insert(block, target);
        }
    }
    empty
}

fn optimize(
    ir: &mut ir::Ir,
    blocks: &[ir::BasicBlockId],
    empty_blocks: &FxHashMap<ir::BasicBlockId, ir::BasicBlockId>,
) {
    for &block in blocks.iter() {
        match &mut ir.basic_blocks[block].terminator {
            ir::Terminator::Jump { target } => {
                forward_block(block, target, empty_blocks);
            }
            ir::Terminator::Branch {
                then_block,
                else_block,
                ..
            } => {
                forward_block(block, then_block, empty_blocks);
                forward_block(block, else_block, empty_blocks);
            }
            ir::Terminator::Return => {}
        }
    }
}

fn forward_block(
    block: ir::BasicBlockId,
    target: &mut ir::BasicBlockId,
    empty_blocks: &FxHashMap<ir::BasicBlockId, ir::BasicBlockId>,
) {
    let mut seen = FxHashSet::default();
    let mut new_target = *target;
    seen.insert(block);
    seen.insert(*target);
    while let Some(&next) = empty_blocks.get(&new_target) {
        if !seen.insert(next) {
            break;
        }
        new_target = next;
    }
    *target = new_target;
}
