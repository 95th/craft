use std::collections::VecDeque;

use bitvec::vec::BitVec;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    arena::{Arena, Id},
    ast,
    bind::{SymbolId, SymbolKind},
    interner::StrId,
    typechecker::TyCtxt,
    types::{CanonicalTy, Ty, TyId},
};

#[derive(Clone, Copy)]
struct LoopContext {
    continue_target: BasicBlockId,
    break_target: BasicBlockId,
}

pub struct Lowering<'a> {
    ast: &'a ast::AstNodes,
    tcx: &'a mut TyCtxt,
    basic_blocks: Arena<BasicBlock>,
    reg_map: BitVec,
    current_block: BasicBlockId,
    function_worklist: VecDeque<FunctionId>,
    loop_contexts: Vec<LoopContext>,
    symbol_registers: FxHashMap<SymbolId, RegId>,
    return_reg: RegId,
    reg_maps: FxHashMap<BasicBlockId, BitVec>,
    fn_registry: FnRegistry,
    subst: FxHashMap<TyId, TyId>,
}

impl<'a> Lowering<'a> {
    pub fn new(ast: &'a ast::AstNodes, tcx: &'a mut TyCtxt) -> Self {
        Self {
            ast,
            tcx,
            basic_blocks: Arena::new(),
            reg_map: BitVec::new(),
            current_block: BasicBlockId::MAX,
            function_worklist: VecDeque::new(),
            loop_contexts: Vec::new(),
            symbol_registers: FxHashMap::default(),
            return_reg: RegId::MAX,
            reg_maps: FxHashMap::default(),
            fn_registry: FnRegistry::new(),
            subst: FxHashMap::default(),
        }
    }

    pub fn lower(mut self, stmts: &[ast::StmtId]) -> Ir {
        let main = self.create_block();
        self.current_block = main;
        for &stmt in stmts {
            self.lower_stmt(stmt);
        }
        self.reg_maps
            .insert(main, std::mem::take(&mut self.reg_map));

        let mut functions = FxHashMap::default();
        while let Some(function_id) = self.function_worklist.pop_front() {
            if functions.contains_key(&function_id) {
                continue;
            }
            let (symbol, subst) = self.fn_registry.vec[function_id as usize].clone();
            let function = self.tcx.bcx.symbols[symbol].declaration;
            let entry = self.create_block();
            functions.insert(function_id, entry);
            self.current_block = entry;
            self.subst = subst;
            self.lower_function(function);
            self.reg_maps
                .insert(entry, std::mem::take(&mut self.reg_map));
        }

        Ir {
            main,
            functions,
            basic_blocks: self.basic_blocks,
            reg_maps: self.reg_maps,
        }
    }

    fn lower_function(&mut self, function: ast::StmtId) {
        let symbol = self.ast.stmts.value_binding(function);
        let ty = self.tcx.symbol_types[&symbol];
        let Ty::Fn(_, return_ty) = self.resolve_ty(ty) else {
            unreachable!()
        };
        self.return_reg = self.new_reg(return_ty);
        match &self.ast.stmts[function] {
            ast::Stmt::Fn(f) => {
                for &p in f.params.iter() {
                    self.lower_stmt(p);
                }
                self.lower_expr_into(self.return_reg, f.body);
            }
            ast::Stmt::TupleStruct(..) => {
                let symbol = self.ast.stmts.type_binding(function);
                let ty = self.tcx.tuple_struct_properties[&symbol].clone();
                let mut fields = Vec::with_capacity(ty.len());
                let mut ref_map = BitVec::with_capacity(ty.len());
                for &ty in ty.iter() {
                    let reg = self.new_reg(ty);
                    self.push_stmt(Statement::Param { reg });
                    fields.push(reg);
                    ref_map.push(self.is_ref_ty(ty));
                }
                self.push_stmt(Statement::Assign {
                    reg: self.return_reg,
                    value: RValue::Struct { fields, ref_map },
                });
            }
            _ => unreachable!(),
        }
    }

    fn lower_stmt(&mut self, stmt: ast::StmtId) {
        match self.ast.stmts[stmt] {
            ast::Stmt::Let { init, .. } => {
                let (reg, symbol) = self.new_reg_for_decl(stmt);
                self.lower_expr_into(reg, init);
                self.symbol_registers.insert(symbol, reg);
            }
            ast::Stmt::Expr { expr, .. } => {
                self.lower_expr(expr);
            }
            ast::Stmt::Param { .. } => {
                let (reg, symbol) = self.new_reg_for_decl(stmt);
                self.push_stmt(Statement::Param { reg });
                self.symbol_registers.insert(symbol, reg);
            }
            _ => {}
        }
    }

