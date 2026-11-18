use std::collections::HashMap;

use crate::{
    ast::{BinaryOp, Node, NodeArena, NodeId, Span},
    scanner::{
        Scanner,
        Token::{self, *},
    },
};

pub struct Parser<'a> {
    source: &'a str,
    scanner: Scanner<'a>,
    nodes: &'a mut NodeArena,
    token: Token,
    prev_span: Span,
    had_error: bool,
    panic_mode: bool,
}

impl<'a> Parser<'a> {
    pub fn new(source: &'a str, nodes: &'a mut NodeArena) -> Self {
        Self {
            source,
            scanner: Scanner::new(source),
            nodes,
            token: Error,
            prev_span: Span::new(0, 0),
            had_error: false,
            panic_mode: false,
        }
    }

    pub fn parse(&mut self) -> Option<Vec<NodeId>> {
        self.advance();
        let mut nodes = Vec::new();
        while self.token != Token::Eof {
            nodes.push(declaration(self));
        }
        if self.had_error { None } else { Some(nodes) }
    }

    fn eat(&mut self, token: Token) -> bool {
        if self.token == token {
            self.advance();
            true
        } else {
            false
        }
    }

    fn advance(&mut self) {
        self.prev_span = self.scanner.span();
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

    fn error(&mut self, message: &str) -> NodeId {
        self.error_at(message, self.scanner.span())
    }

    fn error_at(&mut self, message: &str, span: Span) -> NodeId {
        if self.panic_mode {
            return NodeId::ERROR;
        }
        self.panic_mode = true;
        self.had_error = true;
        print!("Error at '{}': {message}", span.text(self.source));
        NodeId::ERROR
    }
}

fn declaration(p: &mut Parser) -> NodeId {
    match p.token {
        Let => let_declaration(p),
        Fn => fn_declaration(p),
        Struct => struct_declaration(p),
        _ => expression_statement(p),
    }
}

fn let_declaration(p: &mut Parser) -> NodeId {
    p.advance();
    let start = p.prev_span;
    let name = p.scanner.token_text().to_string();
    p.consume(Identifier, "Expect variable name.");
    let type_annotation = if p.eat(Colon) { Some(ty(p)) } else { None };
    p.consume(Equal, "Expect '=' before initializer.");
    let init = expression(p);
    p.consume(Semicolon, "Expect ';' after initializer.");
    p.nodes.alloc(
        Node::Let(name, type_annotation, init),
        start.to(p.prev_span),
    )
}

fn fn_declaration(p: &mut Parser) -> NodeId {
    p.advance();
    let start = p.prev_span;
    let name = p.scanner.token_text().to_string();
    p.consume(Identifier, "Expect function name.");
    p.consume(OpenParen, "Expect '(' after function name.");
    let mut params = Vec::new();
    if p.token != CloseParen {
        loop {
            let name = p.scanner.token_text().to_string();
            p.consume(Identifier, "Expect parameter name.");
            let param_span = p.prev_span;
            p.consume(Colon, "Expect ':' after parameter name.");
            let ty = ty(p);
            params.push(
                p.nodes
                    .alloc(Node::Param(name, ty), param_span.to(p.prev_span)),
            );
            if !p.eat(Comma) {
                break;
            }
        }
    }
    p.consume(CloseParen, "Expect ')' after function parameters.");
    let return_ty = if p.eat(RightArrow) { Some(ty(p)) } else { None };
    p.consume(OpenBrace, "Expect '{' before function body");
    let body = block_expr(p);
    p.nodes.alloc(
        Node::Fn(name, params, return_ty, body),
        start.to(p.prev_span),
    )
}

fn struct_declaration(p: &mut Parser) -> NodeId {
    p.advance();
    let start = p.prev_span;
    let name = p.scanner.token_text().to_string();
    p.consume(Identifier, "Expect struct name.");
    p.consume(OpenBrace, "Expect '{' after struct name.");
    let mut fields = HashMap::new();
    if p.token != CloseBrace {
        loop {
            let name = p.scanner.token_text();
            p.consume(Identifier, "Expect struct field name.");
            let name_span = p.prev_span;
            p.consume(Colon, "Expect ':' after field name.");
            let ty = ty(p);
            if fields.insert(name.to_string(), ty).is_some() {
                p.error_at(&format!("Duplicate field: {name}"), name_span);
            };
            if !p.eat(Comma) {
                break;
            }
        }
    }
    p.consume(CloseBrace, "Expect '}' after struct fields.");
    p.nodes
        .alloc(Node::Struct(name, fields), start.to(p.prev_span))
}

fn expression_statement(p: &mut Parser) -> NodeId {
    let expr = expression(p);
    if p.eat(Semicolon) {
        p.nodes
            .alloc(Node::Semi(expr), expr.span(p.nodes).to(p.prev_span))
    } else {
        expr
    }
}

fn ty(p: &mut Parser) -> NodeId {
    match p.token {
        Identifier => variable(p),
        Fn => fn_ty(p),
        OpenParen => tuple_ty(p),
        _ => p.error("Expected a type"),
    }
}

fn fn_ty(p: &mut Parser) -> NodeId {
    p.advance();
    let start = p.prev_span;
    p.consume(OpenParen, "Expect '(' after 'fn' type.");
    let mut params = Vec::new();
    if p.token != CloseParen {
        loop {
            params.push(ty(p));
            if !p.eat(Comma) {
                break;
            }
        }
    }
    p.consume(CloseParen, "Expect ')' after parameters.");
    let return_ty = if p.eat(RightArrow) { Some(ty(p)) } else { None };
    p.nodes
        .alloc(Node::FnType(params, return_ty), start.to(p.prev_span))
}

fn tuple_ty(p: &mut Parser) -> NodeId {
    p.advance();
    let start = p.prev_span;
    if p.eat(CloseParen) {
        return p.nodes.alloc(Node::Unit, start.to(p.prev_span));
    }

    let t = ty(p);
    if p.eat(CloseParen) {
        return t;
    }

    let mut types = vec![t];
    if p.eat(Comma) {
        loop {
            types.push(ty(p));
            if !p.eat(Comma) {
                break;
            }
        }
    }
    p.consume(CloseParen, "Expect ')' after tuple types.");
    p.nodes.alloc(Node::Tuple(types), start.to(p.prev_span))
}

fn expression(p: &mut Parser) -> NodeId {
    parse_precedence(p, Precedence::Assignment as u8)
}

fn tuple(p: &mut Parser) -> NodeId {
    p.advance();
    let start = p.prev_span;
    if p.eat(CloseParen) {
        return p.nodes.alloc(Node::Unit, start.to(p.prev_span));
    }

    let expr = expression(p);
    if p.eat(Comma) {
        let mut exprs = vec![expr];
        loop {
            exprs.push(expression(p));
            if !p.eat(Comma) {
                break;
            }
        }
        p.consume(CloseParen, "Expect ')' after tuple values.");
        return p.nodes.alloc(Node::Tuple(exprs), start.to(p.prev_span));
    }

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
        BarBar => BinaryOp::Or,
        AmpersandAmpersand => BinaryOp::And,
        Bar => BinaryOp::BitOr,
        Ampersand => BinaryOp::BitAnd,
        _ => unreachable!(),
    };
    let precedence = p.token.precedence() as u8;
    p.advance();
    let rhs = parse_precedence(p, precedence + 1);
    p.nodes.alloc(
        Node::Binary(lhs, op, rhs),
        lhs.span(p.nodes).to(p.prev_span),
    )
}

