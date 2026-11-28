use core::fmt;

use crate::{
    FxIndexMap,
    arena::{AstArena, AstId},
    interner::StrId,
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
    String(StrId),
    Not(ExprId),
    Negate(ExprId),
    Binary {
        lhs: ExprId,
        op: BinaryOp,
        rhs: ExprId,
    },
    Variable(StrId),
    Assign {
        lhs: ExprId,
        rhs: ExprId,
    },
    If {
        cond: ExprId,
        then_clause: ExprId,
        else_clause: Option<ExprId>,
    },
    Block {
        stmts: Box<[StmtId]>,
        expr: Option<ExprId>,
    },
    While {
        cond: ExprId,
        body: ExprId,
    },
    For {
        init: Option<StmtId>,
        cond: Option<ExprId>,
        incr: Option<ExprId>,
        body: ExprId,
    },
    Break,
    Continue,
    Call {
        callee: ExprId,
        args: Box<[ExprId]>,
    },
    Return(Option<ExprId>),
    Unit,
    TupleField {
        obj: ExprId,
        index: u16,
    },
    Tuple(Box<[ExprId]>),
    Struct(Box<StructExpr>),
    StructField {
        obj: ExprId,
        name: StrId,
    },
    Array(Box<[ExprId]>),
    ArrayRepeat {
        expr: ExprId,
        count: ExprId,
    },
    ArrayIndex {
        array: ExprId,
        index: ExprId,
    },
}

#[derive(Debug)]
pub enum Stmt {
    Let {
        name: StrId,
        typ: Option<TypeId>,
        init: ExprId,
    },
    Expr {
        expr: ExprId,
        semi: bool,
    },
    Param {
        name: StrId,
        typ: TypeId,
    },
    Fn(Box<FnDecl>),
    Struct(Box<StructDecl>),
    TupleStruct(Box<TupleStructDecl>),
    TypeParam(StrId),
}

#[derive(Debug)]
pub struct StructDecl {
    pub name: StrId,
    pub fields: FxIndexMap<StrId, TypeId>,
}

#[derive(Debug)]
pub struct TupleStructDecl {
    pub name: StrId,
    pub fields: Box<[TypeId]>,
}

#[derive(Debug)]
pub struct StructExpr {
    pub name: StrId,
    pub fields: FxIndexMap<StrId, ExprId>,
}

#[derive(Debug)]
pub struct FnDecl {
    pub name: StrId,
    pub type_params: Box<[StmtId]>,
    pub params: Box<[StmtId]>,
    pub return_typ: Option<TypeId>,
    pub body: ExprId,
}

#[derive(Debug)]
pub enum Type {
    Named(StrId),
    FnType {
        params: Box<[TypeId]>,
        ret: Option<TypeId>,
    },
    Unit,
    Tuple(Box<[TypeId]>),
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
