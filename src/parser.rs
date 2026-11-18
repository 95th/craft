use std::ops::Add;

use crate::{
    ast::{BinaryOp, Node, NodeArena, NodeId},
    scanner::{
        Scanner,
        Token::{self, *},
    },
};

pub struct Parser<'a> {
    scanner: Scanner<'a>,
    nodes: &'a mut NodeArena,
    token: Token,
    had_error: bool,
    panic_mode: bool,
}

impl<'a> Parser<'a> {
    pub fn new(source: &'a str, nodes: &'a mut NodeArena) -> Self {
        Self {
            scanner: Scanner::new(source),
            nodes,
            token: Error,
            had_error: false,
            panic_mode: false,
        }
    }

    pub fn parse(&mut self) -> NodeId {
        self.advance();
        expression(self)
    }

    fn advance(&mut self) {
        loop {
            self.token = self.scanner.scan_token();
            if self.token == Error {
                self.error(self.scanner.error_message());
            } else {
                break;
            }
        }
    }

    fn consume(&mut self, token: Token, message: &str) {
        if self.token == token {
            self.advance();
            return;
        }

        self.error(message);
    }

    fn error(&mut self, message: &str) {
        if self.panic_mode {
            return;
        }
        self.panic_mode = true;
        self.had_error = true;
        match self.token {
            Eof => print!("Error at end of file: {message}"),
            Error => print!("[Line: {}] Error: {message}", self.scanner.line()),
            _ => print!(
                "[Line: {}] Error at '{}': {message}",
                self.scanner.line(),
                self.scanner.token_text()
            ),
        }
    }
}

fn expression(p: &mut Parser) -> NodeId {
    parse_precedence(p, Precedence::ASSIGNMENT)
}

fn grouping(p: &mut Parser) -> NodeId {
    p.advance();
    let expr = expression(p);
    p.consume(CloseParen, "Expect ')' after grouping.");
    expr
}

fn binary(p: &mut Parser, lhs: NodeId) -> NodeId {
    let op = match p.token {
        Plus => BinaryOp::Add,
        Minus => BinaryOp::Sub,
        Star => BinaryOp::Mul,
        Slash => BinaryOp::Div,
        Greater => BinaryOp::Greater,
        GreaterEqual => BinaryOp::GreaterEqual,
        Less => BinaryOp::Less,
        LessEqual => BinaryOp::LessEqual,
        EqualEqual => BinaryOp::Equal,
        BangEqual => BinaryOp::NotEqual,
        _ => unreachable!(),
    };
    let precedence = precedence_of(p.token);
    p.advance();
    let rhs = parse_precedence(p, precedence + 1);
    p.nodes.alloc(Node::Binary(lhs, op, rhs))
}

fn unary(p: &mut Parser) -> NodeId {
    let op = p.token;
    p.advance();
    let expr = parse_precedence(p, Precedence::UNARY);
    match op {
        Bang => p.nodes.alloc(Node::Not(expr)),
        Minus => p.nodes.alloc(Node::Negate(expr)),
        _ => unreachable!(),
    }
}

fn number(p: &mut Parser) -> NodeId {
    let n = p.scanner.token_text().parse().unwrap();
    p.advance();
    p.nodes.alloc(Node::Number(n))
}

fn string(p: &mut Parser) -> NodeId {
    let text = p.scanner.token_text();
    let text = text[1..text.len() - 1].to_string();
    p.advance();
    p.nodes.alloc(Node::String(text))
}

fn boolean(p: &mut Parser) -> NodeId {
    let value = p.token == Token::True;
    p.advance();
    p.nodes.alloc(Node::Bool(value))
}

fn parse_precedence(p: &mut Parser, precedence: Precedence) -> NodeId {
    let Some(mut expr) = parse_prefix(p) else {
        p.error("Expect expression.");
        return p.nodes.alloc(Node::Error);
    };

    while precedence <= precedence_of(p.token) {
        expr = parse_infix(p, expr);
    }

    expr
}

fn parse_prefix(p: &mut Parser) -> Option<NodeId> {
    Some(match p.token {
        OpenParen => grouping(p),
        Minus | Bang => unary(p),
        NumberLiteral => number(p),
        StringLiteral => string(p),
        True | False => boolean(p),
        _ => return None,
    })
}

fn parse_infix(p: &mut Parser, lhs: NodeId) -> NodeId {
    match p.token {
        Plus | Minus | Star | Slash | Greater | GreaterEqual | Less | LessEqual | EqualEqual
        | BangEqual => binary(p, lhs),
        _ => lhs,
    }
}

fn precedence_of(token: Token) -> Precedence {
    match token {
        Plus | Minus => Precedence::TERM,
        Star | Slash => Precedence::FACTOR,
        Greater | GreaterEqual | Less | LessEqual => Precedence::COMPARISON,
        EqualEqual | BangEqual => Precedence::EQUALITY,
        _ => Precedence::NONE,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Precedence(u8);

impl Add<u8> for Precedence {
    type Output = Self;

    fn add(self, rhs: u8) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl Precedence {
    const NONE: Self = Self(0);
    const ASSIGNMENT: Self = Self(1);
    const OR: Self = Self(2);
    const AND: Self = Self(3);
    const EQUALITY: Self = Self(4);
    const COMPARISON: Self = Self(5);
    const TERM: Self = Self(6);
    const FACTOR: Self = Self(7);
    const UNARY: Self = Self(8);
    const CALL: Self = Self(9);
}
