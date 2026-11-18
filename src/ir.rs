use std::collections::VecDeque;

use rustc_hash::FxHashMap;

use crate::{
    arena::{Arena, Id},
    ast,
    bind::{SymbolId, SymbolKind},
    interner::StrId,
    typechecker::TyCtxt,
    types::{Ty, TyId},
};

#[derive(Clone, Copy)]
struct LoopContext {
    continue_target: BasicBlockId,
    continue_locals: usize,
    break_target: BasicBlockId,
    break_locals: usize,
}

pub struct Lowering<'a> {
    ast: &'a ast::AstNodes,
    tcx: TyCtxt,
    body: Body,
    current_block: BasicBlockId,
    function_worklist: VecDeque<SymbolId>,
    loop_contexts: Vec<LoopContext>,
    local_stack: Vec<LocalId>,
    local_symbols: FxHashMap<SymbolId, LocalId>,
}

impl<'a> Lowering<'a> {
    pub fn new(ast: &'a ast::AstNodes, tcx: TyCtxt) -> Self {
        Self {
            ast,
            tcx,
            body: Body::new(),
            current_block: BasicBlockId::MAX,
            function_worklist: VecDeque::new(),
            loop_contexts: Vec::new(),
            local_stack: Vec::new(),
            local_symbols: FxHashMap::default(),
        }
    }

    pub fn lower(mut self, stmts: &[ast::StmtId]) -> Body {
        self.body.entry = self.create_block();
        self.current_block = self.body.entry;
        for &stmt in stmts {
            self.lower_stmt(stmt);
        }
        self.drop_locals(0);

        while let Some(symbol) = self.function_worklist.pop_front() {
            if self.body.functions.contains_key(&symbol) {
                continue;
            }
            let function = self.tcx.bcx.symbols[symbol].declaration;
            let prev_body = std::mem::replace(&mut self.body, Body::new());
            self.local_symbols = FxHashMap::default();
            self.local_stack = Vec::new();
            self.body.entry = self.create_block();
            self.current_block = self.body.entry;
            self.lower_function(function);
            let function = std::mem::replace(&mut self.body, prev_body);
            self.body.functions.insert(symbol, function);
        }

        self.body
    }

    fn lower_function(&mut self, function: ast::StmtId) {
        let fn_symbol = self.ast.stmts.value_binding(function);
        let fn_ty = self.tcx.value_bindings[&fn_symbol];
        let Ty::Fn(params, return_ty) = self.tcx.types.probe_value(fn_ty) else {
            unreachable!();
        };
        let result_local = self.create_local(return_ty);

        match &self.ast.stmts[function] {
            ast::Stmt::Fn(f) => {
                for &p in f.params.iter() {
                    self.lower_stmt(p);
                }
                self.lower_expr(f.body, Some(Place::local(result_local)));
            }
            ast::Stmt::TupleStruct(_) => {
                let fields = params
                    .iter()
                    .map(|&p| {
                        let p = self.create_local(p);
                        Operand::Place(Place::local(p))
                    })
                    .collect();
                self.push_stmt(Statement::Assign {
                    place: Place::local(result_local),
                    value: RValue::Struct { fields },
                });
            }
            _ => unreachable!(),
        }
        self.drop_locals(1);
    }

    fn lower_stmt(&mut self, stmt: ast::StmtId) {
        match self.ast.stmts[stmt] {
            ast::Stmt::Let { init, .. } => {
                let local = self.lower_expr_to_local(init);
                let symbol = self.ast.stmts.value_binding(stmt);
                self.local_symbols.insert(symbol, local);
            }
            ast::Stmt::Expr { expr, .. } => {
                self.lower_expr(expr, None);
            }
            ast::Stmt::Param { .. } => {
                let symbol = self.ast.stmts.value_binding(stmt);
                let ty = self.tcx.value_bindings[&symbol];
                let local = self.create_local(ty);
                self.local_symbols.insert(symbol, local);
            }
            _ => {}
        }
    }

