use crate::{
    Error, FxIndexMap,
    ast::{AstNodes, BinaryOp, Expr, ExprId, Span, Stmt, StmtId, Type, TypeId},
    diagnostics::Diagnostics,
    scanner::{
        Scanner,
        Token::{self, *},
    },
};

bitflags::bitflags! {
    #[derive(Clone, Copy)]
    struct Restrictions: u8 {
        const DisallowStruct = 1 << 0;
    }
}

struct Checkpoint<'a> {
    scanner: Scanner<'a>,
    exprs: usize,
    stmts: usize,
    types: usize,
    diagnostics: usize,
    token: Token,
    prev_span: Span,
    had_error: bool,
    panic_mode: bool,
    restrictions: Restrictions,
}

pub struct Parser<'a> {
    scanner: Scanner<'a>,
    ast: &'a mut AstNodes,
    diagnostics: &'a mut Diagnostics,
    token: Token,
    prev_span: Span,
    had_error: bool,
    panic_mode: bool,
    restrictions: Restrictions,
}

impl<'a> Parser<'a> {
    pub fn new(source: &'a str, ast: &'a mut AstNodes, diagnostics: &'a mut Diagnostics) -> Self {
        Self {
            scanner: Scanner::new(source),
            ast,
            diagnostics,
            token: Error,
            prev_span: Span::new(0, 0),
            had_error: false,
            panic_mode: false,
            restrictions: Restrictions::empty(),
        }
    }

    pub fn parse(&mut self) -> Result<Vec<StmtId>, Error> {
        self.advance();
        let mut nodes = Vec::new();
        while self.token != Token::Eof {
            nodes.push(declaration(self));
        }
        if self.had_error {
            Err(Error::Parse)
        } else {
            Ok(nodes)
        }
    }

    fn eat(&mut self, token: Token) -> bool {
        if self.token == token {
            self.advance();
            true
        } else {
            false
        }
    }

    fn lookahead<T>(&mut self, callback: impl FnOnce(&mut Self) -> T) -> T {
        let checkpoint = self.checkpoint();
        let result = callback(self);
        self.restore(checkpoint);
        result
    }

    fn with_restriction<T>(
        &mut self,
        restriction: Restrictions,
        callback: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let prev = self.restrictions;
        self.restrictions.insert(restriction);
        let result = callback(self);
        self.restrictions = prev;
        result
    }

    fn without_restriction<T>(
        &mut self,
        restriction: Restrictions,
        callback: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let prev = self.restrictions;
        self.restrictions.remove(restriction);
        let result = callback(self);
        self.restrictions = prev;
        result
    }

    fn checkpoint(&self) -> Checkpoint<'a> {
        Checkpoint {
            scanner: self.scanner.clone(),
            exprs: self.ast.exprs.len(),
            stmts: self.ast.stmts.len(),
            types: self.ast.types.len(),
            diagnostics: self.diagnostics.len(),
            token: self.token,
            prev_span: self.prev_span,
            had_error: self.had_error,
            panic_mode: self.panic_mode,
            restrictions: self.restrictions,
        }
    }

    fn restore(&mut self, c: Checkpoint<'a>) {
        self.scanner = c.scanner;
        self.ast.exprs.truncate(c.exprs);
        self.ast.stmts.truncate(c.stmts);
        self.ast.types.truncate(c.types);
        self.diagnostics.truncate(c.diagnostics);
        self.token = c.token;
        self.prev_span = c.prev_span;
        self.had_error = c.had_error;
        self.panic_mode = c.panic_mode;
        self.restrictions = c.restrictions;
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

    fn error(&mut self, message: &str) {
        self.error_at(message, self.scanner.span());
    }

    fn error_at(&mut self, message: &str, span: Span) {
        if self.panic_mode {
            return;
        }
        self.panic_mode = true;
        self.had_error = true;
        self.diagnostics.push(message, span);
    }
}

fn declaration(p: &mut Parser) -> StmtId {
    match p.token {
        Let => let_declaration(p),
        Fn => fn_declaration(p),
        Struct => struct_declaration(p),
        _ => expression_statement(p),
    }
}

fn let_declaration(p: &mut Parser) -> StmtId {
    p.advance();
    let start = p.prev_span;
    let name = p.scanner.token_text().to_string();
    p.consume(Identifier, "expect identifier");
    let type_annotation = if p.eat(Colon) { Some(ty(p)) } else { None };
    p.consume(Equal, "expect '='");
    let init = expression(p);
    p.consume(Semicolon, "expect ';'");
    p.ast.stmts.alloc(
        Stmt::Let(name, type_annotation, init),
        start.to(p.prev_span),
    )
}