fn assignment(p: &mut Parser, lhs: NodeId) -> NodeId {
    p.advance();
    let rhs = expression(p);
    let lhs_span = lhs.span(p.nodes);
    match lhs.get(p.nodes) {
        Node::Variable(..) => p
            .nodes
            .alloc(Node::Assign(lhs, rhs), lhs_span.to(p.prev_span)),
        _ => p.error_at("Invalid assignment target", lhs_span),
    }
}

fn call(p: &mut Parser, lhs: NodeId) -> NodeId {
    p.advance();
    let mut args = Vec::new();
    if p.token != CloseParen {
        loop {
            args.push(expression(p));
            if !p.eat(Comma) {
                break;
            }
        }
    }
    p.consume(CloseParen, "Expect ')' after arguments.");
    p.nodes
        .alloc(Node::Call(lhs, args), lhs.span(p.nodes).to(p.prev_span))
}

fn dot(p: &mut Parser, lhs: NodeId) -> NodeId {
    p.advance();
    let name = p.scanner.token_text().to_string();
    if p.eat(NumberLiteral) {
        let index = match name.parse() {
            Ok(n) => n,
            _ => {
                p.error_at("Expect an integer tuple field", p.prev_span);
                0
            }
        };
        p.nodes.alloc(
            Node::TupleField(lhs, index),
            lhs.span(p.nodes).to(p.prev_span),
        )
    } else if p.eat(Identifier) {
        p.nodes.alloc(
            Node::StructField(lhs, name),
            lhs.span(p.nodes).to(p.prev_span),
        )
    } else {
        p.error("Expect identifier or integer property name.")
    }
}