    fn lower_expr(&mut self, expr: ast::ExprId, place: Option<Place>) -> Operand {
        crate::stack_safe(|| self.lower_expr_unsafe(expr, place))
    }

    fn lower_expr_unsafe(&mut self, expr: ast::ExprId, place: Option<Place>) -> Operand {
        let ty = self.tcx.expr_types[&expr];
        let value = match self.ast.exprs[expr] {
            ast::Expr::Bool(x) => {
                return self.constant_operand(place, Constant::Bool(x));
            }
            ast::Expr::Number(x) => {
                return self.constant_operand(place, Constant::F64(x));
            }
            ast::Expr::Int(x) => {
                let constant = if self.tcx.types.probe_value(ty) == Ty::U64 {
                    Constant::U64(x as u64)
                } else {
                    Constant::I64(x as i64)
                };
                return self.constant_operand(place, constant);
            }
            ast::Expr::String(s) => {
                return self.constant_operand(place, Constant::String(s));
            }
            ast::Expr::Binary { lhs, op, rhs } => {
                let lhs = self.lower_expr(lhs, None);
                let rhs = self.lower_expr(rhs, None);
                RValue::Binary { op, lhs, rhs }
            }
            ast::Expr::Not(e) => {
                let operand = self.lower_expr(e, None);
                RValue::Not(operand)
            }
            ast::Expr::Negate(e) => {
                let operand = self.lower_expr(e, None);
                RValue::Neg(operand)
            }
            ast::Expr::Call { callee, ref args } => {
                let symbol = self.ast.exprs.value_binding(callee);
                if symbol == self.tcx.bcx.std.print {
                    let arg = self.lower_expr(args[0], None);
                    self.push_stmt(Statement::Print(arg));
                    return Operand::Constant(Constant::Unit);
                }

                let args = args.iter().map(|x| self.lower_expr(*x, None)).collect();
                let callee = self.lower_expr(callee, None);
                RValue::Call { callee, args }
            }
            ast::Expr::Array(ref args) => {
                let args = args.iter().map(|x| self.lower_expr(*x, None)).collect();
                RValue::Struct { fields: args }
            }
            ast::Expr::ArrayRepeat { expr, count } => {
                let value = self.lower_expr(expr, None);
                let count = self.lower_expr(count, None);
                RValue::StructRepeat { value, count }
            }
            ast::Expr::Struct(ref s) => {
                let args = s
                    .fields
                    .values()
                    .map(|x| self.lower_expr(*x, None))
                    .collect();
                RValue::Struct { fields: args }
            }
            ast::Expr::Tuple(ref args) => {
                let args = args.iter().map(|x| self.lower_expr(*x, None)).collect();
                RValue::Struct { fields: args }
            }
            ast::Expr::StructField { obj, name } => {
                let obj_ty = self.tcx.expr_types[&obj];
                let local = self.lower_expr_to_local(obj);
                let Ty::Named(symbol) = self.tcx.types.probe_value(obj_ty) else {
                    unreachable!();
                };
                let properties = &self.tcx.struct_properties[&symbol];
                let index = properties.get_index_of(&name).unwrap();
                return Operand::Place(Place {
                    local,
                    projection: Some(Projection::Field(index as u16)),
                });
            }
            ast::Expr::TupleField { obj, index } => {
                let local = self.lower_expr_to_local(obj);
                return Operand::Place(Place {
                    local,
                    projection: Some(Projection::Field(index as u16)),
                });
            }
            ast::Expr::ArrayIndex { array, index } => {
                let array = self.lower_expr_to_local(array);
                let index = self.lower_expr_to_local(index);
                return Operand::Place(Place {
                    local: array,
                    projection: Some(Projection::Index(index)),
                });
            }
            ast::Expr::Block { ref stmts, expr } => {
                if stmts.is_empty() {
                    return match expr {
                        Some(expr) => self.lower_expr(expr, place),
                        None => self.constant_operand(place, Constant::Unit),
                    };
                }

                let prev_locals = self.local_stack.len();
                for &stmt in stmts {
                    self.lower_stmt(stmt);
                }
                let value = if let Some(expr) = expr {
                    return self.lower_expr(expr, place);
                } else {
                    self.constant_operand(place, Constant::Unit)
                };
                self.drop_locals(prev_locals);
                return value;
            }
            ast::Expr::Variable(..) => {
                let symbol_id = self.ast.exprs.value_binding(expr);
                let symbol = &self.tcx.bcx.symbols[symbol_id];
                return match symbol.kind {
                    SymbolKind::Let | SymbolKind::Param => {
                        let local = self.local_symbols[&symbol_id];
                        self.store_in_place(place, Operand::Place(Place::local(local)))
                    }
                    SymbolKind::Fn => {
                        self.function_worklist.push_back(symbol_id);
                        self.constant_operand(place, Constant::Function(symbol_id))
                    }
                    _ => unreachable!(),
                };
            }
            ast::Expr::If {
                cond,
                then_clause,
                else_clause,
            } => {
                let cond = self.lower_expr(cond, None);

                let then_block = self.create_block();
                let else_block = else_clause.map(|_| self.create_block());
                let merge_block = self.create_block();

                self.set_terminator(Terminator::Branch {
                    cond,
                    then_block,
                    else_block: else_block.unwrap_or(merge_block),
                });
                self.add_edge(self.current_block, then_block);
                self.add_edge(self.current_block, else_block.unwrap_or(merge_block));

                self.current_block = then_block;
                self.lower_expr(then_clause, place);
                self.add_edge(self.current_block, merge_block);

                if let Some(else_clause) = else_clause
                    && let Some(else_block) = else_block
                {
                    self.current_block = else_block;
                    self.lower_expr(else_clause, place);
                    self.add_edge(self.current_block, merge_block);
                }

                self.current_block = merge_block;
                return place.map_or(Operand::Constant(Constant::Unit), |place| {
                    Operand::Place(place)
                });
            }
            ast::Expr::Assign { lhs, rhs } => {
                let lhs_place = match self.ast.exprs[lhs] {
                    ast::Expr::Variable(_) => {
                        let symbol = self.ast.exprs.value_binding(lhs);
                        let local = self.local_symbols[&symbol];
                        Place::local(local)
                    }
                    ast::Expr::TupleField { obj, index } => {
                        let obj = self.lower_expr_to_local(obj);
                        Place {
                            local: obj,
                            projection: Some(Projection::Field(index)),
                        }
                    }
                    ast::Expr::StructField { obj, name } => {
                        let ty = self.tcx.expr_types[&obj];
                        let Ty::Named(symbol) = self.tcx.types.probe_value(ty) else {
                            unreachable!();
                        };
                        let properties = &self.tcx.struct_properties[&symbol];
                        let index = properties.get_index_of(&name).unwrap();

                        let obj = self.lower_expr_to_local(obj);
                        Place {
                            local: obj,
                            projection: Some(Projection::Field(index as u16)),
                        }
                    }
                    ast::Expr::ArrayIndex { array, index } => {
                        let array = self.lower_expr_to_local(array);
                        let index = self.lower_expr_to_local(index);
                        Place {
                            local: array,
                            projection: Some(Projection::Index(index)),
                        }
                    }
                    _ => unreachable!(),
                };
                self.lower_expr(rhs, Some(lhs_place));
                return self.constant_operand(place, Constant::Unit);
            }
            ast::Expr::While { cond, body } => {
                let locals = self.local_stack.len();
                let cond_block = self.create_block();
                let body_block = self.create_block();
                let merge_block = self.create_block();

                self.loop_contexts.push(LoopContext {
                    continue_target: cond_block,
                    break_target: merge_block,
                    continue_locals: locals,
                    break_locals: locals,
                });

                self.add_edge(self.current_block, cond_block);
                self.current_block = cond_block;
                let cond = self.lower_expr(cond, None);

                self.set_terminator(Terminator::Branch {
                    cond,
                    then_block: body_block,
                    else_block: merge_block,
                });
                self.add_edge(self.current_block, body_block);
                self.add_edge(self.current_block, merge_block);

                self.current_block = body_block;
                self.lower_expr(body, None);
                self.set_terminator(Terminator::Jump { target: cond_block });
                self.add_edge(self.current_block, cond_block);

                self.current_block = merge_block;
                self.loop_contexts.pop();
                return self.constant_operand(place, Constant::Unit);
            }
            ast::Expr::For {
                init,
                cond,
                incr,
                body,
            } => {
                let prev_locals = self.local_stack.len();
                if let Some(init) = init {
                    self.lower_stmt(init);
                }
                let continue_locals = self.local_stack.len();
                let cond_block = cond.map(|_| self.create_block());
                let body_block = self.create_block();
                let incr_block = incr.map(|_| self.create_block());
                let merge_block = self.create_block();

                if let Some(cond) = cond
                    && let Some(cond_block) = cond_block
                {
                    self.add_edge(self.current_block, cond_block);
                    self.current_block = cond_block;
                    let cond = self.lower_expr(cond, None);
                    self.set_terminator(Terminator::Branch {
                        cond,
                        then_block: body_block,
                        else_block: merge_block,
                    });
                    self.add_edge(self.current_block, body_block);
                    self.add_edge(self.current_block, merge_block);
                } else {
                    self.add_edge(self.current_block, body_block);
                }

                self.current_block = body_block;
                let continue_target = incr_block.or(cond_block).unwrap_or(body_block);

                self.loop_contexts.push(LoopContext {
                    continue_target,
                    continue_locals,
                    break_target: merge_block,
                    break_locals: continue_locals,
                });

                self.lower_expr(body, None);
                self.drop_locals(continue_locals);
                self.set_terminator(Terminator::Jump {
                    target: continue_target,
                });
                self.add_edge(self.current_block, continue_target);

                if let Some(incr) = incr
                    && let Some(incr_block) = incr_block
                {
                    self.current_block = incr_block;
                    self.lower_expr(incr, None);
                    let incr_target = cond_block.unwrap_or(body_block);
                    self.set_terminator(Terminator::Jump {
                        target: incr_target,
                    });
                    self.add_edge(self.current_block, incr_target);
                }

                self.current_block = merge_block;
                self.loop_contexts.pop();
                self.drop_locals(prev_locals);
                return self.constant_operand(place, Constant::Unit);
            }
            ast::Expr::Break => {
                let context = *self.loop_contexts.last().unwrap();
                for i in (context.break_locals..self.local_stack.len()).rev() {
                    self.push_stmt(Statement::StorageDead(self.local_stack[i]));
                }
                self.set_terminator(Terminator::Jump {
                    target: context.break_target,
                });
                self.add_edge(self.current_block, context.break_target);

                self.current_block = self.create_block();
                return Operand::Constant(Constant::Unit);
            }
            ast::Expr::Continue => {
                let context = *self.loop_contexts.last().unwrap();
                for i in (context.continue_locals..self.local_stack.len()).rev() {
                    self.push_stmt(Statement::StorageDead(self.local_stack[i]));
                }
                self.set_terminator(Terminator::Jump {
                    target: context.continue_target,
                });
                self.add_edge(self.current_block, context.continue_target);

                self.current_block = self.create_block();
                return Operand::Constant(Constant::Unit);
            }
            ast::Expr::Return(expr) => {
                let value = if let Some(expr) = expr {
                    self.lower_expr(expr, None)
                } else {
                    Operand::Constant(Constant::Unit)
                };
                self.push_stmt(Statement::Assign {
                    place: Place::local(LocalId::RETURN),
                    value: RValue::Use(value),
                });
                // 0 is the RETURN slot
                for i in (1..self.local_stack.len()).rev() {
                    self.push_stmt(Statement::StorageDead(self.local_stack[i]));
                }
                return Operand::Constant(Constant::Unit);
            }
            ast::Expr::Unit => return self.constant_operand(place, Constant::Unit),
        };
        let place = place.unwrap_or_else(|| Place::local(self.create_local(ty)));
        self.push_stmt(Statement::Assign { place, value });
        Operand::Place(place)
    }

