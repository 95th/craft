use bitvec::prelude::*;
use rustc_hash::FxHashMap;

use crate::ir::{self, RegId};

pub struct Liveness {
    pub live_in: FxHashMap<ir::BasicBlockId, BitVec>,
    pub live_out: FxHashMap<ir::BasicBlockId, BitVec>,
    pub reg_interval: Vec<Interval>,
    pub block_interval: FxHashMap<ir::BasicBlockId, Interval>,
}

pub struct LivenessAnalysis<'a> {
    ir: &'a ir::Ir,
    uses: FxHashMap<ir::BasicBlockId, BitVec>,
    defs: FxHashMap<ir::BasicBlockId, BitVec>,
}

impl<'a> LivenessAnalysis<'a> {
    pub fn new(ir: &'a ir::Ir) -> Self {
        Self {
            ir,
            uses: FxHashMap::default(),
            defs: FxHashMap::default(),
        }
    }

    pub fn compute(&mut self, function: ir::BasicBlockId) -> Liveness {
        let mut out = Liveness {
            live_in: FxHashMap::default(),
            live_out: FxHashMap::default(),
            reg_interval: Vec::new(),
            block_interval: FxHashMap::default(),
        };

        let reg_count = self.ir.reg_count[&function] as usize;
        let blocks = self.ir.reachable_postorder(function);
        let regs = self.ir.regs(function);
        for block in blocks.iter() {
            self.compute_use_def(&regs, *block, reg_count);
            out.live_in.insert(*block, BitVec::new());
            out.live_out.insert(*block, BitVec::new());
        }

        let mut iterations = 0;
        loop {
            iterations += 1;
            let mut changed = false;
            for &block in blocks.iter() {
                let mut live_out = BitVec::repeat(false, reg_count);
                self.ir
                    .for_each_successor(block, |succ| live_out |= &out.live_in[&succ]);

                let live_in =
                    self.uses[&block].clone() | (live_out.clone() & !self.defs[&block].clone());

                if out.live_in[&block] != live_in || out.live_out[&block] != live_out {
                    out.live_in.insert(block, live_in);
                    out.live_out.insert(block, live_out);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        if cfg!(feature = "trace") {
            println!("Iterations: {iterations}");
            for &block in blocks.iter() {
                println!("Live-in {block:?} => {}", out.live_in[&block]);
                println!("Live-out {block:?} => {}", out.live_out[&block]);
            }
        }

        let mut tracker = IntervalTracker {
            i: 0,
            blocks: FxHashMap::default(),
            regs: vec![Interval::UNINIT; reg_count],
        };

        let regs = self.ir.regs(function);

        for &block in blocks.iter().rev() {
            tracker.start_block(block);
            for stmt in self.ir.basic_blocks[block].statements.iter() {
                regs.visit_stmt(stmt, &mut tracker);
                tracker.increment();
            }
            regs.visit_terminator(block, &mut tracker);
            tracker.end_block(block);
            tracker.increment();
        }

        for &block in blocks.iter().rev() {
            let block_interval = tracker.blocks[&block];
            for reg in out.live_in[&block].iter_ones() {
                debug_assert_ne!(tracker.regs[reg], Interval::UNINIT);
                if tracker.regs[reg].start > block_interval.start {
                    tracker.regs[reg].start = block_interval.start;
                }
            }

            for reg in out.live_out[&block].iter_ones() {
                debug_assert_ne!(tracker.regs[reg], Interval::UNINIT);
                if tracker.regs[reg].end < block_interval.end {
                    tracker.regs[reg].end = block_interval.end;
                }
            }
        }

        out.reg_interval = tracker.regs;
        out.block_interval = tracker.blocks;

        if cfg!(feature = "trace") {
            for (reg, interval) in out.reg_interval.iter().enumerate() {
                println!("Reg {reg} => {interval:?}");
            }
        }

        out
    }

    fn compute_use_def(&mut self, regs: &ir::Regs, block: ir::BasicBlockId, reg_count: usize) {
        let mut defuse = ComputeDefUse {
            uses: BitVec::repeat(false, reg_count),
            defs: BitVec::repeat(false, reg_count),
        };
        for stmt in self.ir.basic_blocks[block].statements.iter() {
            regs.visit_stmt(stmt, &mut defuse);
        }
        regs.visit_terminator(block, &mut defuse);

        self.uses.insert(block, defuse.uses);
        self.defs.insert(block, defuse.defs);
    }
}

struct ComputeDefUse {
    uses: BitVec,
    defs: BitVec,
}

impl ir::RegVisitor for ComputeDefUse {
    fn on_def(&mut self, reg: RegId) {
        self.defs.set(reg as usize, true);
    }

    fn on_use(&mut self, reg: RegId) {
        if !self.defs[reg as usize] {
            self.uses.set(reg as usize, true);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interval {
    pub start: usize,
    pub end: usize,
}

impl Interval {
    const UNINIT: Self = Self {
        start: usize::MAX,
        end: usize::MAX,
    };
}

struct IntervalTracker {
    i: usize,
    regs: Vec<Interval>,
    blocks: FxHashMap<ir::BasicBlockId, Interval>,
}

impl ir::RegVisitor for IntervalTracker {
    fn on_def(&mut self, reg: RegId) {
        self.update_reg(reg);
    }

    fn on_use(&mut self, reg: RegId) {
        self.update_reg(reg);
    }
}

impl IntervalTracker {
    fn increment(&mut self) {
        self.i += 1;
    }

    fn start_block(&mut self, block: ir::BasicBlockId) {
        self.blocks.insert(
            block,
            Interval {
                start: self.i,
                end: self.i,
            },
        );
    }

    fn end_block(&mut self, block: ir::BasicBlockId) {
        self.blocks.get_mut(&block).unwrap().end = self.i;
    }

    fn update_reg(&mut self, reg: RegId) {
        if self.regs[reg as usize].start == usize::MAX {
            self.regs[reg as usize].start = self.i;
        }
        self.regs[reg as usize].end = self.i;
    }
}
