use std::collections::VecDeque;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::{ir, ir_opt::IrOptimizationPass};

pub struct BranchForwarding {
    reachable: Vec<ir::BasicBlockId>,
    empty_blocks: FxHashMap<ir::BasicBlockId, ir::BasicBlockId>,
}

impl IrOptimizationPass for BranchForwarding {
    fn run(&mut self, ir: &mut ir::Ir, function: ir::BasicBlockId) {
        self.collect_reachable_blocks(ir, function);
        self.collect_empty_blocks(ir);
        self.optimize(ir);
    }
}

impl BranchForwarding {
    pub fn new() -> Self {
        Self {
            reachable: Vec::new(),
            empty_blocks: FxHashMap::default(),
        }
    }

    fn collect_reachable_blocks(&mut self, ir: &ir::Ir, entry: ir::BasicBlockId) {
        let mut seen = FxHashSet::default();
        let mut pending = VecDeque::new();
        pending.push_back(entry);

        while let Some(block) = pending.pop_front() {
            seen.insert(block);
            self.reachable.push(block);

            if ir.basic_blocks[block].statements.is_empty()
                && let ir::Terminator::Jump { target } = ir.basic_blocks[block].terminator
                && block != target
            {
                self.empty_blocks.insert(block, target);
            }
            ir.basic_blocks[block].for_each_successor(|succ| {
                if seen.insert(succ) {
                    pending.push_back(succ);
                }
            });
        }
    }

    fn collect_empty_blocks(&mut self, ir: &ir::Ir) {
        for &block in self.reachable.iter() {
            if ir.basic_blocks[block].statements.is_empty()
                && let ir::Terminator::Jump { target } = ir.basic_blocks[block].terminator
                && block != target
            {
                self.empty_blocks.insert(block, target);
            }
        }
    }

    fn optimize(&self, ir: &mut ir::Ir) {
        for &block in self.reachable.iter() {
            match &mut ir.basic_blocks[block].terminator {
                ir::Terminator::Jump { target } => {
                    self.forward_block(block, target);
                }
                ir::Terminator::Branch {
                    then_block,
                    else_block,
                    ..
                } => {
                    self.forward_block(block, then_block);
                    self.forward_block(block, else_block);
                }
                ir::Terminator::Return => {}
            }
        }
    }

    fn forward_block(&self, block: ir::BasicBlockId, target: &mut ir::BasicBlockId) {
        let mut seen = FxHashSet::default();
        let mut new_target = *target;
        seen.insert(block);
        seen.insert(*target);
        while let Some(&next) = self.empty_blocks.get(&new_target) {
            if !seen.insert(next) {
                break;
            }
            new_target = next;
        }
        *target = new_target;
    }
}