    fn lower_expr_to_local(&mut self, expr: ast::ExprId) -> LocalId {
        let ty = self.tcx.expr_types[&expr];
        let local = self.create_local(ty);
        self.lower_expr(expr, Some(Place::local(local)));
        local
    }

    fn constant_operand(&mut self, place: Option<Place>, value: Constant) -> Operand {
        self.store_in_place(place, Operand::Constant(value))
    }

    fn store_in_place(&mut self, place: Option<Place>, operand: Operand) -> Operand {
        if let Some(place) = place {
            self.push_stmt(Statement::Assign {
                place,
                value: RValue::Use(operand),
            });
        }
        operand
    }

    fn set_terminator(&mut self, terminator: Terminator) {
        self.body.basic_blocks[self.current_block].terminator = Some(terminator);
    }

    fn create_block(&mut self) -> BasicBlockId {
        self.body.basic_blocks.alloc(BasicBlock {
            statements: Vec::new(),
            terminator: None,
            successors: Vec::new(),
        })
    }

    fn create_local(&mut self, ty: TyId) -> LocalId {
        let local = self.body.locals.alloc(Local { ty });
        self.push_stmt(Statement::StorageLive(local));
        self.local_stack.push(local);
        local
    }

    fn add_edge(&mut self, from: BasicBlockId, to: BasicBlockId) {
        self.body.basic_blocks[from].successors.push(to);
    }

