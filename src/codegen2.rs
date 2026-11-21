use std::collections::VecDeque;

use rustc_hash::FxHashMap;

use crate::{
    bind::SymbolId,
    interner::{StrId, StrInterner},
    ir,
    register_chunk::{Code, Opcode::*, Reg},
    typechecker::TyCtxt,
    types::Ty,
};

pub struct Codegen2<'a> {
    ir: &'a ir::Ir,
    tcx: &'a mut TyCtxt,
    interner: &'a StrInterner,
    code: Code,
    jump_worklist: Vec<(ir::BasicBlockId, usize)>,
    function_worklist: Vec<(SymbolId, usize)>,
    string_worklist: Vec<(StrId, usize)>,
    reg_map: FxHashMap<u32, Reg>,
    next_reg: Reg,
}

impl<'a> Codegen2<'a> {
    pub fn new(ir: &'a ir::Ir, tcx: &'a mut TyCtxt, interner: &'a StrInterner) -> Self {
        Self {
            ir,
            tcx,
            interner,
            code: Code::new(),
            jump_worklist: Vec::new(),
            function_worklist: Vec::new(),
            string_worklist: Vec::new(),
            reg_map: FxHashMap::default(),
            next_reg: 0,
        }
    }

    pub fn generate(mut self) -> Code {
        self.gen_function(self.ir.main);

        let mut functions = FxHashMap::default();
        while let Some((function, patch_offset)) = self.function_worklist.pop() {
            let function = *functions.entry(function).or_insert_with(|| {
                let function_offset = self.code.len();
                self.gen_function(self.ir.functions[&function]);
                function_offset
            });
            self.code.write_u64_at(patch_offset, function as u64);
        }

        let code_end = self.code.len();

        let mut strings = FxHashMap::default();
        for (string, patch_offset) in self.string_worklist.drain(..) {
            let string_offset = *strings.entry(string).or_insert_with(|| {
                let string_offset = self.code.len();
                self.code.write_string(self.interner.lookup(string));
                self.code.write_u64_at(patch_offset, string_offset as u64);
                string_offset
            });
            self.code.write_u64_at(patch_offset, string_offset as u64);
        }

        self.code.disassemble(code_end);
        self.code
    }

    fn gen_function(&mut self, function: ir::BasicBlockId) {
        self.reg_map.clear();
        self.next_reg = 0;

        let mut done = FxHashMap::default();
        let mut pending = VecDeque::new();
        pending.push_back(function);
        while let Some(block) = pending.pop_front() {
            done.insert(block, self.code.len());
            self.gen_block(block);
            for &succ in self.ir.basic_blocks[block].successors.iter() {
                if !done.contains_key(&succ) {
                    pending.push_back(succ);
                }
            }
        }

        for (block, patch_offset) in self.jump_worklist.drain(..) {
            self.code.patch_jump(patch_offset, done[&block]);
        }
    }

    fn gen_block(&mut self, block: ir::BasicBlockId) {
        for stmt in self.ir.basic_blocks[block].statements.iter() {
            self.gen_stmt(stmt);
        }
        match self.ir.basic_blocks[block].terminator {
            Some(ir::Terminator::Jump { target }) => {
                let start = self.code.op_jump();
                self.jump_worklist.push((target, start));
            }
            Some(ir::Terminator::Branch {
                cond,
                then_block,
                else_block,
            }) => {
                let cond = self.gen_operand(cond, None);
                let false_jump = self.code.op_jump_if_false(cond);
                let true_jump = self.code.op_jump();
                self.jump_worklist.push((then_block, true_jump));
                self.jump_worklist.push((else_block, false_jump));
            }
            None => match self.ir.basic_blocks[block].successors[..] {
                [] => self.code.op_return(),
                [block] => {
                    let jump = self.code.op_jump();
                    self.jump_worklist.push((block, jump));
                }
                _ => unreachable!("Normal blocks must have zero or one successor."),
            },
        }
    }

    fn gen_stmt(&mut self, stmt: &ir::Statement) {
        match stmt {
            ir::Statement::Assign { place, value } => match place.projection {
                ir::Projection::Field(index) => {
                    let obj = self.gen_reg(place.reg);
                    let value = self.gen_rvalue(value, None);
                    self.code.op_set_struct_property(obj, index as u8, value);
                }
                ir::Projection::Index(index) => {
                    let obj = self.gen_reg(place.reg);
                    let index = self.gen_reg(index);
                    let value = self.gen_rvalue(value, None);
                    self.code.op_set_struct_index(obj, index, value);
                }
                ir::Projection::None => {
                    let reg = self.gen_reg(place.reg);
                    self.gen_rvalue(value, Some(reg));
                }
            },
            ir::Statement::StorageLive { implicit, reg } => {
                if *implicit {
                    self.gen_reg(*reg);
                }
            }
            ir::Statement::Print(operand, ty) => {
                let value = self.gen_operand(*operand, None);
                match self.tcx.types.probe_value(*ty) {
                    Ty::F64 => self.code.op_print_f64(value),
                    Ty::I64 => self.code.op_print_i64(value),
                    Ty::U64 => self.code.op_print_u64(value),
                    Ty::String => self.code.op_print_string(value),
                    Ty::Bool => self.code.op_print_bool(value),
                    _ => self.code.op_print_obj(value),
                }
            }
        }
    }