    fn lower_expr(&mut self, expr: ast::ExprId) -> Operand {
        let ty = self.tcx.expr_types[&expr];
        match self.ast.exprs[expr] {
            ast::Expr::Variable(..) => {
                let symbol_id = self.ast.exprs.value_binding(expr);
                let symbol = &self.tcx.bcx.symbols[symbol_id];
                match symbol.kind {
                    SymbolKind::Let | SymbolKind::Param => {
                        Operand::Reg(self.symbol_registers[&symbol_id])
                    }
                    SymbolKind::Fn => {
                        let ty = self.concrete_ty(ty);
                        let declared_ty = self.tcx.symbol_types[&symbol_id];
                        let function_id =
                            self.fn_registry
                                .create_mono_id(symbol_id, declared_ty, ty, self.tcx);
                        self.function_worklist.push_back(function_id);
                        Operand::Constant(Constant::Function(function_id))
                    }
                    _ => unreachable!(),
                }
            }
            ast::Expr::Bool(x) => Operand::Constant(Constant::Bool(x)),
            ast::Expr::Number(x) => Operand::Constant(Constant::F64(x)),
            ast::Expr::Int(x) => {
                if self.resolve_ty(ty) == Ty::U64 {
                    Operand::Constant(Constant::U64(x as u64))
                } else {
                    Operand::Constant(Constant::I64(x as i64))
                }
            }
            _ => {
                let dest = self.new_reg_for_expr(expr);
                self.lower_expr_into(dest, expr);
                Operand::Reg(dest)
            }
        }
    }

    fn lower_expr_into(&mut self, dest: RegId, expr: ast::ExprId) {
        crate::stack_safe(|| self.unsafe_lower_expr_into(dest, expr));
    }