fn type_params(p: &mut Parser) -> Vec<StmtId> {
    let mut type_params = Vec::new();
    if p.eat(Less) {
        loop {
            let name = p.scanner.token_text().to_string();
            p.consume(Identifier, "expect identifier");
            type_params.push(p.ast.stmts.alloc(Stmt::TypeParam(name), p.prev_span));
            if !p.eat(Comma) {
                break;
            }
        }
        p.consume(Greater, "Expect '>'");
    }
    type_params
}

fn fn_declaration(p: &mut Parser) -> StmtId {
    p.advance();
    let start = p.prev_span;
    let name = p.scanner.token_text().to_string();
    p.consume(Identifier, "expect identifier");
    let type_params = type_params(p);
    p.consume(OpenParen, "expect '('");
    let mut params = Vec::new();
    if p.token != CloseParen {
        loop {
            let name = p.scanner.token_text().to_string();
            p.consume(Identifier, "expect identifier");
            let param_span = p.prev_span;
            p.consume(Colon, "expect ':'");
            let ty = ty(p);
            params.push(
                p.ast
                    .stmts
                    .alloc(Stmt::Param(name, ty), param_span.to(p.prev_span)),
            );
            if !p.eat(Comma) {
                break;
            }
        }
    }
    p.consume(CloseParen, "expect ')'");
    let return_ty = if p.eat(RightArrow) { Some(ty(p)) } else { None };
    p.consume(OpenBrace, "expect '{'");
    let body = block_expr(p);
    p.ast.stmts.alloc(
        Stmt::Fn(name, type_params, params, return_ty, body),
        start.to(p.prev_span),
    )
}

fn struct_declaration(p: &mut Parser) -> StmtId {
    p.advance();
    let start = p.prev_span;
    let name = p.scanner.token_text().to_string();
    p.consume(Identifier, "expect identifier");
    p.consume(OpenBrace, "expect '{'");
    let mut fields = FxIndexMap::default();
    if p.token != CloseBrace {
        loop {
            let name = p.scanner.token_text();
            p.consume(Identifier, "expect identifier");
            let name_span = p.prev_span;
            p.consume(Colon, "expect ':'");
            let ty = ty(p);
            if fields.insert(name.to_string(), ty).is_some() {
                p.error_at("duplicate field", name_span);
            };
            if !p.eat(Comma) {
                break;
            }
        }
    }
    p.consume(CloseBrace, "expect '}'");
    p.ast
        .stmts
        .alloc(Stmt::Struct(name, fields), start.to(p.prev_span))
}

fn expression_statement(p: &mut Parser) -> StmtId {
    let start = p.scanner.span();
    let expr = expression(p);
    let semi = p.eat(Semicolon);
    p.ast
        .stmts
        .alloc(Stmt::Expr(expr, semi), start.to(p.prev_span))
}

fn ty(p: &mut Parser) -> TypeId {
    match p.token {
        Identifier => named_ty(p),
        Fn => fn_ty(p),
        OpenParen => tuple_ty(p),
        _ => {
            p.error("unexpected token");
            TypeId::MAX
        }
    }
}

fn named_ty(p: &mut Parser) -> TypeId {
    let name = p.scanner.token_text().to_string();
    p.advance();
    p.ast.types.alloc(Type::Named(name), p.prev_span)
}

fn fn_ty(p: &mut Parser) -> TypeId {
    p.advance();
    let start = p.prev_span;
    let type_params = type_params(p);
    p.consume(OpenParen, "expect '('");
    let mut params = Vec::new();
    if p.token != CloseParen {
        loop {
            params.push(ty(p));
            if !p.eat(Comma) {
                break;
            }
        }
    }
    p.consume(CloseParen, "expect ')'");
    let return_ty = if p.eat(RightArrow) { Some(ty(p)) } else { None };
    p.ast.types.alloc(
        Type::FnType(type_params, params, return_ty),
        start.to(p.prev_span),
    )
}