    fn gen_rvalue(&mut self, value: &ir::RValue, dest: Option<Reg>) -> Reg {
        match *value {
            ir::RValue::Use(operand) => return self.gen_operand(operand, dest),
            ir::RValue::Binary { op, lhs, rhs } => return self.gen_binary(op, lhs, rhs, dest),
            ir::RValue::Not(operand) => {
                let dest = self.gen_operand(operand, dest);
                self.code.op_not(dest, dest);
                dest
            }
            ir::RValue::F64Neg(operand) => {
                let dest = self.gen_operand(operand, dest);
                self.code.op_neg_f64(dest, dest);
                dest
            }
            ir::RValue::I64Neg(operand) => {
                let dest = self.gen_operand(operand, dest);
                self.code.op_neg_i64(dest, dest);
                dest
            }
            ir::RValue::Struct { ref fields } => {
                let regs: Vec<_> = (0..fields.len()).map(|_| self.temp()).collect();
                for i in 0..fields.len() {
                    self.gen_operand(fields[i], Some(regs[i]));
                }
                if fields.is_empty() {
                    let dest = self.dest_or_temp(dest);
                    self.code.op_struct(dest, 0, 0);
                    return dest;
                }
                let dest = dest.unwrap_or(regs[0]);
                self.code.op_struct(dest, regs[0], fields.len() as u8);
                self.next_reg = regs[0] + 1;
                dest
            }
            ir::RValue::StructRepeat { value, count } => {
                let value = self.gen_operand(value, None);
                let count = self.gen_operand(count, None);
                let dest = self.dest_or_temp(dest);
                self.code.op_repeat_struct(dest, value, count);
                dest
            }
            ir::RValue::Call { callee, ref args } => {
                let first = match dest {
                    Some(dest) if dest + 1 == self.next_reg => dest,
                    _ => self.temp(),
                };
                for _ in 0..args.len() {
                    self.temp();
                }
                self.gen_operand(callee, Some(first));
                for i in 0..args.len() {
                    self.gen_operand(args[i], Some(first + i as Reg + 1));
                }
                self.code.op_call(first);
                self.next_reg = first + 1;
                match dest {
                    Some(dest) if dest == first => dest,
                    Some(dest) => {
                        self.code.op_copy(dest, first);
                        dest
                    }
                    None => first,
                }
            }
        }
    }