    fn unsafe_lower_expr_into(&mut self, dest: RegId, expr: ast::ExprId) {
        let ty = self.tcx.expr_types[&expr];
        match self.ast.exprs[expr] {
            ast::Expr::Bool(x) => {
                self.store_constant(dest, Constant::Bool(x));
            }
            ast::Expr::Number(x) => {
                self.store_constant(dest, Constant::F64(x));
            }
            ast::Expr::Int(x) => {
                let constant = if self.resolve_ty(ty) == Ty::U64 {
                    Constant::U64(x as u64)
                } else {
                    Constant::I64(x as i64)
                };
                self.store_constant(dest, constant);
            }
            ast::Expr::String(s) => {
                self.store_value(dest, RValue::String(s));
            }
            ast::Expr::Binary {
                lhs,
                op: ast::BinaryOp::Or,
                rhs,
            } => {
                self.lower_expr_into(dest, lhs);

                let merge_block = self.create_block();
                let false_block = self.create_block();
                self.set_terminator(Terminator::Branch {
                    cond: Operand::Reg(dest),
                    then_block: merge_block,
                    else_block: false_block,
                });

                self.current_block = false_block;
                self.lower_expr_into(dest, rhs);
                self.set_terminator(Terminator::Jump {
                    target: merge_block,
                });

                self.current_block = merge_block;
            }
            ast::Expr::Binary {
                lhs,
                op: ast::BinaryOp::And,
                rhs,
            } => {
                self.lower_expr_into(dest, lhs);
                let merge_block = self.create_block();
                let true_block = self.create_block();
                self.set_terminator(Terminator::Branch {
                    cond: Operand::Reg(dest),
                    then_block: true_block,
                    else_block: merge_block,
                });

                self.current_block = true_block;
                self.lower_expr_into(dest, rhs);
                self.set_terminator(Terminator::Jump {
                    target: merge_block,
                });

                self.current_block = merge_block;
            }
            ast::Expr::Binary { lhs, op, rhs } => {
                let lhs_ty = self.tcx.expr_types[&lhs];
                let op = BinaryOp::from_ast(op, self.resolve_ty(lhs_ty));
                let lhs = self.lower_expr(lhs);
                let rhs = self.lower_expr(rhs);
                self.store_value(dest, RValue::Binary { op, lhs, rhs });
            }
            ast::Expr::Not(e) => {
                let reg = self.lower_expr(e);
                self.store_value(dest, RValue::Not(reg))
            }
            ast::Expr::Negate(e) => {
                let reg = self.lower_expr(e);
                let value = match self.resolve_ty(ty) {
                    Ty::F64 => RValue::F64Neg(reg),
                    Ty::I64 => RValue::I64Neg(reg),
                    _ => unreachable!(),
                };
                self.store_value(dest, value);
            }
            ast::Expr::Call { callee, ref args } => {
                let symbol = self.ast.exprs.value_binding(callee);
                if symbol == self.tcx.bcx.std.print {
                    let arg = self.lower_expr(args[0]);
                    let arg_ty = self.tcx.expr_types[&args[0]];
                    self.push_stmt(Statement::Print(arg, arg_ty));
                    return;
                }

                if symbol == self.tcx.bcx.std.assert {
                    let arg = self.lower_expr(args[0]);
                    self.push_stmt(Statement::Assert(arg));
                    return;
                }

                if symbol != SymbolId::MAX
                    && let decl = self.tcx.bcx.symbols[symbol].declaration
                    && let ast::Stmt::TupleStruct(_) = &self.ast.stmts[decl]
                {
                    self.lower_struct_into(dest, args);
                } else {
                    let callee = self.lower_expr(callee);
                    let return_reg = self.new_reg_for_expr(expr);
                    let regs: Vec<_> = args.iter().map(|arg| self.new_reg_for_expr(*arg)).collect();
                    for i in 0..args.len() {
                        self.lower_expr_into(regs[i], args[i]);
                    }
                    self.push_stmt(Statement::Call {
                        callee,
                        return_reg,
                        args: regs,
                    });
                    self.store_value(dest, RValue::Use(Operand::Reg(return_reg)))
                }
            }
            ast::Expr::Array(ref values) => {
                let Ty::Array(el_ty) = self.resolve_ty(ty) else {
                    unreachable!()
                };
                let regs: Vec<_> = values
                    .iter()
                    .map(|arg| self.new_reg_for_expr(*arg))
                    .collect();
                for i in 0..values.len() {
                    self.lower_expr_into(regs[i], values[i]);
                }
                let is_ref = self.is_ref_ty(el_ty);
                self.store_value(
                    dest,
                    RValue::Array {
                        elements: regs,
                        is_ref,
                    },
                )
            }
            ast::Expr::Tuple(ref fields) => {
                self.lower_struct_into(dest, fields);
            }
            ast::Expr::ArrayRepeat { expr, count } => {
                let value = self.lower_expr(expr);
                let count = self.lower_expr(count);
                let is_ref = self.has_ref_ty(expr);
                self.store_value(
                    dest,
                    RValue::ArrayRepeat {
                        value,
                        count,
                        is_ref,
                    },
                )
            }
            ast::Expr::Struct(ref s) => {
                let symbol = self.ast.exprs.value_binding(expr);
                let declared = self.tcx.struct_properties[&symbol].clone();
                let regs: Vec<_> = declared
                    .keys()
                    .map(|name| self.new_reg_for_expr(s.fields[name]))
                    .collect();
                let mut ref_map = BitVec::with_capacity(declared.len());
                for (i, name) in declared.keys().enumerate() {
                    let field = s.fields[name];
                    self.lower_expr_into(regs[i], field);
                    ref_map.push(self.has_ref_ty(field));
                }
                self.store_value(
                    dest,
                    RValue::Struct {
                        fields: regs,
                        ref_map,
                    },
                )
            }
            ast::Expr::StructField { obj, name } => {
                let obj_ty = self.tcx.expr_types[&obj];
                let reg = self.lower_expr(obj);
                let Ty::Named(symbol) = self.resolve_ty(obj_ty) else {
                    unreachable!();
                };
                let properties = &self.tcx.struct_properties[&symbol];
                let index = properties.get_index_of(&name).unwrap();
                self.store_value(
                    dest,
                    RValue::Field {
                        base: reg,
                        index: index as u16,
                    },
                );
            }
            ast::Expr::TupleField { obj, index } => {
                let reg = self.lower_expr(obj);
                self.store_value(
                    dest,
                    RValue::Field {
                        base: reg,
                        index: index as u16,
                    },
                );
            }
            ast::Expr::ArrayIndex { array, index } => {
                let array = self.lower_expr(array);
                let index = self.lower_expr(index);
                self.store_value(dest, RValue::Index { base: array, index });
            }
            ast::Expr::Block { ref stmts, expr } => {
                for &stmt in stmts {
                    self.lower_stmt(stmt);
                }
                if let Some(expr) = expr {
                    self.lower_expr_into(dest, expr)
                } else {
                    self.store_constant(dest, Constant::Unit);
                }
            }
            ast::Expr::Variable(..) => {
                let symbol_id = self.ast.exprs.value_binding(expr);
                let symbol = &self.tcx.bcx.symbols[symbol_id];
                match symbol.kind {
                    SymbolKind::Let | SymbolKind::Param => {
                        let reg = self.symbol_registers[&symbol_id];
                        if reg != dest {
                            self.push_stmt(Statement::Assign {
                                reg: dest,
                                value: RValue::Use(Operand::Reg(reg)),
                            });
                        }
                    }
                    SymbolKind::Fn => {
                        let ty = self.concrete_ty(ty);
                        let declared_ty = self.tcx.symbol_types[&symbol_id];
                        let function_id =
                            self.fn_registry
                                .create_mono_id(symbol_id, declared_ty, ty, self.tcx);
                        self.function_worklist.push_back(function_id);
                        self.store_constant(dest, Constant::Function(function_id));
                    }
                    _ => unreachable!(),
                };
            }
            ast::Expr::If {
                cond,
                then_clause,
                else_clause,
            } => {
                let cond = self.lower_expr(cond);

                let then_block = self.create_block();
                let else_block = else_clause.map(|_| self.create_block());
                let merge_block = self.create_block();

                self.set_terminator(Terminator::Branch {
                    cond,
                    then_block,
                    else_block: else_block.unwrap_or(merge_block),
                });

                self.current_block = then_block;
                self.lower_expr_into(dest, then_clause);
                self.set_terminator(Terminator::Jump {
                    target: merge_block,
                });

                if let Some(else_clause) = else_clause
                    && let Some(else_block) = else_block
                {
                    self.current_block = else_block;
                    self.lower_expr_into(dest, else_clause);
                    self.set_terminator(Terminator::Jump {
                        target: merge_block,
                    });
                }

                self.current_block = merge_block;
            }
            ast::Expr::Assign { lhs, rhs } => {
                match self.ast.exprs[lhs] {
                    ast::Expr::Variable(_) => {
                        let symbol = self.ast.exprs.value_binding(lhs);
                        let place = self.symbol_registers[&symbol];
                        self.lower_expr_into(place, rhs);
                    }
                    ast::Expr::TupleField { obj, index } => {
                        let obj = self.lower_expr(obj);
                        let value = self.lower_expr(rhs);
                        self.push_stmt(Statement::SetField {
                            base: obj,
                            index,
                            value,
                        });
                    }
                    ast::Expr::StructField { obj, name } => {
                        let ty = self.tcx.expr_types[&obj];
                        let Ty::Named(symbol) = self.resolve_ty(ty) else {
                            unreachable!();
                        };
                        let properties = &self.tcx.struct_properties[&symbol];
                        let index = properties.get_index_of(&name).unwrap() as u16;

                        let obj = self.lower_expr(obj);
                        let value = self.lower_expr(rhs);
                        self.push_stmt(Statement::SetField {
                            base: obj,
                            index,
                            value,
                        });
                    }
                    ast::Expr::ArrayIndex { array, index } => {
                        let array = self.lower_expr(array);
                        let index = self.lower_expr(index);
                        let value = self.lower_expr(rhs);
                        self.push_stmt(Statement::SetIndex {
                            base: array,
                            index,
                            value,
                        });
                    }
                    _ => unreachable!(),
                };
            }
            ast::Expr::While { cond, body } => {
                let body_block = self.create_block();
                let cond_block = self.create_block();
                let merge_block = self.create_block();

                let cond_reg = self.lower_expr(cond);
                self.set_terminator(Terminator::Branch {
                    cond: cond_reg,
                    then_block: body_block,
                    else_block: merge_block,
                });

                self.current_block = body_block;
                self.enter_loop(cond_block, merge_block);
                self.lower_expr(body);
                self.loop_contexts.pop();
                self.set_terminator(Terminator::Jump { target: cond_block });

                self.current_block = cond_block;
                let cond_reg = self.lower_expr(cond);
                self.set_terminator(Terminator::Branch {
                    cond: cond_reg,
                    then_block: body_block,
                    else_block: merge_block,
                });

                self.current_block = merge_block;
            }
            ast::Expr::For {
                init,
                cond,
                incr,
                body,
            } => {
                if let Some(init) = init {
                    self.lower_stmt(init);
                }

                let body_block = self.create_block();
                let incr_block = self.create_block();
                let merge_block = self.create_block();

                if let Some(cond) = cond {
                    let cond = self.lower_expr(cond);
                    self.set_terminator(Terminator::Branch {
                        cond,
                        then_block: body_block,
                        else_block: merge_block,
                    });
                } else {
                    self.set_terminator(Terminator::Jump { target: body_block });
                }

                self.current_block = body_block;
                self.enter_loop(incr_block, merge_block);
                self.lower_expr(body);
                self.loop_contexts.pop();
                self.set_terminator(Terminator::Jump { target: incr_block });

                self.current_block = incr_block;
                if let Some(incr) = incr {
                    self.lower_expr(incr);
                }
                if let Some(cond) = cond {
                    let cond = self.lower_expr(cond);
                    self.set_terminator(Terminator::Branch {
                        cond,
                        then_block: body_block,
                        else_block: merge_block,
                    });
                } else {
                    self.set_terminator(Terminator::Jump { target: body_block });
                }

                self.current_block = merge_block;
            }
            ast::Expr::Break => {
                let context = *self.loop_contexts.last().unwrap();
                self.set_terminator(Terminator::Jump {
                    target: context.break_target,
                });
                self.current_block = self.create_block();
            }
            ast::Expr::Continue => {
                let context = *self.loop_contexts.last().unwrap();
                self.set_terminator(Terminator::Jump {
                    target: context.continue_target,
                });

                self.current_block = self.create_block();
            }
            ast::Expr::Return(expr) => {
                if let Some(expr) = expr {
                    self.lower_expr_into(self.return_reg, expr);
                }
                self.set_terminator(Terminator::Return);
                self.current_block = self.create_block();
            }
            ast::Expr::Unit => {
                self.store_constant(dest, Constant::Unit);
            }
        }
    }

