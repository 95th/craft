use core::fmt;

#[derive(Debug)]
pub enum Node {
    Error,
    Bool(bool),
    Number(f64),
    String(String),
    Not(NodeId),
    Negate(NodeId),
    Binary(NodeId, BinaryOp, NodeId),
    Let(String, Option<NodeId>, NodeId),
    Variable(String),
    Assign(NodeId, NodeId),
    If(NodeId, NodeId, Option<NodeId>),
    Block(Vec<NodeId>),
    ExprStatement(NodeId),
    While(NodeId, NodeId),
    Break,
    Continue,
    Fn(String, Vec<(String, NodeId)>, Option<NodeId>, NodeId),
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
    pub error_node: NodeId,
}

impl NodeArena {
    pub fn new() -> Self {
        Self {
            nodes: vec![Node::Error],
            spans: vec![Span::new(0, 0)],
            error_node: NodeId::new(0),
        }
    }

    pub fn alloc(&mut self, node: Node, span: Span) -> NodeId {
        self.nodes.push(node);
        self.spans.push(span);
        NodeId::new(self.nodes.len() - 1)
    }

    pub fn dump(&self, source: &str) {
        for i in 0..self.nodes.len() {
            println!(
                "{:?}: {:?} -> {}",
                self.nodes[i],
                self.spans[i].text(source),
                NodeId::new(i).display(self)
            )
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(u32);

impl NodeId {
    pub fn new(index: usize) -> Self {
        Self(index as u32)
    }

    pub fn get(self, arena: &NodeArena) -> &Node {
        &arena.nodes[self.0 as usize]
    }

    pub fn span(self, arena: &NodeArena) -> Span {
        arena.spans[self.0 as usize]
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
            Node::Error => write!(f, "<error>"),
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
            Node::Block(nodes) => {
                write!(f, "{{ ")?;
                for node in nodes {
                    write!(f, "{}", node.display(arena))?;
                }
                write!(f, " }}")
            }
            Node::ExprStatement(node) => {
                write!(f, "{};", node.display(arena))
            }
            Node::While(cond, body) => {
                write!(f, "while {} {}", cond.display(arena), body.display(arena))
            }
            Node::Break => write!(f, "break"),
            Node::Continue => write!(f, "continue"),
            Node::Fn(name, params, return_ty, body) => {
                write!(f, "fn {name}(")?;
                for (name, ty) in params {
                    write!(f, "{name}: {}", ty.display(arena))?;
                }
                write!(f, ")")?;
                if let Some(return_ty) = return_ty {
                    write!(f, " -> {}", return_ty.display(arena))?;
                }
                write!(f, " {}", body.display(arena))
            }
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
