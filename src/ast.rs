use core::fmt;
use std::collections::HashMap;

use crate::arena::{AstArena, AstId};

pub type ExprId = AstId<Expr>;
pub type StmtId = AstId<Stmt>;
pub type TypeId = AstId<Type>;

pub struct AstNodes {
    pub exprs: AstArena<Expr>,
    pub stmts: AstArena<Stmt>,
    pub types: AstArena<Type>,
}

impl AstNodes {
    pub fn new() -> Self {
        Self {
            exprs: AstArena::new(),
            stmts: AstArena::new(),
            types: AstArena::new(),
        }
    }
}

#[derive(Debug)]
pub enum Expr {
    Bool(bool),
    Number(f64),
    String(String),
    Not(ExprId),
    Negate(ExprId),
    Binary(ExprId, BinaryOp, ExprId),
    Variable(String),
    Assign(ExprId, ExprId),
    If(ExprId, ExprId, Option<ExprId>),
    Block(Vec<StmtId>, Option<ExprId>),
    While(ExprId, ExprId),
    For(Option<StmtId>, Option<ExprId>, Option<ExprId>, ExprId),
    Break,
    Continue,
    Call(ExprId, Vec<ExprId>),
    Return(Option<ExprId>),
    Unit,
    TupleField(ExprId, u16),
    Tuple(Vec<ExprId>),
    Struct(String, HashMap<String, ExprId>),
    StructField(ExprId, String),
}

#[derive(Debug)]
pub enum Stmt {
    Let(String, Option<TypeId>, ExprId),
    Expr(ExprId, bool),
    Param(String, TypeId),
    Fn(String, Vec<StmtId>, Option<TypeId>, ExprId),
    Struct(String, HashMap<String, TypeId>),
}

#[derive(Debug)]
pub enum Type {
    Named(String),
    FnType(Vec<TypeId>, Option<TypeId>),
    Unit,
    Tuple(Vec<TypeId>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
    Equal,
    NotEqual,
    Or,
    And,
    BitOr,
    BitAnd,
}

impl fmt::Display for BinaryOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Add => write!(f, "+"),
            Self::Sub => write!(f, "-"),
            Self::Mul => write!(f, "*"),
            Self::Div => write!(f, "/"),
            Self::Greater => write!(f, ">"),
            Self::GreaterEqual => write!(f, ">="),
            Self::Less => write!(f, "<"),
            Self::LessEqual => write!(f, "<="),
            Self::Equal => write!(f, "=="),
            Self::NotEqual => write!(f, "!="),
            Self::Or => write!(f, "||"),
            Self::And => write!(f, "&&"),
            Self::BitOr => write!(f, "|"),
            Self::BitAnd => write!(f, "&"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            start: start as u32,
            end: end as u32,
        }
    }

    pub fn text(self, source: &str) -> &str {
        &source[self.start as usize..self.end as usize]
    }

    pub fn to(self, other: Self) -> Self {
        Self {
            start: self.start,
            end: other.end,
        }
    }
}

pub trait AstVisitor {
    fn visit_stmt(&mut self, _id: StmtId, _ast: &AstNodes) {}

    fn visit_expr(&mut self, _id: ExprId, _ast: &AstNodes) {}

    fn visit_type(&mut self, _id: TypeId, _ast: &AstNodes) {}
}

impl AstNodes {
    pub fn visit(&self, stmts: &[StmtId], visitor: &mut impl AstVisitor) {
        for &stmt in stmts {
            self.visit_stmt(stmt, visitor);
        }
    }

    fn visit_stmt(&self, stmt: StmtId, visitor: &mut impl AstVisitor) {
        visitor.visit_stmt(stmt, self);
        match self.stmts[stmt] {
            Stmt::Struct(_, ref fields) => {
                for ty in fields.values() {
                    self.visit_type(*ty, visitor);
                }
            }
            Stmt::Fn(_, ref params, ret, body) => {
                for p in params {
                    self.visit_stmt(*p, visitor);
                }
                if let Some(ret) = ret {
                    self.visit_type(ret, visitor);
                }
                self.visit_expr(body, visitor);
            }
            Stmt::Let(_, ty, init) => {
                if let Some(ty) = ty {
                    self.visit_type(ty, visitor);
                }
                self.visit_expr(init, visitor)
            }
            Stmt::Expr(expr, _) => self.visit_expr(expr, visitor),
            Stmt::Param(_, type_id) => self.visit_type(type_id, visitor),
        }
    }

    fn visit_expr(&self, expr: ExprId, visitor: &mut impl AstVisitor) {
        visitor.visit_expr(expr, self);
        match self.exprs[expr] {
            Expr::Bool(_) | Expr::Number(_) | Expr::String(_) => {}
            Expr::Not(expr) => self.visit_expr(expr, visitor),
            Expr::Negate(expr) => self.visit_expr(expr, visitor),
            Expr::Binary(lhs, _, rhs) => {
                self.visit_expr(lhs, visitor);
                self.visit_expr(rhs, visitor);
            }
            Expr::Variable(_) => {}
            Expr::Assign(lhs, rhs) => {
                self.visit_expr(lhs, visitor);
                self.visit_expr(rhs, visitor);
            }
            Expr::If(cond, then_clause, else_clause) => {
                self.visit_expr(cond, visitor);
                self.visit_expr(then_clause, visitor);
                if let Some(else_clause) = else_clause {
                    self.visit_expr(else_clause, visitor);
                }
            }
            Expr::Block(ref stmts, expr) => crate::stack_safe(|| {
                for stmt in stmts {
                    self.visit_stmt(*stmt, visitor);
                }
                if let Some(expr) = expr {
                    self.visit_expr(expr, visitor);
                }
            }),
            Expr::While(cond, body) => {
                self.visit_expr(cond, visitor);
                self.visit_expr(body, visitor);
            }
            Expr::For(init, cond, incr, body) => {
                if let Some(stmt) = init {
                    self.visit_stmt(stmt, visitor);
                }
                if let Some(expr) = cond {
                    self.visit_expr(expr, visitor);
                }
                if let Some(expr) = incr {
                    self.visit_expr(expr, visitor);
                }
                self.visit_expr(body, visitor);
            }
            Expr::Break | Expr::Continue => {}
            Expr::Call(callee, ref args) => {
                self.visit_expr(callee, visitor);
                for arg in args {
                    self.visit_expr(*arg, visitor);
                }
            }
            Expr::Return(expr) => {
                if let Some(expr) = expr {
                    self.visit_expr(expr, visitor);
                }
            }
            Expr::Unit => {}
            Expr::TupleField(expr, _) => {
                self.visit_expr(expr, visitor);
            }
            Expr::Tuple(ref args) => {
                for arg in args {
                    self.visit_expr(*arg, visitor);
                }
            }
            Expr::Struct(_, ref fields) => {
                for value in fields.values() {
                    self.visit_expr(*value, visitor);
                }
            }
            Expr::StructField(expr, _) => {
                self.visit_expr(expr, visitor);
            }
        }
    }

    fn visit_type(&self, type_id: TypeId, visitor: &mut impl AstVisitor) {
        visitor.visit_type(type_id, self);
        match self.types[type_id] {
            Type::Named(_) => {}
            Type::FnType(ref params, ret) => {
                for p in params {
                    self.visit_type(*p, visitor);
                }
                if let Some(ret) = ret {
                    self.visit_type(ret, visitor);
                }
            }
            Type::Unit => {}
            Type::Tuple(ref params) => {
                for p in params {
                    self.visit_type(*p, visitor);
                }
            }
        }
    }
}