    fn lower_struct_into(&mut self, dest: RegId, fields: &[ast::ExprId]) {
        let regs: Vec<_> = fields
            .iter()
            .map(|arg| self.new_reg_for_expr(*arg))
            .collect();
        let mut ref_map = BitVec::with_capacity(fields.len());
        for i in 0..fields.len() {
            self.lower_expr_into(regs[i], fields[i]);
            ref_map.push(self.has_ref_ty(fields[i]));
        }
        self.store_value(
            dest,
            RValue::Struct {
                fields: regs,
                ref_map,
            },
        )
    }

    fn has_ref_ty(&mut self, expr: ast::ExprId) -> bool {
        self.is_ref_ty(self.tcx.expr_types[&expr])
    }

    fn is_ref_ty(&mut self, ty: TyId) -> bool {
        match self.resolve_ty(ty) {
            Ty::Unit | Ty::Bool | Ty::F64 | Ty::I64 | Ty::U64 | Ty::Fn(..) | Ty::Never => false,
            Ty::String | Ty::Named(..) | Ty::Tuple(..) | Ty::Array(..) => true,
            Ty::Unknown | Ty::UnknownInt | Ty::TypeParam(..) => unreachable!(),
        }
    }

    fn store_constant(&mut self, dest: RegId, value: Constant) {
        self.store_value(dest, RValue::Use(Operand::Constant(value)));
    }