    fn push_stmt(&mut self, statement: Statement) {
        self.body.basic_blocks[self.current_block]
            .statements
            .push(statement);
    }

    fn drop_locals(&mut self, keep_len: usize) {
        while self.local_stack.len() > keep_len {
            let local = self.local_stack.pop().unwrap();
            self.push_stmt(Statement::StorageDead(local));
        }
    }
}

pub type BasicBlockId = Id<BasicBlock>;
pub type LocalId = Id<Local>;

impl LocalId {
    const RETURN: Self = Self::ZERO;
}

pub struct Body {
    pub basic_blocks: Arena<BasicBlock>,
    pub entry: BasicBlockId,
    pub locals: Arena<Local>,
    pub functions: FxHashMap<SymbolId, Body>,
}

impl Body {
    fn new() -> Self {
        Self {
            basic_blocks: Arena::new(),
            locals: Arena::new(),
            entry: BasicBlockId::MAX,
            functions: FxHashMap::default(),
        }
    }
}

pub struct Local {
    pub ty: TyId,
}

pub struct BasicBlock {
    pub statements: Vec<Statement>,
    pub terminator: Option<Terminator>,
    pub successors: Vec<BasicBlockId>,
}

pub enum Statement {
    Assign { place: Place, value: RValue },
    StorageLive(LocalId),
    StorageDead(LocalId),
    Print(Operand),
}

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

pub enum RValue {
    Use(Operand),
    Binary {
        op: ast::BinaryOp,
        lhs: Operand,
        rhs: Operand,
    },
    Not(Operand),
    Neg(Operand),
    Struct {
        fields: Vec<Operand>,
    },
    StructRepeat {
        value: Operand,
        count: Operand,
    },
    Call {
        callee: Operand,
        args: Vec<Operand>,
    },
}

#[derive(Clone, Copy)]
pub enum Operand {
    Place(Place),
    Constant(Constant),
}

#[derive(Clone, Copy)]
pub struct Place {
    pub local: LocalId,
    pub projection: Option<Projection>,
}

#[derive(Clone, Copy)]
pub enum Projection {
    Field(u16),
    Index(LocalId),
}

#[derive(Clone, Copy)]
pub enum Constant {
    Unit,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    String(StrId),
    Function(SymbolId),
}

impl Place {
    fn local(local: LocalId) -> Self {
        Self {
            local,
            projection: None,
        }
    }
}