fn unary(p: &mut Parser) -> NodeId {
    let op = p.token;
    p.advance();
    let start = p.prev_span;
    let expr = parse_precedence(p, Precedence::Unary as u8);
    match op {
        Bang => p.nodes.alloc(Node::Not(expr), start.to(p.prev_span)),
        Minus => p.nodes.alloc(Node::Negate(expr), start.to(p.prev_span)),
        _ => unreachable!(),
    }
}

fn number(p: &mut Parser) -> NodeId {
    let n = p.scanner.token_text().parse().unwrap();
    p.advance();
    p.nodes.alloc(Node::Number(n), p.prev_span)
}

fn string(p: &mut Parser) -> NodeId {
    let text = p.scanner.token_text();
    let text = text[1..text.len() - 1].to_string();
    p.advance();
    p.nodes.alloc(Node::String(text), p.prev_span)
}

fn boolean(p: &mut Parser) -> NodeId {
    let value = p.token == Token::True;
    p.advance();
    p.nodes.alloc(Node::Bool(value), p.prev_span)
}

fn break_expr(p: &mut Parser) -> NodeId {
    p.advance();
    p.nodes.alloc(Node::Break, p.prev_span)
}

fn continue_expr(p: &mut Parser) -> NodeId {
    p.advance();
    p.nodes.alloc(Node::Continue, p.prev_span)
}

fn return_expr(p: &mut Parser) -> NodeId {
    p.advance();
    let start = p.prev_span;
    let value = if p.token != Semicolon {
        Some(expression(p))
    } else {
        None
    };
    p.nodes.alloc(Node::Return(value), start.to(p.prev_span))
}

fn variable(p: &mut Parser) -> NodeId {
    let name = p.scanner.token_text().to_string();
    p.advance();
    p.nodes.alloc(Node::Variable(name), p.prev_span)
}

fn if_expr(p: &mut Parser) -> NodeId {
    p.advance();
    let start = p.prev_span;
    let cond = expression(p);
    p.consume(OpenBrace, "Expect '{' after 'if' condition.");
    let then_clause = block_expr(p);
    let else_clause = if p.eat(Else) {
        Some(match p.token {
            OpenBrace => {
                p.advance();
                block_expr(p)
            }
            If => if_expr(p),
            _ => p.error("Expect 'if' or '{' after else."),
        })
    } else {
        None
    };
    p.nodes.alloc(
        Node::If(cond, then_clause, else_clause),
        start.to(p.prev_span),
    )
}

fn while_expr(p: &mut Parser) -> NodeId {
    p.advance();
    let start = p.prev_span;
    let cond = expression(p);
    p.consume(OpenBrace, "Expect '{' after 'while' condition.");
    let body = block_expr(p);
    p.nodes
        .alloc(Node::While(cond, body), start.to(p.prev_span))
}