    fn store_value(&mut self, dest: RegId, value: RValue) {
        self.push_stmt(Statement::Assign { reg: dest, value });
    }

    fn set_terminator(&mut self, terminator: Terminator) {
        if cfg!(feature = "trace") {
            println!("{:?} => {:?}", self.current_block, terminator);
        }
        self.basic_blocks[self.current_block].terminator = terminator;
    }

    fn create_block(&mut self) -> BasicBlockId {
        self.basic_blocks.alloc(BasicBlock {
            statements: Vec::new(),
            terminator: Terminator::Return,
        })
    }

    fn new_reg_for_decl(&mut self, stmt: ast::StmtId) -> (RegId, SymbolId) {
        let symbol = self.ast.stmts.value_binding(stmt);
        let ty = self.tcx.symbol_types[&symbol];
        let reg = self.new_reg(ty);
        (reg, symbol)
    }

    fn new_reg_for_expr(&mut self, expr: ast::ExprId) -> RegId {
        let ty = self.tcx.expr_types[&expr];
        self.new_reg(ty)
    }

    fn new_reg(&mut self, ty: TyId) -> RegId {
        let is_ref = self.is_ref_ty(ty);
        let reg = self.reg_map.len() as RegId;
        self.reg_map.push(is_ref);
        reg
    }

