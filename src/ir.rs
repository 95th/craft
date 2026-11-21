use std::collections::VecDeque;

use rustc_hash::{FxHashMap, FxHashSet};

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
    reg_count: FxHashMap<BasicBlockId, RegId>,
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
            reg_count: FxHashMap::default(),
        }
    }

    pub fn lower(mut self, stmts: &[ast::StmtId]) -> Ir {
        let main = self.create_block();
        self.current_block = main;
        for &stmt in stmts {
            self.lower_stmt(stmt);
        }
        self.reg_count.insert(main, self.next_register);

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
            self.reg_count.insert(entry, self.next_register);
        }

        Ir {
            main,
            functions,
            basic_blocks: self.basic_blocks,
            reg_count: self.reg_count,
        }
    }

    fn lower_function(&mut self, function: ast::StmtId) {
        self.return_reg = self.create_register(true);
        match &self.ast.stmts[function] {
            ast::Stmt::Fn(f) => {
                for &p in f.params.iter() {
                    self.lower_stmt(p);
                }
                self.lower_expr_into(self.return_reg, f.body);
            }
            ast::Stmt::TupleStruct(args) => {
                let fields = args
                    .fields
                    .iter()
                    .map(|_| self.create_register(true))
                    .collect();
                self.push_stmt(Statement::Assign {
                    reg: self.return_reg,
                    value: RValue::Struct { fields },
                });
            }
            _ => unreachable!(),
        }
    }

    fn lower_stmt(&mut self, stmt: ast::StmtId) {
        match self.ast.stmts[stmt] {
            ast::Stmt::Let { init, .. } => {
                let reg = self.lower_expr(init);
                let symbol = self.ast.stmts.value_binding(stmt);
                self.symbol_registers.insert(symbol, reg);
            }
            ast::Stmt::Expr { expr, .. } => {
                self.lower_expr(expr);
            }
            ast::Stmt::Param { .. } => {
                let symbol = self.ast.stmts.value_binding(stmt);
                let reg = self.create_register(true);
                self.symbol_registers.insert(symbol, reg);
            }
            _ => {}
        }
    }

    fn lower_expr(&mut self, expr: ast::ExprId) -> RegId {
        match self.ast.exprs[expr] {
            ast::Expr::Variable(..) => {
                let symbol_id = self.ast.exprs.value_binding(expr);
                let symbol = &self.tcx.bcx.symbols[symbol_id];
                match symbol.kind {
                    SymbolKind::Let | SymbolKind::Param => self.symbol_registers[&symbol_id],
                    SymbolKind::Fn => {
                        self.function_worklist.push_back(symbol_id);
                        let dest = self.create_register(false);
                        self.store_constant(dest, Constant::Function(symbol_id));
                        dest
                    }
                    _ => unreachable!(),
                }
            }
            _ => {
                let dest = self.create_register(false);
                self.lower_expr_into(dest, expr);
                dest
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
                let constant = if self.tcx.types.probe_value(ty) == Ty::U64 {
                    Constant::U64(x as u64)
                } else {
                    Constant::I64(x as i64)
                };
                self.store_constant(dest, constant);
            }
            ast::Expr::String(s) => {
                self.store_constant(dest, Constant::String(s));
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
                    cond: dest,
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
                    cond: dest,
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
                let op = BinaryOp::from_ast(op, self.tcx.types.probe_value(lhs_ty));
                let lhs = self.lower_expr(lhs);
                let rhs = self.lower_expr(rhs);
                self.store_value(dest, RValue::Binary { op, lhs, rhs });
            }
            ast::Expr::Not(e) => {
                let operand = self.lower_expr(e);
                self.store_value(dest, RValue::Not(operand))
            }
            ast::Expr::Negate(e) => {
                let operand = self.lower_expr(e);
                let value = match self.tcx.types.probe_value(ty) {
                    Ty::F64 => RValue::F64Neg(operand),
                    Ty::I64 => RValue::I64Neg(operand),
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

                if symbol != SymbolId::MAX
                    && let decl = self.tcx.bcx.symbols[symbol].declaration
                    && let ast::Stmt::TupleStruct(_) = &self.ast.stmts[decl]
                {
                    let args = args.iter().map(|x| self.lower_expr(*x)).collect();
                    self.store_value(dest, RValue::Struct { fields: args })
                } else {
                    let callee = self.lower_expr(callee);
                    let args = args.iter().map(|x| self.lower_expr(*x)).collect();
                    self.store_value(dest, RValue::Call { callee, args })
                }
            }
            ast::Expr::Array(ref args) => {
                let args = args.iter().map(|x| self.lower_expr(*x)).collect();
                self.store_value(dest, RValue::Struct { fields: args })
            }
            ast::Expr::ArrayRepeat { expr, count } => {
                let value = self.lower_expr(expr);
                let count = self.lower_expr(count);
                self.store_value(dest, RValue::StructRepeat { value, count })
            }
            ast::Expr::Struct(ref s) => {
                let symbol = self.ast.exprs.value_binding(expr);
                let declared = self.tcx.struct_properties[&symbol].clone();
                let args = declared
                    .keys()
                    .map(|name| self.lower_expr(s.fields[name]))
                    .collect();
                self.store_value(dest, RValue::Struct { fields: args })
            }
            ast::Expr::Tuple(ref args) => {
                let args = args.iter().map(|x| self.lower_expr(*x)).collect();
                self.store_value(dest, RValue::Struct { fields: args })
            }
            ast::Expr::StructField { obj, name } => {
                let obj_ty = self.tcx.expr_types[&obj];
                let reg = self.lower_expr(obj);
                let Ty::Named(symbol) = self.tcx.types.probe_value(obj_ty) else {
                    unreachable!();
                };
                let properties = &self.tcx.struct_properties[&symbol];
                let index = properties.get_index_of(&name).unwrap();
                self.store_value(
                    dest,
                    RValue::Field {
                        reg,
                        index: index as u16,
                    },
                );
            }
            ast::Expr::TupleField { obj, index } => {
                let reg = self.lower_expr(obj);
                self.store_value(
                    dest,
                    RValue::Field {
                        reg,
                        index: index as u16,
                    },
                );
            }
            ast::Expr::ArrayIndex { array, index } => {
                let array = self.lower_expr(array);
                let index = self.lower_expr(index);
                self.store_value(dest, RValue::Index { reg: array, index });
            }
            ast::Expr::Block { ref stmts, expr } => {
                for &stmt in stmts {
                    self.lower_stmt(stmt);
                }
                if let Some(expr) = expr {
                    self.lower_expr_into(dest, expr)
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
                                value: RValue::Use(reg),
                            });
                        }
                    }
                    SymbolKind::Fn => {
                        self.function_worklist.push_back(symbol_id);
                        self.store_constant(dest, Constant::Function(symbol_id));
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
                            reg: obj,
                            index,
                            value,
                        });
                    }
                    ast::Expr::StructField { obj, name } => {
                        let ty = self.tcx.expr_types[&obj];
                        let Ty::Named(symbol) = self.tcx.types.probe_value(ty) else {
                            unreachable!();
                        };
                        let properties = &self.tcx.struct_properties[&symbol];
                        let index = properties.get_index_of(&name).unwrap() as u16;

                        let obj = self.lower_expr(obj);
                        let value = self.lower_expr(rhs);
                        self.push_stmt(Statement::SetField {
                            reg: obj,
                            index,
                            value,
                        });
                    }
                    ast::Expr::ArrayIndex { array, index } => {
                        let array = self.lower_expr(array);
                        let index = self.lower_expr(index);
                        let value = self.lower_expr(rhs);
                        self.push_stmt(Statement::SetIndex {
                            reg: array,
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

    fn store_constant(&mut self, dest: RegId, value: Constant) {
        self.store_value(dest, RValue::Constant(value));
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

    fn create_register(&mut self, implicit: bool) -> RegId {
        let reg = self.next_register;
        self.next_register += 1;
        if implicit {
            self.push_stmt(Statement::ImplicitDef { reg });
        }
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
}

pub type BasicBlockId = Id<BasicBlock>;
pub type RegId = u32;

pub struct Ir {
    pub main: BasicBlockId,
    pub functions: FxHashMap<SymbolId, BasicBlockId>,
    pub basic_blocks: Arena<BasicBlock>,
    pub reg_count: FxHashMap<BasicBlockId, RegId>,
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
        reg: RegId,
        index: u16,
        value: RegId,
    },
    SetIndex {
        reg: RegId,
        index: RegId,
        value: RegId,
    },
    ImplicitDef {
        reg: RegId,
    },
    Print(RegId, TyId),
}

#[derive(Debug)]
pub enum Terminator {
    Jump {
        target: BasicBlockId,
    },
    Branch {
        cond: RegId,
        then_block: BasicBlockId,
        else_block: BasicBlockId,
    },
    Return,
}

#[derive(Debug)]
pub enum RValue {
    Constant(Constant),
    Use(RegId),
    Field {
        reg: RegId,
        index: u16,
    },
    Index {
        reg: RegId,
        index: RegId,
    },
    Binary {
        op: BinaryOp,
        lhs: RegId,
        rhs: RegId,
    },
    Not(RegId),
    F64Neg(RegId),
    I64Neg(RegId),
    Struct {
        fields: Vec<RegId>,
    },
    StructRepeat {
        value: RegId,
        count: RegId,
    },
    Call {
        callee: RegId,
        args: Vec<RegId>,
    },
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
}
