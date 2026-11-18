use std::collections::VecDeque;

use rustc_hash::FxHashMap;

use crate::{
    ast::{AstNodes, BinaryOp, Expr, ExprId, Stmt, StmtId},
    bind::{SymbolId, SymbolKind},
    chunk::{Code, Opcode},
    interner::{StrId, StrInterner},
    typechecker::TyContext,
    types::Ty,
};

struct LoopCtx {
    offset: usize,
    break_offsets: Vec<usize>,
}

impl LoopCtx {
    fn new(offset: usize) -> Self {
        Self {
            offset,
            break_offsets: Vec::new(),
        }
    }
}

pub struct Codegen<'a> {
    ast: &'a AstNodes,
    interner: &'a StrInterner,
    ty_ctx: TyContext,
    code: Code,
    loop_ctxts: Vec<LoopCtx>,
    function_table: FxHashMap<SymbolId, usize>,
    function_worklist: VecDeque<(SymbolId, usize)>,
    string_worklist: Vec<(usize, StrId)>,
    locals: usize,
}

impl<'a> Codegen<'a> {
    pub fn new(ast: &'a AstNodes, interner: &'a StrInterner, ty_ctx: TyContext) -> Self {
        Self {
            ast,
            interner,
            ty_ctx,
            code: Code::new(),
            loop_ctxts: Vec::new(),
            function_table: FxHashMap::default(),
            function_worklist: VecDeque::new(),
            string_worklist: Vec::new(),
            locals: 0,
        }
    }

    pub fn generate(mut self, stmts: &[StmtId]) -> Code {
        for &stmt in stmts {
            self.process_stmt(stmt);
        }
        self.code.op(Opcode::Return);

        while let Some((symbol, patch_offset)) = self.function_worklist.pop_front() {
            if let Some(&function) = self.function_table.get(&symbol) {
                self.code.write_u64_at(patch_offset, function as u64);
            } else {
                let function = self.code.len();
                self.code.write_u64_at(patch_offset, function as u64);
                self.function_table.insert(symbol, function);
                let function = self.ty_ctx.bind_ctx.symbols[symbol].declaration;
                self.process_function(function);
            }
        }

        let mut string_table = FxHashMap::default();
        for (patch_offset, string) in self.string_worklist {
            let string_offset = *string_table.entry(string).or_insert_with(|| {
                let string_offset = self.code.len();
                self.code.write_string(self.interner.lookup(string));
                self.code.write_u64_at(patch_offset, string_offset as u64);
                string_offset
            });
            self.code.write_u64_at(patch_offset, string_offset as u64);
        }
        self.code
    }

    fn process_function(&mut self, function: StmtId) {
        match &self.ast.stmts[function] {
            Stmt::Fn(f) => {
                self.locals = f.params.len();
                self.process_expr(f.body);
                self.code.op_return();
            }
            Stmt::TupleStruct(s) => {
                self.locals = s.fields.len();
                self.code.op_struct(s.fields.len() as u16);
                self.code.op_return();
            }
            _ => unreachable!(),
        };
    }

    fn process_stmt(&mut self, stmt: StmtId) {
        match self.ast.stmts[stmt] {
            Stmt::Let { init, .. } => {
                self.process_expr(init);
                self.locals += 1;
            }
            Stmt::Expr { expr, .. } => {
                self.process_expr(expr);
                self.code.pop();
            }
            _ => {}
        }
    }