    fn push_stmt(&mut self, statement: Statement) {
        if cfg!(feature = "trace") {
            println!("{:?} => {:?}", self.current_block, statement);
        }
        self.basic_blocks[self.current_block]
            .statements
            .push(statement);
    }

    fn enter_loop(&mut self, continue_target: BasicBlockId, break_target: BasicBlockId) {
        self.loop_contexts.push(LoopContext {
            continue_target,
            break_target,
        })
    }

    fn resolve_ty(&mut self, ty: TyId) -> Ty {
        let ty = self.concrete_ty(ty);
        self.tcx.types.probe_value(ty)
    }

    fn concrete_ty(&mut self, ty: TyId) -> TyId {
        if self.subst.is_empty() {
            ty
        } else {
            ty.substitute(&mut self.tcx.types, &self.subst)
        }
    }
}

pub type BasicBlockId = Id<BasicBlock>;
pub type RegId = u32;
pub type FunctionId = u32;

#[derive(PartialEq, Eq, Hash)]
struct InstanceKey {
    symbol: SymbolId,
    type_params: Vec<CanonicalTy>,
}

struct FnRegistry {
    map: FxHashMap<InstanceKey, FunctionId>,
    vec: Vec<(SymbolId, FxHashMap<TyId, TyId>)>,
}

impl FnRegistry {
    fn new() -> Self {
        Self {
            map: FxHashMap::default(),
            vec: Vec::new(),
        }
    }

    fn create_mono_id(
        &mut self,
        symbol: SymbolId,
        declared_ty: TyId,
        actual_ty: TyId,
        tcx: &mut TyCtxt,
    ) -> FunctionId {
        if cfg!(feature = "trace") {
            let declared = declared_ty.debug(&mut tcx.types);
            let actual = actual_ty.debug(&mut tcx.types);
            println!("Computing the substitution map from {declared} to {actual}");
        }
        let mut subst = FxHashMap::default();
        let type_params = if let Some(type_params) = tcx.type_params.get(&declared_ty) {
            for t in type_params {
                subst.insert(*t, tcx.types.new_key(Ty::Unknown));
            }
            let subst_ty = declared_ty.substitute(&mut tcx.types, &subst);
            subst_ty.unify(actual_ty, &mut tcx.types);
            type_params
                .iter()
                .map(|t| subst[t].canonicalize(&mut tcx.types))
                .collect()
        } else {
            Vec::new()
        };
        let key = InstanceKey {
            symbol,
            type_params,
        };
        if let Some(id) = self.map.get(&key) {
            return *id;
        }
        let id = self.vec.len() as FunctionId;
        self.vec.push((symbol, subst));
        self.map.insert(key, id);
        id
    }
}

pub struct Ir {
    pub main: BasicBlockId,
    pub functions: FxHashMap<FunctionId, BasicBlockId>,
    pub basic_blocks: Arena<BasicBlock>,
    pub reg_maps: FxHashMap<BasicBlockId, BitVec>,
}