fn struct_expr(p: &mut Parser) -> NodeId {
    p.advance();
    let start = p.prev_span;
    let name = p.scanner.token_text().to_string();
    p.consume(Identifier, "Expect struct name.");
    p.consume(OpenBrace, "Expect '{' after struct name");
    let mut fields = HashMap::new();
    if p.token != CloseBrace {
        loop {
            let name = p.scanner.token_text();
            p.consume(Identifier, "Expect struct field name.");
            let name_span = p.prev_span;
            p.consume(Colon, "Expect ':' after field name.");
            let expr = expression(p);
            if fields.insert(name.to_string(), expr).is_some() {
                p.error_at(&format!("Duplicate field: {name}"), name_span);
            };
            if !p.eat(Comma) {
                break;
            }
        }
    }
    p.consume(CloseBrace, "Expect '}' after struct fields.");
    p.nodes
        .alloc(Node::Struct(name, fields), start.to(p.prev_span))
}

fn block_expr(p: &mut Parser) -> NodeId {
    let start = p.prev_span;
    let mut nodes = Vec::new();
    let mut trailing_expr = None;
    if p.token != CloseBrace && p.token != Eof {
        loop {
            let node = declaration(p);
            if p.token == CloseBrace || p.token == Eof {
                if is_expr(p, node) {
                    trailing_expr = Some(node);
                } else {
                    nodes.push(node);
                }
                break;
            }
            nodes.push(node);
            if is_expr(p, node) && expr_requires_semi(p, node) {
                p.error_at("Expect ';' after expression", node.span(p.nodes));
            }
        }
    }
    p.consume(CloseBrace, "Expect '}' after block.");
    p.nodes
        .alloc(Node::Block(nodes, trailing_expr), start.to(p.prev_span))
}

fn parse_precedence(p: &mut Parser, precedence: u8) -> NodeId {
    crate::stack_safe(|| {
        let Some(mut expr) = parse_prefix(p) else {
            let node = p.error("Expect expression.");
            p.advance();
            return node;
        };

        while precedence <= p.token.precedence() as u8 {
            expr = parse_infix(p, expr);
        }

        expr
    })
}

fn parse_prefix(p: &mut Parser) -> Option<NodeId> {
    Some(match p.token {
        OpenParen => tuple(p),
        Minus | Bang => unary(p),
        NumberLiteral => number(p),
        StringLiteral => string(p),
        True | False => boolean(p),
        Break => break_expr(p),
        Continue => continue_expr(p),
        Return => return_expr(p),
        Identifier => variable(p),
        If => if_expr(p),
        While => while_expr(p),
        Struct => struct_expr(p),
        OpenBrace => {
            p.advance();
            block_expr(p)
        }
        _ => return None,
    })
}

fn parse_infix(p: &mut Parser, lhs: NodeId) -> NodeId {
    match p.token {
        Plus | Minus | Star | Slash | Greater | GreaterEqual | Less | LessEqual | EqualEqual
        | BangEqual | Bar | BarBar | Ampersand | AmpersandAmpersand => binary(p, lhs),
        Equal => assignment(p, lhs),
        OpenParen => call(p, lhs),
        Dot => dot(p, lhs),
        _ => lhs,
    }
}

impl Token {
    fn precedence(self) -> Precedence {
        match self {
            Plus | Minus => Precedence::Term,
            Star | Slash => Precedence::Factor,
            Greater | GreaterEqual | Less | LessEqual => Precedence::Comparison,
            EqualEqual | BangEqual => Precedence::Equality,
            Equal => Precedence::Assignment,
            Bar => Precedence::BitOr,
            BarBar => Precedence::Or,
            Ampersand => Precedence::BitAnd,
            AmpersandAmpersand => Precedence::And,
            OpenParen | Dot => Precedence::Call,
            _ => Precedence::None,
        }
    }
}

fn is_expr(p: &Parser, node: NodeId) -> bool {
    !matches!(
        node.get(p.nodes),
        Node::Let(..) | Node::Semi(..) | Node::Fn(..) | Node::Param(..)
    )
}

fn expr_requires_semi(p: &Parser, node: NodeId) -> bool {
    !matches!(
        node.get(p.nodes),
        Node::If(..) | Node::While(..) | Node::Block(..)
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
enum Precedence {
    None,
    Assignment,
    Or,
    And,
    Equality,
    Comparison,
    BitOr,
    BitAnd,
    Term,
    Factor,
    Unary,
    Call,
}
