use core::fmt;
use std::collections::HashMap;

#[derive(Debug)]
pub enum Node {
    Bool(bool),
    Number(f64),
    String(String),
    Not(NodeId),
    Negate(NodeId),
    Binary(NodeId, BinaryOp, NodeId),
    Variable(String),
    Assign(NodeId, NodeId),
    If(NodeId, NodeId, Option<NodeId>),
    Block(Vec<NodeId>, Option<NodeId>),
    While(NodeId, NodeId),
    For(Option<NodeId>, Option<NodeId>, Option<NodeId>, NodeId),
    Break,
    Continue,
    Call(NodeId, Vec<NodeId>),
    Return(Option<NodeId>),
    Unit,
    TupleField(NodeId, u16),
    Tuple(Vec<NodeId>),
    Struct(String, HashMap<String, NodeId>),
    StructField(NodeId, String),

    // Statements
    Let(String, Option<NodeId>, NodeId),
    Semi(NodeId),
    Param(String, NodeId),

    // Items
    Fn(String, Vec<NodeId>, Option<NodeId>, NodeId),

    // Type
    FnType(Vec<NodeId>, Option<NodeId>),
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

pub struct NodeArena {
    nodes: Vec<Node>,
    spans: Vec<Span>,
}

impl NodeArena {
    pub fn new() -> Self {
        Self {
            nodes: vec![],
            spans: vec![],
        }
    }

    pub fn alloc(&mut self, node: Node, span: Span) -> NodeId {
        self.nodes.push(node);
        self.spans.push(span);
        NodeId::new(self.nodes.len() - 1)
    }

    pub fn dump(&self, source: &str) {
        for i in 0..self.nodes.len() {
            let node = NodeId::new(i);
            println!(
                "{node:?} = {:?}, source={:?}, parsed={}",
                self.nodes[i],
                self.spans[i].text(source),
                node.display(self)
            )
        }
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn truncate(&mut self, len: usize) {
        self.nodes.truncate(len);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(u32);

impl NodeId {
    pub const ERROR: Self = Self(u32::MAX);

    pub fn new(index: usize) -> Self {
        Self(index as u32)
    }

    pub fn idx(self) -> usize {
        self.0 as usize
    }

    pub fn get(self, arena: &NodeArena) -> &Node {
        &arena.nodes[self.idx()]
    }

    pub fn span(self, arena: &NodeArena) -> Span {
        arena.spans[self.idx()]
    }

    pub fn display(self, arena: &NodeArena) -> impl fmt::Display {
        NodeDisplay(self, arena)
    }
}

struct NodeDisplay<'a>(NodeId, &'a NodeArena);

impl fmt::Display for NodeDisplay<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self(node, arena) = *self;
        match node.get(arena) {
            Node::Bool(x) => write!(f, "{x}"),
            Node::Number(x) => write!(f, "{x:?}"),
            Node::String(x) => write!(f, "{x:?}"),
            Node::Not(x) => write!(f, "!{}", x.display(arena)),
            Node::Negate(x) => write!(f, "-{}", x.display(arena)),
            Node::Binary(lhs, op, rhs) => {
                write!(f, "({} {} {})", lhs.display(arena), op, rhs.display(arena))
            }
            Node::Let(name, ty, init) => {
                write!(f, "let {name}")?;
                if let Some(ty) = ty {
                    write!(f, ": {}", ty.display(arena))?;
                }
                write!(f, " = {};", init.display(arena))
            }
            Node::Variable(x) => write!(f, "{x}"),
            Node::Assign(lhs, rhs) => write!(f, "{} = {}", lhs.display(arena), rhs.display(arena)),
            Node::If(cond, then_clause, else_clause) => {
                write!(
                    f,
                    "if {} {}",
                    cond.display(arena),
                    then_clause.display(arena)
                )?;
                if let Some(else_clause) = else_clause {
                    write!(f, " else {}", else_clause.display(arena))?;
                }
                Ok(())
            }
            Node::Block(nodes, expr) => {
                write!(f, "{{ ")?;
                for node in nodes {
                    write!(f, "{}", node.display(arena))?;
                }
                if let Some(expr) = expr {
                    write!(f, "{}", expr.display(arena))?;
                }
                write!(f, " }}")
            }
            Node::Semi(node) => {
                write!(f, "{};", node.display(arena))
            }
            Node::While(cond, body) => {
                write!(f, "while {} {}", cond.display(arena), body.display(arena))
            }
            Node::For(init, cond, incr, body) => {
                write!(f, "for (");
                if let Some(init) = init {
                    write!(f, "{}", init.display(arena));
                }
                write!(f, ";");
                if let Some(cond) = cond {
                    write!(f, "{}", cond.display(arena));
                }
                write!(f, ";");
                if let Some(incr) = incr {
                    write!(f, "{}", incr.display(arena));
                }
                write!(f, ") {}", body.display(arena))
            }
            Node::Break => write!(f, "break"),
            Node::Continue => write!(f, "continue"),
            Node::Fn(name, params, return_ty, body) => {
                write!(f, "fn {name}(")?;
                let mut sep = "";
                for param in params {
                    write!(f, "{sep}{}", param.display(arena))?;
                    sep = ", ";
                }
                write!(f, ")")?;
                if let Some(return_ty) = return_ty {
                    write!(f, " -> {}", return_ty.display(arena))?;
                }
                write!(f, " {}", body.display(arena))
            }
            Node::Param(name, ty) => write!(f, "{name}: {}", ty.display(arena)),
            Node::Call(target, args) => {
                write!(f, "{}(", target.display(arena))?;
                let mut sep = "";
                for arg in args {
                    write!(f, "{sep}{}", arg.display(arena))?;
                    sep = ", ";
                }
                write!(f, ")")
            }
            Node::Return(value) => {
                write!(f, "return")?;
                if let Some(value) = value {
                    write!(f, " {}", value.display(arena))?;
                }
                write!(f, ";")
            }
            Node::FnType(params, return_ty) => {
                write!(f, "fn(")?;
                let mut sep = "";
                for param in params {
                    write!(f, "{sep}{}", param.display(arena))?;
                    sep = ", ";
                }
                write!(f, ")")?;
                if let Some(return_ty) = return_ty {
                    write!(f, " -> {}", return_ty.display(arena))?;
                }
                Ok(())
            }
            Node::Unit => write!(f, "()"),
            Node::Tuple(nodes) => {
                write!(f, "(")?;
                let mut sep = "";
                for t in nodes {
                    write!(f, "{sep}{}", t.display(arena))?;
                    sep = ", ";
                }
                write!(f, ")")
            }
            Node::TupleField(target, index) => write!(f, "{}.{}", target.display(arena), index),
            Node::Struct(name, fields) => {
                write!(f, "struct {name} {{ ")?;
                let mut sep = "";
                for (field_name, field_ty) in fields {
                    write!(f, "{sep}{field_name}: {}", field_ty.display(arena))?;
                    sep = ", ";
                }
                write!(f, " }}")
            }
            Node::StructField(target, name) => write!(f, "{}.{}", target.display(arena), name),
        }
    }
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