pub struct BasicBlock {
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

#[derive(Debug)]
pub enum Statement {
    Assign {
        reg: RegId,
        value: RValue,
    },
    SetField {
        base: Operand,
        index: u16,
        value: Operand,
    },
    SetIndex {
        base: Operand,
        index: Operand,
        value: Operand,
    },
    Param {
        reg: RegId,
    },
    Call {
        callee: Operand,
        return_reg: RegId,
        args: Vec<RegId>,
    },
    Print(Operand, TyId),
    Assert(Operand),
}

#[derive(Debug)]
pub enum Terminator {
    Jump {
        target: BasicBlockId,
    },
    Branch {
        cond: Operand,
        then_block: BasicBlockId,
        else_block: BasicBlockId,
    },
    Return,
}

#[derive(Debug)]
pub enum RValue {
    Use(Operand),
    Field {
        base: Operand,
        index: u16,
    },
    Index {
        base: Operand,
        index: Operand,
    },
    Binary {
        op: BinaryOp,
        lhs: Operand,
        rhs: Operand,
    },
    Not(Operand),
    F64Neg(Operand),
    I64Neg(Operand),
    String(StrId),
    Struct {
        fields: Vec<RegId>,
        ref_map: BitVec,
    },
    Array {
        elements: Vec<RegId>,
        is_ref: bool,
    },
    ArrayRepeat {
        value: Operand,
        count: Operand,
        is_ref: bool,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum Constant {
    Unit,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    Function(FunctionId),
}

impl PartialEq for Constant {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Unit, Self::Unit) => true,
            (Self::Bool(l), Self::Bool(r)) => l == r,
            (Self::I64(l), Self::I64(r)) => l == r,
            (Self::U64(l), Self::U64(r)) => l == r,
            (Self::F64(l), Self::F64(r)) => l.to_bits() == r.to_bits(),
            (Self::Function(l), Self::Function(r)) => l == r,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Operand {
    Constant(Constant),
    Reg(RegId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    F64Add,
    F64Sub,
    F64Mul,
    F64Div,
    F64Greater,
    F64GreaterEqual,
    F64Less,
    F64LessEqual,
    F64Equal,
    F64NotEqual,

    I64Add,
    I64Sub,
    I64Mul,
    I64Div,
    I64Greater,
    I64GreaterEqual,
    I64Less,
    I64LessEqual,
    I64Or,
    I64And,

    U64Add,
    U64Sub,
    U64Mul,
    U64Div,
    U64Greater,
    U64GreaterEqual,
    U64Less,
    U64LessEqual,
    U64Or,
    U64And,

    Equal,
    NotEqual,
}

impl BinaryOp {
    fn from_ast(op: ast::BinaryOp, ty: Ty) -> Self {
        match ty {
            Ty::F64 => match op {
                ast::BinaryOp::Add => Self::F64Add,
                ast::BinaryOp::Sub => Self::F64Sub,
                ast::BinaryOp::Mul => Self::F64Mul,
                ast::BinaryOp::Div => Self::F64Div,
                ast::BinaryOp::Greater => Self::F64Greater,
                ast::BinaryOp::GreaterEqual => Self::F64GreaterEqual,
                ast::BinaryOp::Less => Self::F64Less,
                ast::BinaryOp::LessEqual => Self::F64LessEqual,
                ast::BinaryOp::Equal => Self::F64Equal,
                ast::BinaryOp::NotEqual => Self::F64NotEqual,
                _ => unreachable!(),
            },
            Ty::I64 => match op {
                ast::BinaryOp::Add => Self::I64Add,
                ast::BinaryOp::Sub => Self::I64Sub,
                ast::BinaryOp::Mul => Self::I64Mul,
                ast::BinaryOp::Div => Self::I64Div,
                ast::BinaryOp::Greater => Self::I64Greater,
                ast::BinaryOp::GreaterEqual => Self::I64GreaterEqual,
                ast::BinaryOp::Less => Self::I64Less,
                ast::BinaryOp::LessEqual => Self::I64LessEqual,
                ast::BinaryOp::Equal => Self::Equal,
                ast::BinaryOp::NotEqual => Self::NotEqual,
                ast::BinaryOp::BitOr => Self::I64Or,
                ast::BinaryOp::BitAnd => Self::I64And,
                _ => unreachable!(),
            },
            Ty::U64 => match op {
                ast::BinaryOp::Add => Self::U64Add,
                ast::BinaryOp::Sub => Self::U64Sub,
                ast::BinaryOp::Mul => Self::U64Mul,
                ast::BinaryOp::Div => Self::U64Div,
                ast::BinaryOp::Greater => Self::U64Greater,
                ast::BinaryOp::GreaterEqual => Self::U64GreaterEqual,
                ast::BinaryOp::Less => Self::U64Less,
                ast::BinaryOp::LessEqual => Self::U64LessEqual,
                ast::BinaryOp::Equal => Self::Equal,
                ast::BinaryOp::NotEqual => Self::NotEqual,
                ast::BinaryOp::BitOr => Self::U64Or,
                ast::BinaryOp::BitAnd => Self::U64And,
                _ => unreachable!(),
            },
            _ => match op {
                ast::BinaryOp::Equal => Self::Equal,
                ast::BinaryOp::NotEqual => Self::NotEqual,
                _ => unreachable!(),
            },
        }
    }
}

impl Ir {
    pub fn for_each_successor(&self, entry: BasicBlockId, mut f: impl FnMut(BasicBlockId)) {
        match self.basic_blocks[entry].terminator {
            Terminator::Branch {
                then_block,
                else_block,
                ..
            } => {
                f(then_block);
                f(else_block);
            }
            Terminator::Jump { target } => f(target),
            Terminator::Return => {}
        }
    }

    pub fn reachable_reverse_postorder(&self, entry: BasicBlockId) -> Vec<BasicBlockId> {
        let mut blocks = self.reachable_postorder(entry);
        blocks.reverse();
        blocks
    }

    pub fn reachable_postorder(&self, entry: BasicBlockId) -> Vec<BasicBlockId> {
        fn dfs(
            ir: &Ir,
            block: BasicBlockId,
            seen: &mut FxHashSet<BasicBlockId>,
            list: &mut Vec<BasicBlockId>,
        ) {
            if !seen.insert(block) {
                return;
            }
            ir.for_each_successor(block, |block| dfs(ir, block, seen, list));
            list.push(block);
        }

        let mut list = Vec::new();
        let mut seen = FxHashSet::default();
        dfs(self, entry, &mut seen, &mut list);
        list
    }

    pub fn regs(&self, entry: BasicBlockId) -> Regs<'_> {
        Regs { ir: self, entry }
    }
}

pub struct Regs<'a> {
    ir: &'a Ir,
    entry: BasicBlockId,
}

impl<'a> Regs<'a> {
    pub fn visit_stmt(&self, stmt: &Statement, visitor: &mut impl RegVisitor) {
        match *stmt {
            Statement::Assign { reg, ref value } => {
                self.visit_rvalue(value, visitor);
                visitor.on_def(reg);
            }
            Statement::SetField {
                base: reg, value, ..
            } => {
                self.visit_operand(reg, visitor);
                self.visit_operand(value, visitor);
            }
            Statement::SetIndex {
                base: reg,
                index,
                value,
            } => {
                self.visit_operand(reg, visitor);
                self.visit_operand(index, visitor);
                self.visit_operand(value, visitor);
            }
            Statement::Param { reg } => {
                visitor.on_def(reg);
            }
            Statement::Print(operand, ..) => self.visit_operand(operand, visitor),
            Statement::Call {
                callee,
                return_reg,
                ref args,
            } => {
                self.visit_operand(callee, visitor);
                for arg in args {
                    visitor.on_use(*arg);
                }
                visitor.on_def(return_reg);
            }
            Statement::Assert(operand) => self.visit_operand(operand, visitor),
        }
    }

