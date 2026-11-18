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

struct LoopContext {
    start: BasicBlockId,
    end: BasicBlockId,
}

pub struct Lowering<'a> {
    ast: &'a ast::AstNodes,
    tcx: TyCtxt,
    body: Body,
    current_block: BasicBlockId,
    function_worklist: VecDeque<SymbolId>,
    loop_contexts: Vec<LoopContext>,
    local_stack: Vec<LocalId>,
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
                let value = self.lower_expr(f.body);
                self.push_stmt(Statement::Assign {
                    place: Place::local(result_local),
                    value: RValue::Use(value),
                });
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
                self.body.local_symbols.insert(symbol, local);
            }
            ast::Stmt::Expr { expr, .. } => {
                self.lower_expr(expr);
            }
            ast::Stmt::Param { .. } => {
                let symbol = self.ast.stmts.value_binding(stmt);
                let ty = self.tcx.value_bindings[&symbol];
                let local = self.create_local(ty);
                self.body.local_symbols.insert(symbol, local);
            }
            _ => {}
        }
    }

    fn lower_expr(&mut self, expr: ast::ExprId) -> Operand {
        let ty = self.tcx.expr_types[&expr];
        let value = match self.ast.exprs[expr] {
            ast::Expr::Bool(x) => return Operand::Constant(Constant::Bool(x)),
            ast::Expr::Number(x) => return Operand::Constant(Constant::F64(x)),
            ast::Expr::Int(x) => {
                return if self.tcx.types.probe_value(ty) == Ty::U64 {
                    Operand::Constant(Constant::U64(x as u64))
                } else {
                    Operand::Constant(Constant::I64(x as i64))
                };
            }
            ast::Expr::String(s) => return Operand::Constant(Constant::String(s)),
            ast::Expr::Binary { lhs, op, rhs } => {
                let lhs = self.lower_expr(lhs);
                let rhs = self.lower_expr(rhs);
                RValue::Binary { op, lhs, rhs }
            }
            ast::Expr::Not(e) => {
                let operand = self.lower_expr(e);
                RValue::Not(operand)
            }
            ast::Expr::Negate(e) => {
                let operand = self.lower_expr(e);
                RValue::Neg(operand)
            }
            ast::Expr::Call { callee, ref args } => {
                let symbol = self.ast.exprs.value_binding(callee);
                if symbol == self.tcx.bcx.std.print {
                    let arg = self.lower_expr(args[0]);
                    self.push_stmt(Statement::Print(arg));
                    return Operand::Constant(Constant::Unit);
                }

                let args = args.iter().map(|x| self.lower_expr(*x)).collect();
                let callee = self.lower_expr(callee);
                RValue::Call { callee, args }
            }
            ast::Expr::Array(ref args) => {
                let args = args.iter().map(|x| self.lower_expr(*x)).collect();
                RValue::Struct { fields: args }
            }
            ast::Expr::ArrayRepeat { expr, count } => {
                let value = self.lower_expr(expr);
                let count = self.lower_expr(count);
                RValue::StructRepeat { value, count }
            }
            ast::Expr::Struct(ref s) => {
                let args = s.fields.values().map(|x| self.lower_expr(*x)).collect();
                RValue::Struct { fields: args }
            }
            ast::Expr::Tuple(ref args) => {
                let args = args.iter().map(|x| self.lower_expr(*x)).collect();
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
                let result_local = self.create_local(ty);
                let prev_locals = self.local_stack.len();
                for &stmt in stmts {
                    self.lower_stmt(stmt);
                }
                let value = if let Some(expr) = expr {
                    self.lower_expr(expr)
                } else {
                    Operand::Constant(Constant::Unit)
                };
                self.push_stmt(Statement::Assign {
                    place: Place::local(result_local),
                    value: RValue::Use(value),
                });
                self.drop_locals(prev_locals);
                return Operand::Place(Place::local(result_local));
            }
            ast::Expr::Variable(..) => {
                let symbol_id = self.ast.exprs.value_binding(expr);
                let symbol = &self.tcx.bcx.symbols[symbol_id];
                return match symbol.kind {
                    SymbolKind::Let | SymbolKind::Param => {
                        Operand::Place(Place::local(self.body.local_symbols[&symbol_id]))
                    }
                    SymbolKind::Fn => {
                        self.function_worklist.push_back(symbol_id);
                        Operand::Constant(Constant::Function(symbol_id))
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
                let result_local = self.create_local(ty);

                let then_block = self.create_block();
                let else_block = self.create_block();
                let merge_block = self.create_block();
                self.set_terminator(Terminator::Branch {
                    cond,
                    then_block,
                    else_block,
                });
                self.add_edge(self.current_block, then_block);
                self.add_edge(self.current_block, else_block);

                self.current_block = then_block;
                let then_value = self.lower_expr(then_clause);
                self.push_stmt(Statement::Assign {
                    place: Place::local(result_local),
                    value: RValue::Use(then_value),
                });
                if self.body.basic_blocks[self.current_block]
                    .terminator
                    .is_none()
                {
                    self.add_edge(self.current_block, merge_block);
                }

                self.current_block = else_block;
                if let Some(else_clause) = else_clause {
                    let else_value = self.lower_expr(else_clause);
                    self.push_stmt(Statement::Assign {
                        place: Place::local(result_local),
                        value: RValue::Use(else_value),
                    });
                }
                if self.body.basic_blocks[self.current_block]
                    .terminator
                    .is_none()
                {
                    self.add_edge(self.current_block, merge_block);
                }

                self.current_block = merge_block;
                return Operand::Place(Place::local(result_local));
            }
            ast::Expr::Assign { lhs, rhs } => {
                let place = match self.ast.exprs[lhs] {
                    ast::Expr::Variable(_) => {
                        let symbol = self.ast.exprs.value_binding(lhs);
                        let local = self.body.local_symbols[&symbol];
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
                let value = self.lower_expr(rhs);
                self.push_stmt(Statement::Assign {
                    place,
                    value: RValue::Use(value),
                });
                return Operand::Constant(Constant::Unit);
            }
            ast::Expr::While { cond, body } => {
                let cond_block = self.create_block();
                let body_block = self.create_block();
                let merge_block = self.create_block();

                self.loop_contexts.push(LoopContext {
                    start: cond_block,
                    end: merge_block,
                });

                self.add_edge(self.current_block, cond_block);
                self.current_block = cond_block;
                let cond = self.lower_expr(cond);

                self.set_terminator(Terminator::Branch {
                    cond,
                    then_block: body_block,
                    else_block: merge_block,
                });
                self.add_edge(self.current_block, body_block);
                self.add_edge(self.current_block, merge_block);

                self.current_block = body_block;
                self.lower_expr(body);
                self.set_terminator(Terminator::Jump { target: cond_block });
                self.add_edge(self.current_block, cond_block);

                self.current_block = merge_block;
                self.loop_contexts.pop();
                return Operand::Constant(Constant::Unit);
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
                let cond_block = self.create_block();
                let body_block = self.create_block();
                let merge_block = self.create_block();

                self.loop_contexts.push(LoopContext {
                    start: cond_block,
                    end: merge_block,
                });

                let terminator = if let Some(cond) = cond {
                    self.add_edge(self.current_block, cond_block);
                    self.current_block = cond_block;
                    let cond = self.lower_expr(cond);
                    Terminator::Branch {
                        cond,
                        then_block: body_block,
                        else_block: merge_block,
                    }
                } else {
                    Terminator::Jump { target: body_block }
                };
                self.set_terminator(terminator);
                self.add_edge(self.current_block, body_block);
                self.add_edge(self.current_block, merge_block);

                self.current_block = body_block;
                self.lower_expr(body);
                if let Some(incr) = incr {
                    self.lower_expr(incr);
                }
                self.set_terminator(Terminator::Jump { target: cond_block });
                self.add_edge(self.current_block, cond_block);

                self.current_block = merge_block;
                self.loop_contexts.pop();
                self.drop_locals(prev_locals);
                return Operand::Constant(Constant::Unit);
            }
            ast::Expr::Break => {
                let loop_end = self.loop_contexts.last().unwrap().end;
                self.set_terminator(Terminator::Jump { target: loop_end });

                self.current_block = self.create_block();
                return Operand::Constant(Constant::Unit);
            }
            ast::Expr::Continue => {
                let loop_start = self.loop_contexts.last().unwrap().start;
                self.set_terminator(Terminator::Jump { target: loop_start });

                self.current_block = self.create_block();
                return Operand::Constant(Constant::Unit);
            }
            ast::Expr::Return(expr) => {
                let value = if let Some(expr) = expr {
                    self.lower_expr(expr)
                } else {
                    Operand::Constant(Constant::Unit)
                };
                self.push_stmt(Statement::Assign {
                    place: Place::local(LocalId::RETURN),
                    value: RValue::Use(value),
                });
                return Operand::Constant(Constant::Unit);
            }
            ast::Expr::Unit => return Operand::Constant(Constant::Unit),
        };
        let local = self.create_local(ty);
        self.push_stmt(Statement::Assign {
            place: Place::local(local),
            value,
        });
        Operand::Place(Place::local(local))
    }

    fn lower_expr_to_local(&mut self, expr: ast::ExprId) -> LocalId {
        let value = self.lower_expr(expr);
        match value {
            Operand::Place(Place {
                local,
                projection: None,
            }) => local,
            _ => {
                let ty = self.tcx.expr_types[&expr];
                let local = self.create_local(ty);
                self.push_stmt(Statement::Assign {
                    place: Place::local(local),
                    value: RValue::Use(value),
                });
                local
            }
        }
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
    basic_blocks: Arena<BasicBlock>,
    entry: BasicBlockId,
    locals: Arena<Local>,
    local_symbols: FxHashMap<SymbolId, LocalId>,
    functions: FxHashMap<SymbolId, Body>,
}

impl Body {
    fn new() -> Self {
        Self {
            basic_blocks: Arena::new(),
            locals: Arena::new(),
            local_symbols: FxHashMap::default(),
            entry: BasicBlockId::MAX,
            functions: FxHashMap::default(),
        }
    }
}

pub struct Local {
    ty: TyId,
}

pub struct BasicBlock {
    statements: Vec<Statement>,
    terminator: Option<Terminator>,
    successors: Vec<BasicBlockId>,
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

pub enum Operand {
    Place(Place),
    Constant(Constant),
}

pub struct Place {
    local: LocalId,
    projection: Option<Projection>,
}

pub enum Projection {
    Field(u16),
    Index(LocalId),
}

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
