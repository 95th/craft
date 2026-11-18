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
            return self.nodes.error_node;
        }
        self.panic_mode = true;
        self.had_error = true;
        print!("Error at '{}': {message}", span.text(self.source));
        self.nodes.error_node
    }
}

fn declaration(p: &mut Parser) -> NodeId {
    match p.token {
        Let => let_declaration(p),
        Fn => fn_declaration(p),
        _ => statement(p),
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
            p.consume(Colon, "Expect ':' after parameter name.");
            let ty = ty(p);
            params.push((name, ty));
            if !p.eat(Comma) {
                break;
            }
        }
    }
    p.consume(CloseParen, "Expect ')' after function parameters.");
    let return_ty = if p.eat(RightArrow) { Some(ty(p)) } else { None };
    p.consume(OpenBrace, "Expect '{' before funciton body");
    let body = block_expr(p);
    p.nodes.alloc(
        Node::Fn(name, params, return_ty, body),
        start.to(p.prev_span),
    )
}

fn statement(p: &mut Parser) -> NodeId {
    match p.token {
        While => while_statement(p),
        _ => expression_statement(p),
    }
}

fn while_statement(p: &mut Parser) -> NodeId {
    p.advance();
    let start = p.prev_span;
    let cond = expression(p);
    p.consume(OpenBrace, "Expect '{' after 'while' condition.");
    let body = block_expr(p);
    p.nodes
        .alloc(Node::While(cond, body), start.to(p.prev_span))
}

fn expression_statement(p: &mut Parser) -> NodeId {
    let expr = expression(p);
    if p.eat(Semicolon) {
        p.nodes.alloc(
            Node::ExprStatement(expr),
            expr.span(p.nodes).to(p.prev_span),
        )
    } else {
        expr
    }
}

fn ty(p: &mut Parser) -> NodeId {
    match p.token {
        Identifier => variable(p),
        _ => p.error("Expected a type"),
    }
}

fn expression(p: &mut Parser) -> NodeId {
    parse_precedence(p, Precedence::Assignment as u8)
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

fn block_expr(p: &mut Parser) -> NodeId {
    let start = p.prev_span;
    let mut nodes = Vec::new();
    while p.token != CloseBrace && p.token != Eof {
        nodes.push(declaration(p));
    }
    p.consume(CloseBrace, "Expect '}' after block.");
    p.nodes.alloc(Node::Block(nodes), start.to(p.prev_span))
}

fn parse_precedence(p: &mut Parser, precedence: u8) -> NodeId {
    let Some(mut expr) = parse_prefix(p) else {
        p.error("Expect expression.");
        p.advance();
        return p.nodes.error_node;
    };

    while precedence <= p.token.precedence() as u8 {
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
        Break => break_expr(p),
        Continue => continue_expr(p),
        Identifier => variable(p),
        If => if_expr(p),
        OpenBrace => block_expr(p),
        _ => return None,
    })
}

fn parse_infix(p: &mut Parser, lhs: NodeId) -> NodeId {
    match p.token {
        Plus | Minus | Star | Slash | Greater | GreaterEqual | Less | LessEqual | EqualEqual
        | BangEqual | Bar | BarBar | Ampersand | AmpersandAmpersand => binary(p, lhs),
        Equal => assignment(p, lhs),
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
            _ => Precedence::None,
        }
    }
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
