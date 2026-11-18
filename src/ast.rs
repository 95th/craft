use core::fmt;

use crate::{
    FxIndexMap,
    arena::{AstArena, AstId},
};

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
    Int(i128),
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
    Struct(String, FxIndexMap<String, ExprId>),
    StructField(ExprId, String),
    Array(Vec<ExprId>),
    ArrayRepeat(ExprId, ExprId),
    ArrayIndex(ExprId, ExprId),
}

#[derive(Debug)]
pub enum Stmt {
    Let(String, Option<TypeId>, ExprId),
    Expr(ExprId, bool),
    Param(String, TypeId),
    Fn(String, Vec<StmtId>, Vec<StmtId>, Option<TypeId>, ExprId),
    Struct(String, FxIndexMap<String, TypeId>),
    TupleStruct(String, Vec<TypeId>),
    TypeParam(String),
}

#[derive(Debug)]
pub enum Type {
    Named(String),
    FnType(Vec<TypeId>, Option<TypeId>),
    Unit,
    Tuple(Vec<TypeId>),
    Array(TypeId),
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

    pub fn to(self, other: Self) -> Self {
        Self {
            start: self.start,
            end: other.end,
        }
    }
}
