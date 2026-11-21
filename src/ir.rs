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
    break_target: BasicBlockId,
}

pub struct Lowering<'a> {
    ast: &'a ast::AstNodes,
    tcx: &'a mut TyCtxt,
    basic_blocks: Arena<BasicBlock>,
    next_register: RegId,
    current_block: BasicBlockId,
    function_worklist: VecDeque<SymbolId>,
    loop_contexts: Vec<LoopContext>,
    symbol_registers: FxHashMap<SymbolId, RegId>,
    return_reg: RegId,
}

impl<'a> Lowering<'a> {
    pub fn new(ast: &'a ast::AstNodes, tcx: &'a mut TyCtxt) -> Self {
        Self {
            ast,
            tcx,
            basic_blocks: Arena::new(),
            next_register: 0,
            current_block: BasicBlockId::MAX,
            function_worklist: VecDeque::new(),
            loop_contexts: Vec::new(),
            symbol_registers: FxHashMap::default(),
            return_reg: RegId::MAX,
        }
    }

    pub fn lower(mut self, stmts: &[ast::StmtId]) -> Ir {
        let main = self.create_block();
        self.current_block = main;
        for &stmt in stmts {
            self.lower_stmt(stmt);
        }
        self.set_terminator(Terminator::Return);

        let mut functions = FxHashMap::default();
        while let Some(symbol) = self.function_worklist.pop_front() {
            if functions.contains_key(&symbol) {
                continue;
            }
            let function = self.tcx.bcx.symbols[symbol].declaration;
            self.next_register = 0;
            let entry = self.create_block();
            functions.insert(symbol, entry);
            self.current_block = entry;
            self.lower_function(function);
        }

        Ir {
            main,
            functions,
            basic_blocks: self.basic_blocks,
        }
    }

    fn lower_function(&mut self, function: ast::StmtId) {
        self.return_reg = self.create_register(true);
        match &self.ast.stmts[function] {
            ast::Stmt::Fn(f) => {
                for &p in f.params.iter() {
                    self.lower_stmt(p);
                }
                self.lower_expr(f.body, Some(Place::reg(self.return_reg)));
            }
            ast::Stmt::TupleStruct(args) => {
                let fields = args
                    .fields
                    .iter()
                    .map(|_| Operand::Place(Place::reg(self.create_register(true))))
                    .collect();
                self.push_stmt(Statement::Assign {
                    place: Place::reg(self.return_reg),
                    value: RValue::Struct { fields },
                });
            }
            _ => unreachable!(),
        }
        self.set_terminator(Terminator::Return);
    }