fn tuple_ty(p: &mut Parser) -> TypeId {
    p.advance();
    let start = p.prev_span;
    if p.eat(CloseParen) {
        return p.ast.types.alloc(Type::Unit, start.to(p.prev_span));
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
    p.consume(CloseParen, "expect ')'");
    p.ast.types.alloc(Type::Tuple(types), start.to(p.prev_span))
}

fn expression(p: &mut Parser) -> ExprId {
    parse_precedence(p, Precedence::Assignment as u8)
}

fn tuple(p: &mut Parser) -> ExprId {
    p.without_restriction(Restrictions::DisallowStruct, |p| {
        p.advance();
        let start = p.prev_span;
        if p.eat(CloseParen) {
            return p.ast.exprs.alloc(Expr::Unit, start.to(p.prev_span));
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
            p.consume(CloseParen, "expect ')'");
            return p.ast.exprs.alloc(Expr::Tuple(exprs), start.to(p.prev_span));
        }

        p.consume(CloseParen, "expect ')'");
        expr
    })
}

fn binary(p: &mut Parser, lhs: ExprId) -> ExprId {
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
    p.ast.exprs.alloc(
        Expr::Binary(lhs, op, rhs),
        p.ast.exprs.span(lhs).to(p.prev_span),
    )
}

fn assignment(p: &mut Parser, lhs: ExprId) -> ExprId {
    p.advance();
    let rhs = expression(p);
    let lhs_span = p.ast.exprs.span(lhs);
    match p.ast.exprs[lhs] {
        Expr::Variable(..) | Expr::StructField(..) | Expr::TupleField(..) => p
            .ast
            .exprs
            .alloc(Expr::Assign(lhs, rhs), lhs_span.to(p.prev_span)),
        _ => {
            p.error_at("invalid assignment target", lhs_span);
            ExprId::MAX
        }
    }
}

fn call(p: &mut Parser, lhs: ExprId) -> ExprId {
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
    p.consume(CloseParen, "expect ')'");
    p.ast
        .exprs
        .alloc(Expr::Call(lhs, args), p.ast.exprs.span(lhs).to(p.prev_span))
}

fn dot(p: &mut Parser, lhs: ExprId) -> ExprId {
    p.advance();
    let name = p.scanner.token_text();
    if p.eat(NumberLiteral) {
        let start = p.ast.exprs.span(lhs);
        if let Some((a, b)) = name.split_once('.') {
            let a_index = a.parse().unwrap();
            let a_span = Span::new(
                p.prev_span.start as usize,
                p.prev_span.start as usize + a.len(),
            );
            let a_expr = p
                .ast
                .exprs
                .alloc(Expr::TupleField(lhs, a_index), start.to(a_span));

            let b_index = b.parse().unwrap();
            p.ast
                .exprs
                .alloc(Expr::TupleField(a_expr, b_index), start.to(p.prev_span))
        } else {
            let index = name.parse().unwrap();
            p.ast
                .exprs
                .alloc(Expr::TupleField(lhs, index), start.to(p.prev_span))
        }
    } else if p.eat(Identifier) {
        p.ast.exprs.alloc(
            Expr::StructField(lhs, name.to_string()),
            p.ast.exprs.span(lhs).to(p.prev_span),
        )
    } else {
        p.error("unexpected token");
        ExprId::MAX
    }
}

fn unary(p: &mut Parser) -> ExprId {
    let op = p.token;
    p.advance();
    let start = p.prev_span;
    let expr = parse_precedence(p, Precedence::Unary as u8);
    match op {
        Bang => p.ast.exprs.alloc(Expr::Not(expr), start.to(p.prev_span)),
        Minus => p.ast.exprs.alloc(Expr::Negate(expr), start.to(p.prev_span)),
        _ => unreachable!(),
    }
}

fn number(p: &mut Parser) -> ExprId {
    let n = p.scanner.token_text().parse().unwrap();
    p.advance();
    p.ast.exprs.alloc(Expr::Number(n), p.prev_span)
}

fn string(p: &mut Parser) -> ExprId {
    let text = p.scanner.token_text();
    let text = text[1..text.len() - 1].to_string();
    p.advance();
    p.ast.exprs.alloc(Expr::String(text), p.prev_span)
}

fn boolean(p: &mut Parser) -> ExprId {
    let value = p.token == Token::True;
    p.advance();
    p.ast.exprs.alloc(Expr::Bool(value), p.prev_span)
}

fn break_expr(p: &mut Parser) -> ExprId {
    p.advance();
    p.ast.exprs.alloc(Expr::Break, p.prev_span)
}

fn continue_expr(p: &mut Parser) -> ExprId {
    p.advance();
    p.ast.exprs.alloc(Expr::Continue, p.prev_span)
}

fn return_expr(p: &mut Parser) -> ExprId {
    p.advance();
    let start = p.prev_span;
    let value = if p.token != Semicolon {
        Some(expression(p))
    } else {
        None
    };
    p.ast
        .exprs
        .alloc(Expr::Return(value), start.to(p.prev_span))
}

fn variable(p: &mut Parser) -> ExprId {
    let name = p.scanner.token_text().to_string();
    p.advance();

    if !p.restrictions.contains(Restrictions::DisallowStruct)
        && p.token == OpenBrace
        && p.lookahead(|p| {
            p.advance();
            p.eat(CloseBrace) || p.eat(Identifier) && p.eat(Colon)
        })
    {
        let start = p.prev_span;
        p.advance(); // eat the '{'
        let mut fields = FxIndexMap::default();
        if p.token != CloseBrace {
            loop {
                let name = p.scanner.token_text();
                p.consume(Identifier, "expect identifier");
                let name_span = p.prev_span;
                p.consume(Colon, "expect ':'");
                let expr = expression(p);
                if fields.insert(name.to_string(), expr).is_some() {
                    p.error_at("duplicate field", name_span);
                };
                if !p.eat(Comma) {
                    break;
                }
            }
        }
        p.consume(CloseBrace, "expect '}'");
        return p
            .ast
            .exprs
            .alloc(Expr::Struct(name, fields), start.to(p.prev_span));
    }

    p.ast.exprs.alloc(Expr::Variable(name), p.prev_span)
}

fn if_expr(p: &mut Parser) -> ExprId {
    p.advance();
    let start = p.prev_span;
    let cond = p.with_restriction(Restrictions::DisallowStruct, expression);
    p.consume(OpenBrace, "expect '{'");
    let then_clause = block_expr(p);
    let else_clause = if p.eat(Else) {
        Some(match p.token {
            OpenBrace => {
                p.advance();
                block_expr(p)
            }
            If => if_expr(p),
            _ => {
                p.error("unexpected token");
                ExprId::MAX
            }
        })
    } else {
        None
    };
    p.ast.exprs.alloc(
        Expr::If(cond, then_clause, else_clause),
        start.to(p.prev_span),
    )
}

fn while_expr(p: &mut Parser) -> ExprId {
    p.advance();
    let start = p.prev_span;
    let cond = p.with_restriction(Restrictions::DisallowStruct, expression);
    p.consume(OpenBrace, "expect '{'");
    let body = block_expr(p);
    p.ast
        .exprs
        .alloc(Expr::While(cond, body), start.to(p.prev_span))
}

fn for_expr(p: &mut Parser) -> ExprId {
    p.advance();
    let start = p.prev_span;
    p.consume(OpenParen, "expect '('");
    let init = if p.eat(Semicolon) {
        None
    } else if p.token == Let {
        Some(let_declaration(p))
    } else {
        let expr = expression(p);
        p.consume(Semicolon, "expect ';'");
        let stmt = p.ast.stmts.alloc(
            Stmt::Expr(expr, true),
            p.ast.exprs.span(expr).to(p.prev_span),
        );
        Some(stmt)
    };
    let cond = if p.eat(Semicolon) {
        None
    } else {
        let expr = expression(p);
        p.consume(Semicolon, "expect ';'");
        Some(expr)
    };
    let incr = if p.eat(CloseParen) {
        None
    } else {
        let expr = expression(p);
        p.consume(CloseParen, "expect ')'");
        Some(expr)
    };
    p.consume(OpenBrace, "expect '{'");
    let body = block_expr(p);
    p.ast
        .exprs
        .alloc(Expr::For(init, cond, incr, body), start.to(p.prev_span))
}

fn block_expr(p: &mut Parser) -> ExprId {
    crate::stack_safe(|| {
        let start = p.prev_span;
        let mut nodes = Vec::new();
        let mut trailing_expr = None;
        if p.token != CloseBrace && p.token != Eof {
            loop {
                let stmt = declaration(p);
                if p.token == CloseBrace || p.token == Eof {
                    if let Stmt::Expr(expr, false) = p.ast.stmts[stmt] {
                        trailing_expr = Some(expr);
                    } else {
                        nodes.push(stmt);
                    }
                    break;
                }
                nodes.push(stmt);
                if expr_requires_semi(p, stmt) {
                    p.error_at("expect ';'", p.ast.stmts.span(stmt));
                }
            }
        }
        p.consume(CloseBrace, "expect '}'");
        p.ast
            .exprs
            .alloc(Expr::Block(nodes, trailing_expr), start.to(p.prev_span))
    })
}

fn parse_precedence(p: &mut Parser, precedence: u8) -> ExprId {
    crate::stack_safe(|| {
        let Some(mut expr) = parse_prefix(p) else {
            p.error("unexpected token");
            p.advance();
            return ExprId::MAX;
        };

        while precedence <= p.token.precedence() as u8 {
            expr = parse_infix(p, expr);
        }

        expr
    })
}

fn parse_prefix(p: &mut Parser) -> Option<ExprId> {
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
        For => for_expr(p),
        OpenBrace => {
            p.advance();
            block_expr(p)
        }
        _ => return None,
    })
}

fn parse_infix(p: &mut Parser, lhs: ExprId) -> ExprId {
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

fn expr_requires_semi(p: &Parser, stmt: StmtId) -> bool {
    if let Stmt::Expr(expr, false) = p.ast.stmts[stmt] {
        !matches!(
            p.ast.exprs[expr],
            Expr::If(..) | Expr::While(..) | Expr::Block(..) | Expr::For(..)
        )
    } else {
        false
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
