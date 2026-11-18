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
}

pub struct NodeArena {
    nodes: Vec<Node>,
}

impl NodeArena {
    pub fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    pub fn alloc(&mut self, node: Node) -> NodeId {
        self.nodes.push(node);
        NodeId::new(self.nodes.len() - 1)
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

    pub fn display(self, arena: &NodeArena) -> impl fmt::Display {
        NodeDisplay(self, arena)
    }
}

struct NodeDisplay<'a>(NodeId, &'a NodeArena);

impl fmt::Display for NodeDisplay<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self(node, arena) = *self;
        match node.get(arena) {
            Node::Error => unreachable!(),
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
        }
    }
}