    pub fn visit_terminator(&self, block: BasicBlockId, visitor: &mut impl RegVisitor) {
        match self.ir.basic_blocks[block].terminator {
            Terminator::Jump { .. } => {}
            Terminator::Branch { cond, .. } => self.visit_operand(cond, visitor),
            Terminator::Return => {
                if self.entry != self.ir.main {
                    // Return variable is required even though there is no explicit read.
                    visitor.on_use(0);
                }
            }
        }
    }

    pub fn visit_rvalue(&self, value: &RValue, visitor: &mut impl RegVisitor) {
        match *value {
            RValue::Use(operand) => self.visit_operand(operand, visitor),
            RValue::Field { base, .. } => self.visit_operand(base, visitor),
            RValue::Index { base, index } => {
                self.visit_operand(base, visitor);
                self.visit_operand(index, visitor);
            }
            RValue::Binary { lhs, rhs, .. } => {
                self.visit_operand(lhs, visitor);
                self.visit_operand(rhs, visitor);
            }
            RValue::Not(operand) => self.visit_operand(operand, visitor),
            RValue::F64Neg(operand) => self.visit_operand(operand, visitor),
            RValue::I64Neg(operand) => self.visit_operand(operand, visitor),
            RValue::String(..) => {}
            RValue::Struct { ref fields, .. } => {
                for value in fields.iter() {
                    visitor.on_use(*value);
                }
            }
            RValue::Array { ref elements, .. } => {
                for e in elements.iter() {
                    visitor.on_use(*e);
                }
            }
            RValue::ArrayRepeat { value, count, .. } => {
                self.visit_operand(value, visitor);
                self.visit_operand(count, visitor);
            }
        }
    }

    pub fn visit_operand(&self, operand: Operand, visitor: &mut impl RegVisitor) {
        match operand {
            Operand::Constant(..) => {}
            Operand::Reg(reg) => visitor.on_use(reg),
        }
    }
}

pub trait RegVisitor {
    fn on_def(&mut self, reg: RegId);
    fn on_use(&mut self, reg: RegId);
}
