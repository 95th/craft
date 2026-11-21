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

    pub fn compute(&mut self) -> Liveness {
        let mut out = Liveness {
            live_in: FxHashMap::default(),
            live_out: FxHashMap::default(),
            reg_interval: Vec::new(),
            block_interval: FxHashMap::default(),
        };
        self.check_function(self.ir.main, &mut out);
        for f in self.ir.functions.values() {
            self.check_function(*f, &mut out);
        }
        out
    }

    fn check_function(&mut self, function: ir::BasicBlockId, out: &mut Liveness) {
        let regs = self.ir.reg_count[&function] as usize;
        let blocks = self.ir.reachable_postorder(function);
        for block in blocks.iter() {
            self.compute_use_def(*block, regs);
            out.live_in.insert(*block, BitVec::new());
            out.live_out.insert(*block, BitVec::new());
        }

        loop {
            let mut changed = false;
            for &block in blocks.iter() {
                let mut live_out = BitVec::repeat(false, regs);
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
            for &block in blocks.iter() {
                println!("Live-in {block:?} => {}", out.live_in[&block]);
                println!("Live-out {block:?} => {}", out.live_out[&block]);
            }
        }

        let mut tracker = IntervalTracker {
            i: 0,
            blocks: FxHashMap::default(),
            regs: vec![Interval::UNINIT; regs],
        };

        for &block in blocks.iter().rev() {
            tracker.start_block(block);
            for stmt in self.ir.basic_blocks[block].statements.iter() {
                self.for_each_reg_in_stmt(stmt, &mut tracker);
                tracker.increment();
            }
            self.for_each_reg_in_terminator(block, &mut tracker);
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
    }

    fn compute_use_def(&mut self, block: ir::BasicBlockId, regs: usize) {
        let mut defuse = ComputeDefUse {
            uses: BitVec::repeat(false, regs),
            defs: BitVec::repeat(false, regs),
        };
        for stmt in self.ir.basic_blocks[block].statements.iter() {
            self.for_each_reg_in_stmt(stmt, &mut defuse);
        }
        self.for_each_reg_in_terminator(block, &mut defuse);

        self.uses.insert(block, defuse.uses);
        self.defs.insert(block, defuse.defs);
    }

    fn for_each_reg_in_stmt(&mut self, stmt: &ir::Statement, visitor: &mut impl RegVisitor) {
        match *stmt {
            ir::Statement::Assign { place, ref value } => {
                self.for_each_reg_in_rvalue(value, visitor);
                match place.projection {
                    ir::Projection::None => {
                        visitor.on_def(place.reg);
                    }
                    ir::Projection::Field(_) => {
                        visitor.on_use(place.reg);
                    }
                    ir::Projection::Index(index) => {
                        visitor.on_use(place.reg);
                        visitor.on_use(index);
                    }
                }
            }
            ir::Statement::StorageLive { .. } => {}
            ir::Statement::Print(operand, ..) => {
                self.for_each_reg_in_operand(operand, visitor);
            }
        }
    }

    fn for_each_reg_in_terminator(
        &mut self,
        block: ir::BasicBlockId,
        visitor: &mut impl RegVisitor,
    ) {
        match self.ir.basic_blocks[block].terminator {
            ir::Terminator::Jump { .. } => {}
            ir::Terminator::Branch { cond, .. } => self.for_each_reg_in_operand(cond, visitor),
            ir::Terminator::Return => {}
        }
    }

    fn for_each_reg_in_rvalue(&mut self, value: &ir::RValue, visitor: &mut impl RegVisitor) {
        match *value {
            ir::RValue::Use(operand) => self.for_each_reg_in_operand(operand, visitor),
            ir::RValue::Binary { lhs, rhs, .. } => {
                self.for_each_reg_in_operand(lhs, visitor);
                self.for_each_reg_in_operand(rhs, visitor);
            }
            ir::RValue::Not(operand) => self.for_each_reg_in_operand(operand, visitor),
            ir::RValue::F64Neg(operand) => self.for_each_reg_in_operand(operand, visitor),
            ir::RValue::I64Neg(operand) => self.for_each_reg_in_operand(operand, visitor),
            ir::RValue::Struct { ref fields } => {
                for value in fields.iter() {
                    self.for_each_reg_in_operand(*value, visitor);
                }
            }
            ir::RValue::StructRepeat { value, count } => {
                self.for_each_reg_in_operand(value, visitor);
                self.for_each_reg_in_operand(count, visitor);
            }
            ir::RValue::Call { callee, ref args } => {
                self.for_each_reg_in_operand(callee, visitor);
                for arg in args {
                    self.for_each_reg_in_operand(*arg, visitor);
                }
            }
        }
    }

    fn for_each_reg_in_operand(&mut self, operand: ir::Operand, visitor: &mut impl RegVisitor) {
        match operand {
            ir::Operand::Place(place) => {
                visitor.on_use(place.reg);
                match place.projection {
                    ir::Projection::None => {}
                    ir::Projection::Field(_) => {}
                    ir::Projection::Index(index) => visitor.on_use(index),
                }
            }
            ir::Operand::Constant(..) => {}
        }
    }
}

trait RegVisitor {
    fn on_def(&mut self, reg: RegId);
    fn on_use(&mut self, reg: RegId);
}

struct ComputeDefUse {
    uses: BitVec,
    defs: BitVec,
}

impl RegVisitor for ComputeDefUse {
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

impl RegVisitor for IntervalTracker {
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