    fn lower_stmt(&mut self, stmt: ast::StmtId) {
        match self.ast.stmts[stmt] {
            ast::Stmt::Let { init, .. } => {
                let reg = self.lower_expr_to_reg(init);
                let symbol = self.ast.stmts.value_binding(stmt);
                self.symbol_registers.insert(symbol, reg);
            }
            ast::Stmt::Expr { expr, .. } => {
                self.lower_expr(expr, None);
            }
            ast::Stmt::Param { .. } => {
                let symbol = self.ast.stmts.value_binding(stmt);
                let reg = self.create_register(true);
                self.symbol_registers.insert(symbol, reg);
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
            ast::Expr::Binary {
                lhs,
                op: ast::BinaryOp::Or,
                rhs,
            } => {
                let place = place.unwrap_or_else(|| Place::reg(self.create_register(false)));
                self.lower_expr(lhs, Some(place));

                let merge_block = self.create_block();
                let false_block = self.create_block();
                self.set_terminator(Terminator::Branch {
                    cond: Operand::Place(place),
                    then_block: merge_block,
                    else_block: false_block,
                });
                self.add_edge(self.current_block, merge_block);
                self.add_edge(self.current_block, false_block);

                self.current_block = false_block;
                self.lower_expr(rhs, Some(place));
                self.set_terminator(Terminator::Jump {
                    target: merge_block,
                });
                self.add_edge(self.current_block, merge_block);

                self.current_block = merge_block;
                return Operand::Place(place);
            }
            ast::Expr::Binary {
                lhs,
                op: ast::BinaryOp::And,
                rhs,
            } => {
                let place = place.unwrap_or_else(|| Place::reg(self.create_register(false)));
                self.lower_expr(lhs, Some(place));

                let merge_block = self.create_block();
                let true_block = self.create_block();
                self.set_terminator(Terminator::Branch {
                    cond: Operand::Place(place),
                    then_block: true_block,
                    else_block: merge_block,
                });
                self.add_edge(self.current_block, true_block);
                self.add_edge(self.current_block, merge_block);

                self.current_block = true_block;
                self.lower_expr(rhs, Some(place));
                self.set_terminator(Terminator::Jump {
                    target: merge_block,
                });
                self.add_edge(self.current_block, merge_block);

                self.current_block = merge_block;
                return Operand::Place(place);
            }
            ast::Expr::Binary { lhs, op, rhs } => {
                let lhs_ty = self.tcx.expr_types[&lhs];
                let op = BinaryOp::from_ast(op, self.tcx.types.probe_value(lhs_ty));
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
                match self.tcx.types.probe_value(ty) {
                    Ty::F64 => RValue::F64Neg(operand),
                    Ty::I64 => RValue::I64Neg(operand),
                    _ => unreachable!(),
                }
            }
            ast::Expr::Call { callee, ref args } => {
                let symbol = self.ast.exprs.value_binding(callee);
                if symbol == self.tcx.bcx.std.print {
                    let arg = self.lower_expr(args[0], None);
                    let arg_ty = self.tcx.expr_types[&args[0]];
                    self.push_stmt(Statement::Print(arg, arg_ty));
                    return self.constant_operand(place, Constant::Unit);
                }

                let args = args.iter().map(|x| self.lower_expr(*x, None)).collect();
                if symbol != SymbolId::MAX
                    && let decl = self.tcx.bcx.symbols[symbol].declaration
                    && let ast::Stmt::TupleStruct(_) = &self.ast.stmts[decl]
                {
                    RValue::Struct { fields: args }
                } else {
                    let callee = self.lower_expr(callee, None);
                    RValue::Call { callee, args }
                }
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
                let symbol = self.ast.exprs.value_binding(expr);
                let declared = self.tcx.struct_properties[&symbol].clone();
                let args = declared
                    .keys()
                    .map(|name| self.lower_expr(s.fields[name], None))
                    .collect();
                RValue::Struct { fields: args }
            }
            ast::Expr::Tuple(ref args) => {
                let args = args.iter().map(|x| self.lower_expr(*x, None)).collect();
                RValue::Struct { fields: args }
            }
            ast::Expr::StructField { obj, name } => {
                let obj_ty = self.tcx.expr_types[&obj];
                let reg = self.lower_expr_to_reg(obj);
                let Ty::Named(symbol) = self.tcx.types.probe_value(obj_ty) else {
                    unreachable!();
                };
                let properties = &self.tcx.struct_properties[&symbol];
                let index = properties.get_index_of(&name).unwrap();
                return self.store_in_place(
                    place,
                    Operand::Place(Place {
                        reg,
                        projection: Projection::Field(index as u16),
                    }),
                );
            }
            ast::Expr::TupleField { obj, index } => {
                let reg = self.lower_expr_to_reg(obj);
                return self.store_in_place(
                    place,
                    Operand::Place(Place {
                        reg,
                        projection: Projection::Field(index as u16),
                    }),
                );
            }
            ast::Expr::ArrayIndex { array, index } => {
                let array = self.lower_expr_to_reg(array);
                let index = self.lower_expr_to_reg(index);
                return self.store_in_place(
                    place,
                    Operand::Place(Place {
                        reg: array,
                        projection: Projection::Index(index),
                    }),
                );
            }
            ast::Expr::Block { ref stmts, expr } => {
                if stmts.is_empty() {
                    return match expr {
                        Some(expr) => self.lower_expr(expr, place),
                        None => self.constant_operand(place, Constant::Unit),
                    };
                }

                for &stmt in stmts {
                    self.lower_stmt(stmt);
                }
                return if let Some(expr) = expr {
                    self.lower_expr(expr, place)
                } else {
                    self.constant_operand(place, Constant::Unit)
                };
            }
            ast::Expr::Variable(..) => {
                let symbol_id = self.ast.exprs.value_binding(expr);
                let symbol = &self.tcx.bcx.symbols[symbol_id];
                return match symbol.kind {
                    SymbolKind::Let | SymbolKind::Param => {
                        let reg = self.symbol_registers[&symbol_id];
                        self.store_in_place(place, Operand::Place(Place::reg(reg)))
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
                        let reg = self.symbol_registers[&symbol];
                        Place::reg(reg)
                    }
                    ast::Expr::TupleField { obj, index } => {
                        let obj = self.lower_expr_to_reg(obj);
                        Place {
                            reg: obj,
                            projection: Projection::Field(index),
                        }
                    }
                    ast::Expr::StructField { obj, name } => {
                        let ty = self.tcx.expr_types[&obj];
                        let Ty::Named(symbol) = self.tcx.types.probe_value(ty) else {
                            unreachable!();
                        };
                        let properties = &self.tcx.struct_properties[&symbol];
                        let index = properties.get_index_of(&name).unwrap();

                        let obj = self.lower_expr_to_reg(obj);
                        Place {
                            reg: obj,
                            projection: Projection::Field(index as u16),
                        }
                    }
                    ast::Expr::ArrayIndex { array, index } => {
                        let array = self.lower_expr_to_reg(array);
                        let index = self.lower_expr_to_reg(index);
                        Place {
                            reg: array,
                            projection: Projection::Index(index),
                        }
                    }
                    _ => unreachable!(),
                };
                self.lower_expr(rhs, Some(lhs_place));
                return self.constant_operand(place, Constant::Unit);
            }
            ast::Expr::While { cond, body } => {
                let body_block = self.create_block();
                let cond_block = self.create_block();
                let merge_block = self.create_block();
                let cond_reg = self.create_register(false);

                let cond_operand = self.lower_expr(cond, Some(Place::reg(cond_reg)));
                self.set_terminator(Terminator::Branch {
                    cond: cond_operand,
                    then_block: body_block,
                    else_block: merge_block,
                });

                self.add_edge(self.current_block, body_block);
                self.add_edge(self.current_block, merge_block);

                self.current_block = body_block;
                self.enter_loop(cond_block, merge_block);
                self.lower_expr(body, None);
                self.loop_contexts.pop();

                self.add_edge(self.current_block, cond_block);

                self.current_block = cond_block;
                let cond_operand = self.lower_expr(cond, Some(Place::reg(cond_reg)));
                self.set_terminator(Terminator::Branch {
                    cond: cond_operand,
                    then_block: body_block,
                    else_block: merge_block,
                });
                self.add_edge(self.current_block, body_block);
                self.add_edge(self.current_block, merge_block);

                self.current_block = merge_block;
                return self.constant_operand(place, Constant::Unit);
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

                let cond_reg = self.create_register(false);

                if let Some(cond) = cond {
                    let cond = self.lower_expr(cond, Some(Place::reg(cond_reg)));
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
                self.enter_loop(incr_block, merge_block);
                self.lower_expr(body, None);
                self.loop_contexts.pop();
                self.add_edge(self.current_block, incr_block);

                self.current_block = incr_block;
                if let Some(incr) = incr {
                    self.lower_expr(incr, None);
                }
                if let Some(cond) = cond {
                    let cond = self.lower_expr(cond, Some(Place::reg(cond_reg)));
                    self.set_terminator(Terminator::Branch {
                        cond,
                        then_block: body_block,
                        else_block: merge_block,
                    });
                    self.add_edge(self.current_block, body_block);
                    self.add_edge(self.current_block, merge_block);
                } else {
                    self.set_terminator(Terminator::Jump { target: body_block });
                    self.add_edge(self.current_block, body_block);
                }

                self.current_block = merge_block;
                return self.constant_operand(place, Constant::Unit);
            }
            ast::Expr::Break => {
                let context = *self.loop_contexts.last().unwrap();
                self.set_terminator(Terminator::Jump {
                    target: context.break_target,
                });
                self.add_edge(self.current_block, context.break_target);

                self.current_block = self.create_block();
                return self.constant_operand(place, Constant::Unit);
            }
            ast::Expr::Continue => {
                let context = *self.loop_contexts.last().unwrap();
                self.set_terminator(Terminator::Jump {
                    target: context.continue_target,
                });
                self.add_edge(self.current_block, context.continue_target);

                self.current_block = self.create_block();
                return self.constant_operand(place, Constant::Unit);
            }
            ast::Expr::Return(expr) => {
                let value = if let Some(expr) = expr {
                    self.lower_expr(expr, None)
                } else {
                    Operand::Constant(Constant::Unit)
                };
                self.push_stmt(Statement::Assign {
                    place: Place::reg(self.return_reg),
                    value: RValue::Use(value),
                });
                self.set_terminator(Terminator::Return);
                self.current_block = self.create_block();
                return self.constant_operand(place, Constant::Unit);
            }
            ast::Expr::Unit => return self.constant_operand(place, Constant::Unit),
        };
        let place = place.unwrap_or_else(|| Place::reg(self.create_register(false)));
        self.push_stmt(Statement::Assign { place, value });
        Operand::Place(place)
    }

    fn lower_expr_to_reg(&mut self, expr: ast::ExprId) -> RegId {
        let reg = self.create_register(false);
        self.lower_expr(expr, Some(Place::reg(reg)));
        reg
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
        if cfg!(feature = "trace") {
            println!("{:?} => {:?}", self.current_block, terminator);
        }
        self.basic_blocks[self.current_block].terminator = Some(terminator);
    }

    fn create_block(&mut self) -> BasicBlockId {
        self.basic_blocks.alloc(BasicBlock {
            statements: Vec::new(),
            terminator: None,
            successors: Vec::new(),
        })
    }

    fn create_register(&mut self, implicit: bool) -> RegId {
        let reg = self.next_register;
        self.next_register += 1;
        self.push_stmt(Statement::StorageLive { reg, implicit });
        reg
    }

    fn add_edge(&mut self, from: BasicBlockId, to: BasicBlockId) {
        self.basic_blocks[from].successors.push(to);
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
}

pub type BasicBlockId = Id<BasicBlock>;
pub type RegId = u32;

pub struct Ir {
    pub main: BasicBlockId,
    pub functions: FxHashMap<SymbolId, BasicBlockId>,
    pub basic_blocks: Arena<BasicBlock>,
}

pub struct BasicBlock {
    pub statements: Vec<Statement>,
    pub terminator: Option<Terminator>,
    pub successors: Vec<BasicBlockId>,
}

#[derive(Debug)]
pub enum Statement {
    Assign { place: Place, value: RValue },
    StorageLive { reg: RegId, implicit: bool },
    Print(Operand, TyId),
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
    Binary {
        op: BinaryOp,
        lhs: Operand,
        rhs: Operand,
    },
    Not(Operand),
    F64Neg(Operand),
    I64Neg(Operand),
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

#[derive(Debug, Clone, Copy)]
pub enum Operand {
    Place(Place),
    Constant(Constant),
}

#[derive(Debug, Clone, Copy)]
pub struct Place {
    pub reg: RegId,
    pub projection: Projection,
}

#[derive(Debug, Clone, Copy)]
pub enum Projection {
    None,
    Field(u16),
    Index(RegId),
}

#[derive(Debug, Clone, Copy)]
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
    fn reg(reg: RegId) -> Self {
        Self {
            reg,
            projection: Projection::None,
        }
    }
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
