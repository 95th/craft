use rustc_hash::FxHashMap;

use crate::{
    arena::{Arena, Id},
    ast,
    bind::{SymbolId, SymbolKind},
    interner::StrId,
    typechecker::TyCtxt,
    types::{Ty, TyId},
};

pub struct Lowering<'a> {
    ast: &'a ast::AstNodes,
    tcx: TyCtxt,
    body: Body,
    current_block: BasicBlockId,
}

impl<'a> Lowering<'a> {
    pub fn new(ast: &'a ast::AstNodes, tcx: TyCtxt) -> Self {
        Self {
            ast,
            tcx,
            body: Body {
                basic_blocks: Arena::new(),
                locals: Arena::new(),
                local_symbols: FxHashMap::default(),
                entry: BasicBlockId::MAX,
            },
            current_block: BasicBlockId::MAX,
        }
    }

    pub fn lower(mut self, stmts: &[ast::StmtId]) -> Body {
        self.body.entry = self.create_block();
        self.current_block = self.body.entry;
        for &stmt in stmts {
            self.lower_stmt(stmt);
        }
        self.body
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
            ast::Expr::StructField { obj, name } => {
                let obj = self.lower_expr(obj);
                let Ty::Named(symbol) = self.tcx.types.probe_value(ty) else {
                    unreachable!();
                };
                let properties = &self.tcx.struct_properties[&symbol];
                let index = properties.get_index_of(&name).unwrap();
                RValue::Field {
                    obj,
                    index: index as u16,
                }
            }
            ast::Expr::TupleField { obj, index } => {
                let obj = self.lower_expr(obj);
                RValue::Field { obj, index }
            }
            ast::Expr::ArrayIndex { array, index } => {
                let array = self.lower_expr(array);
                let index = self.lower_expr(index);
                RValue::Index { array, index }
            }
            ast::Expr::Block { ref stmts, expr } => {
                for &stmt in stmts {
                    self.lower_stmt(stmt);
                }
                return if let Some(expr) = expr {
                    self.lower_expr(expr)
                } else {
                    Operand::Constant(Constant::Unit)
                };
            }
            ast::Expr::Variable(..) => {
                let symbol_id = self.ast.exprs.value_binding(expr);
                let symbol = &self.tcx.bcx.symbols[symbol_id];
                return match symbol.kind {
                    SymbolKind::Let | SymbolKind::Param => {
                        Operand::Place(Place::local(self.body.local_symbols[&symbol_id]))
                    }
                    SymbolKind::Fn => Operand::Constant(Constant::Function(symbol_id)),
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
                self.body.basic_blocks[self.current_block].terminator = Some(Terminator::Branch {
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
            ref other => todo!("{other:?}"),
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
}

pub type BasicBlockId = Id<BasicBlock>;
pub type LocalId = Id<Local>;

pub struct Body {
    basic_blocks: Arena<BasicBlock>,
    entry: BasicBlockId,
    locals: Arena<Local>,
    local_symbols: FxHashMap<SymbolId, LocalId>,
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
    Field {
        obj: Operand,
        index: u16,
    },
    Index {
        array: Operand,
        index: Operand,
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