    fn gen_binary(
        &mut self,
        op: ir::BinaryOp,
        lhs: ir::Operand,
        rhs: ir::Operand,
        dest: Option<Reg>,
    ) -> Reg {
        let dest = self.dest_or_temp(dest);
        let lhs = self.gen_operand(lhs, None);
        let rhs = self.gen_operand(rhs, None);
        match op {
            ir::BinaryOp::F64Add => self.code.op_binary(F64Add, dest, lhs, rhs),
            ir::BinaryOp::F64Sub => self.code.op_binary(F64Sub, dest, lhs, rhs),
            ir::BinaryOp::F64Mul => self.code.op_binary(F64Mul, dest, lhs, rhs),
            ir::BinaryOp::F64Div => self.code.op_binary(F64Div, dest, lhs, rhs),
            ir::BinaryOp::F64Greater => self.code.op_binary(F64Greater, dest, lhs, rhs),
            ir::BinaryOp::F64GreaterEqual => self.code.op_binary(F64GreaterEqual, dest, lhs, rhs),
            ir::BinaryOp::F64Less => self.code.op_binary(F64Less, dest, lhs, rhs),
            ir::BinaryOp::F64LessEqual => self.code.op_binary(F64LessEqual, dest, lhs, rhs),
            ir::BinaryOp::F64Equal => self.code.op_binary(F64Equal, dest, lhs, rhs),
            ir::BinaryOp::F64NotEqual => self.code.op_binary(F64NotEqual, dest, lhs, rhs),
            ir::BinaryOp::I64Add => self.code.op_binary(I64Add, dest, lhs, rhs),
            ir::BinaryOp::I64Sub => self.code.op_binary(I64Sub, dest, lhs, rhs),
            ir::BinaryOp::I64Mul => self.code.op_binary(I64Mul, dest, lhs, rhs),
            ir::BinaryOp::I64Div => self.code.op_binary(I64Div, dest, lhs, rhs),
            ir::BinaryOp::I64Greater => self.code.op_binary(I64Greater, dest, lhs, rhs),
            ir::BinaryOp::I64GreaterEqual => self.code.op_binary(I64GreaterEqual, dest, lhs, rhs),
            ir::BinaryOp::I64Less => self.code.op_binary(I64Less, dest, lhs, rhs),
            ir::BinaryOp::I64LessEqual => self.code.op_binary(I64LessEqual, dest, lhs, rhs),
            ir::BinaryOp::I64Or => self.code.op_binary(I64Or, dest, lhs, rhs),
            ir::BinaryOp::I64And => self.code.op_binary(I64And, dest, lhs, rhs),
            ir::BinaryOp::U64Add => self.code.op_binary(U64Add, dest, lhs, rhs),
            ir::BinaryOp::U64Sub => self.code.op_binary(U64Sub, dest, lhs, rhs),
            ir::BinaryOp::U64Mul => self.code.op_binary(U64Mul, dest, lhs, rhs),
            ir::BinaryOp::U64Div => self.code.op_binary(U64Div, dest, lhs, rhs),
            ir::BinaryOp::U64Greater => self.code.op_binary(U64Greater, dest, lhs, rhs),
            ir::BinaryOp::U64GreaterEqual => self.code.op_binary(U64GreaterEqual, dest, lhs, rhs),
            ir::BinaryOp::U64Less => self.code.op_binary(U64Less, dest, lhs, rhs),
            ir::BinaryOp::U64LessEqual => self.code.op_binary(U64LessEqual, dest, lhs, rhs),
            ir::BinaryOp::U64Or => self.code.op_binary(U64Or, dest, lhs, rhs),
            ir::BinaryOp::U64And => self.code.op_binary(U64And, dest, lhs, rhs),
            ir::BinaryOp::Equal => self.code.op_binary(Equal, dest, lhs, rhs),
            ir::BinaryOp::NotEqual => self.code.op_binary(NotEqual, dest, lhs, rhs),
        }
        dest
    }

    fn gen_operand(&mut self, operand: ir::Operand, dest: Option<Reg>) -> Reg {
        match operand {
            ir::Operand::Place(place) => match place.projection {
                ir::Projection::Field(index) => {
                    let obj = self.gen_reg(place.reg);
                    let dest = self.dest_or_temp(dest);
                    self.code.op_get_struct_property(dest, obj, index as u8);
                    dest
                }
                ir::Projection::Index(index) => {
                    let obj = self.gen_reg(place.reg);
                    let index = self.gen_reg(index);
                    let dest = self.dest_or_temp(dest);
                    self.code.op_get_struct_index(dest, obj, index);
                    dest
                }
                ir::Projection::None => {
                    let reg = self.gen_reg(place.reg);
                    if let Some(dest) = dest
                        && dest != reg
                    {
                        self.code.op_copy(dest, reg);
                        dest
                    } else {
                        reg
                    }
                }
            },
            ir::Operand::Constant(constant) => self.gen_constant(constant, dest),
        }
    }

    fn gen_constant(&mut self, constant: ir::Constant, dest: Option<Reg>) -> Reg {
        let dest = self.dest_or_temp(dest);
        match constant {
            ir::Constant::Unit => self.code.op_const(dest, 0),
            ir::Constant::Bool(x) => self.code.op_const(dest, if x { 1 } else { 0 }),
            ir::Constant::I64(x) => {
                let x = u64::from_ne_bytes(x.to_ne_bytes());
                self.code.op_const(dest, x);
            }
            ir::Constant::U64(x) => self.code.op_const(dest, x),
            ir::Constant::F64(x) => {
                let x = x.to_bits();
                self.code.op_const(dest, x);
            }
            ir::Constant::String(s) => {
                let offset = self.code.len() + 1 + std::mem::size_of::<Reg>();
                self.code.op_string(dest, 0);
                self.string_worklist.push((s, offset));
            }
            ir::Constant::Function(id) => {
                let offset = self.code.len() + 1 + std::mem::size_of::<Reg>();
                self.code.op_const(dest, 0);
                self.function_worklist.push((id, offset));
            }
        }
        dest
    }

    fn gen_reg(&mut self, reg_id: ir::RegId) -> Reg {
        if let Some(reg) = self.reg_map.get(&reg_id) {
            return *reg;
        }
        let reg = self.temp();
        self.reg_map.insert(reg_id, reg);
        reg
    }

    fn dest_or_temp(&mut self, dest: Option<Reg>) -> Reg {
        dest.unwrap_or_else(|| self.temp())
    }

    fn temp(&mut self) -> Reg {
        let reg = self.next_reg;
        self.next_reg = self.next_reg.checked_add(1).expect("Too many temporaries");
        reg
    }
}