    fn process_expr(&mut self, expr: ExprId) {
        match self.ast.exprs[expr] {
            Expr::Bool(b) => self.code.op(if b { Opcode::True } else { Opcode::False }),
            Expr::Int(n) => {
                let ty = self.ty_ctx.expr_types[&expr];
                let value = match self.ty_ctx.types.probe_value(ty) {
                    Ty::I64 => u64::from_ne_bytes((n as i64).to_ne_bytes()),
                    Ty::U64 => n as u64,
                    t => unreachable!("{t:?}"),
                };
                self.code.op_const(value);
            }
            Expr::Number(n) => {
                self.code.op_const(n.to_bits());
            }
            Expr::String(s) => {
                let offset = self.code.len() + 1;
                self.code.op_const(0);
                self.code.op(Opcode::String);
                self.string_worklist.push((offset, s));
            }
            Expr::Not(expr) => {
                self.process_expr(expr);
                self.code.op(Opcode::Not);
            }
            Expr::Negate(expr) => {
                self.process_expr(expr);
                let ty = self.ty_ctx.expr_types[&expr];
                match self.ty_ctx.types.probe_value(ty) {
                    Ty::I64 => self.code.op(Opcode::I64Neg),
                    Ty::F64 => self.code.op(Opcode::F64Neg),
                    t => unreachable!("{t:?}"),
                }
            }
            Expr::Variable(_) => {
                let symbol_id = self.ast.exprs.value_binding(expr);
                let symbol = &self.ty_ctx.bind_ctx.symbols[symbol_id];
                match symbol.kind {
                    SymbolKind::Let | SymbolKind::Param => {
                        self.code.op_get_local(symbol.local_index);
                    }
                    SymbolKind::Fn => {
                        if let Some(&function) = self.function_table.get(&symbol_id) {
                            self.code.op_const(function as u64);
                        } else {
                            let patch_offset = self.code.len() + 1;
                            self.code.op_const(0);
                            self.function_worklist.push_back((symbol_id, patch_offset));
                        }
                    }
                    _ => unreachable!(),
                }
            }
            Expr::Assign { lhs, rhs } => match self.ast.exprs[lhs] {
                Expr::Variable(..) => {
                    let symbol = self.ast.exprs.value_binding(lhs);
                    let index = self.ty_ctx.bind_ctx.symbols[symbol].local_index;
                    self.process_expr(rhs);
                    self.code.op_set_local(index);
                    self.code.op_unit();
                }
                Expr::TupleField { obj, index } => {
                    self.process_expr(obj);
                    self.process_expr(rhs);
                    self.code.op_set_struct_property(index);
                    self.code.op_unit();
                }
                Expr::StructField { obj, name } => {
                    let ty = self.ty_ctx.expr_types[&obj];
                    let Ty::Named(symbol) = self.ty_ctx.types.probe_value(ty) else {
                        unreachable!();
                    };
                    let properties = &self.ty_ctx.struct_properties[&symbol];
                    let index = properties.get_index_of(&name).unwrap();
                    self.process_expr(obj);
                    self.process_expr(rhs);
                    self.code.op_set_struct_property(index as u16);
                    self.code.op_unit();
                }
                Expr::ArrayIndex { array, index } => {
                    self.process_expr(array);
                    self.process_expr(index);
                    self.process_expr(rhs);
                    self.code.op_set_struct_index();
                    self.code.op_unit();
                }
                _ => unreachable!(),
            },
            Expr::Binary {
                lhs,
                op: BinaryOp::And,
                rhs,
            } => {
                self.process_expr(lhs);
                let jump_false = self.code.write_jump(Opcode::JumpIfFalse);
                self.process_expr(rhs);
                let jump_end = self.code.write_jump(Opcode::Jump);
                self.code.patch_jump(jump_false);
                self.code.op(Opcode::False);
                self.code.patch_jump(jump_end);
            }
            Expr::Binary {
                lhs,
                op: BinaryOp::Or,
                rhs,
            } => {
                self.process_expr(lhs);
                let jump_rhs = self.code.write_jump(Opcode::JumpIfFalse);
                let jump_true = self.code.write_jump(Opcode::Jump);
                self.code.patch_jump(jump_rhs);
                self.process_expr(rhs);
                let jump_end = self.code.write_jump(Opcode::Jump);
                self.code.patch_jump(jump_true);
                self.code.op(Opcode::True);
                self.code.patch_jump(jump_end);
            }
            Expr::Binary { lhs, op, rhs } => {
                self.process_expr(lhs);
                self.process_expr(rhs);
                let ty = self.ty_ctx.expr_types[&lhs];
                match self.ty_ctx.types.probe_value(ty) {
                    Ty::F64 => match op {
                        BinaryOp::Add => self.code.op(Opcode::F64Add),
                        BinaryOp::Sub => self.code.op(Opcode::F64Sub),
                        BinaryOp::Mul => self.code.op(Opcode::F64Mul),
                        BinaryOp::Div => self.code.op(Opcode::F64Div),
                        BinaryOp::Greater => self.code.op(Opcode::F64Greater),
                        BinaryOp::GreaterEqual => self.code.op(Opcode::F64GreaterEqual),
                        BinaryOp::Less => self.code.op(Opcode::F64Less),
                        BinaryOp::LessEqual => self.code.op(Opcode::F64LessEqual),
                        BinaryOp::Equal => self.code.op(Opcode::F64Equal),
                        BinaryOp::NotEqual => self.code.op(Opcode::F64NotEqual),
                        _ => {}
                    },
                    Ty::I64 => match op {
                        BinaryOp::Add => self.code.op(Opcode::I64Add),
                        BinaryOp::Sub => self.code.op(Opcode::I64Sub),
                        BinaryOp::Mul => self.code.op(Opcode::I64Mul),
                        BinaryOp::Div => self.code.op(Opcode::I64Div),
                        BinaryOp::Greater => self.code.op(Opcode::I64Greater),
                        BinaryOp::GreaterEqual => self.code.op(Opcode::I64GreaterEqual),
                        BinaryOp::Less => self.code.op(Opcode::I64Less),
                        BinaryOp::LessEqual => self.code.op(Opcode::I64LessEqual),
                        BinaryOp::Equal => self.code.op(Opcode::Equal),
                        BinaryOp::NotEqual => self.code.op(Opcode::NotEqual),
                        BinaryOp::BitOr => self.code.op(Opcode::I64Or),
                        BinaryOp::BitAnd => self.code.op(Opcode::I64And),
                        _ => {}
                    },
                    Ty::U64 => match op {
                        BinaryOp::Add => self.code.op(Opcode::U64Add),
                        BinaryOp::Sub => self.code.op(Opcode::U64Sub),
                        BinaryOp::Mul => self.code.op(Opcode::U64Mul),
                        BinaryOp::Div => self.code.op(Opcode::U64Div),
                        BinaryOp::Greater => self.code.op(Opcode::U64Greater),
                        BinaryOp::GreaterEqual => self.code.op(Opcode::U64GreaterEqual),
                        BinaryOp::Less => self.code.op(Opcode::U64Less),
                        BinaryOp::LessEqual => self.code.op(Opcode::U64LessEqual),
                        BinaryOp::Equal => self.code.op(Opcode::Equal),
                        BinaryOp::NotEqual => self.code.op(Opcode::NotEqual),
                        BinaryOp::BitOr => self.code.op(Opcode::U64Or),
                        BinaryOp::BitAnd => self.code.op(Opcode::U64And),
                        _ => {}
                    },
                    _ => match op {
                        BinaryOp::Equal => self.code.op(Opcode::Equal),
                        BinaryOp::NotEqual => self.code.op(Opcode::NotEqual),
                        _ => {}
                    },
                }
            }
            Expr::Call { callee, ref args } => {
                for &arg in args {
                    self.process_expr(arg);
                }
                let symbol = self.ast.exprs.value_binding(callee);
                if symbol == self.ty_ctx.bind_ctx.print_function {
                    let ty = self.ty_ctx.expr_types[&args[0]];
                    match self.ty_ctx.types.probe_value(ty) {
                        Ty::F64 => self.code.op(Opcode::PrintF64),
                        Ty::I64 => self.code.op(Opcode::PrintI64),
                        Ty::U64 => self.code.op(Opcode::PrintU64),
                        Ty::String => self.code.op(Opcode::PrintString),
                        Ty::Bool => self.code.op(Opcode::PrintBool),
                        _ => self.code.op(Opcode::PrintObj),
                    }
                    self.code.op_unit();
                } else {
                    self.process_expr(callee);
                    self.code.op_call(args.len() as u8);
                }
            }
            Expr::Block { ref stmts, expr } => {
                let prev_locals = std::mem::replace(&mut self.locals, 0);
                for &stmt in stmts {
                    self.process_stmt(stmt);
                }
                if let Some(expr) = expr {
                    self.process_expr(expr);
                    self.code.op_pop_scope(self.locals);
                } else {
                    self.code.op_pop_n(self.locals);
                    self.code.op_unit();
                }
                self.locals = prev_locals;
            }
            Expr::If {
                cond,
                then_clause,
                else_clause,
            } => {
                self.process_expr(cond);
                let then_offset = self.code.write_jump(Opcode::JumpIfFalse);
                self.process_expr(then_clause);
                if let Some(else_clause) = else_clause {
                    let else_offset = self.code.write_jump(Opcode::Jump);
                    self.code.patch_jump(then_offset);
                    self.process_expr(else_clause);
                    self.code.patch_jump(else_offset);
                } else {
                    self.code.patch_jump(then_offset);
                    self.code.op_unit();
                }
            }
            Expr::While { cond, body } => {
                self.enter_loop();
                let start = self.code.len();
                self.process_expr(cond);
                let jump_offset = self.code.write_jump(Opcode::JumpIfFalse);
                self.process_expr(body);
                self.code.pop();
                self.code.write_loop(start);
                self.code.patch_jump(jump_offset);
                self.exit_loop();
            }
            Expr::For {
                init,
                cond,
                incr,
                body,
            } => {
                let prev_locals = std::mem::replace(&mut self.locals, 0);
                if let Some(init) = init {
                    self.process_stmt(init);
                }
                self.enter_loop();
                let start = self.code.len();
                if let Some(cond) = cond {
                    self.process_expr(cond);
                    let jump_offset = self.code.write_jump(Opcode::JumpIfFalse);
                    self.process_expr(body);
                    self.code.pop();
                    if let Some(incr) = incr {
                        self.process_expr(incr);
                        self.code.pop();
                    }
                    self.code.write_loop(start);
                    self.code.patch_jump(jump_offset);
                } else {
                    self.process_expr(body);
                    self.code.pop();
                    if let Some(incr) = incr {
                        self.process_expr(incr);
                        self.code.pop();
                    }
                    self.code.write_loop(start);
                }
                self.exit_loop();
                self.code.op_pop_n(self.locals);
                self.code.op_unit();
                self.locals = prev_locals;
            }
            Expr::Break => {
                let offset = self.code.write_jump(Opcode::Jump);
                self.push_loop_break(offset);
                self.code.pop();
            }
            Expr::Continue => {
                self.code
                    .write_loop(self.loop_ctxts.last_mut().unwrap().offset);
                self.code.pop();
            }
            Expr::Unit => self.code.op_unit(),
            Expr::Return(expr) => {
                if let Some(expr) = expr {
                    self.process_expr(expr);
                } else {
                    self.code.op_unit();
                }
                self.code.op_return();
            }
            Expr::Tuple(ref args) => {
                for &arg in args {
                    self.process_expr(arg);
                }
                self.code.op_struct(args.len() as u16);
            }
            Expr::TupleField { obj, index } => {
                self.process_expr(obj);
                self.code.op_get_struct_property(index);
            }
            Expr::Struct(ref s) => {
                for &expr in s.fields.values() {
                    self.process_expr(expr);
                }
                self.code.op_struct(s.fields.len() as u16);
            }
            Expr::StructField { obj, name } => {
                let ty = self.ty_ctx.expr_types[&obj];
                let Ty::Named(symbol) = self.ty_ctx.types.probe_value(ty) else {
                    unreachable!();
                };
                let properties = &self.ty_ctx.struct_properties[&symbol];
                let index = properties.get_index_of(&name).unwrap();
                self.process_expr(obj);
                self.code.op_get_struct_property(index as u16);
            }
            Expr::Array(ref args) => {
                for &arg in args {
                    self.process_expr(arg);
                }
                self.code.op_struct(args.len() as u16);
            }
            Expr::ArrayRepeat { expr, count } => {
                self.process_expr(expr);
                self.process_expr(count);
                self.code.op_repeat_struct();
            }
            Expr::ArrayIndex { array, index } => {
                self.process_expr(array);
                self.process_expr(index);
                self.code.op_get_struct_index();
            }
        }
    }

    fn enter_loop(&mut self) {
        self.loop_ctxts.push(LoopCtx::new(self.code.len()));
    }

    fn push_loop_break(&mut self, offset: usize) {
        self.loop_ctxts
            .last_mut()
            .unwrap()
            .break_offsets
            .push(offset);
    }

    fn exit_loop(&mut self) {
        let ctx = self.loop_ctxts.pop().unwrap();
        for offset in ctx.break_offsets {
            self.code.patch_jump(offset);
        }
    }
}
