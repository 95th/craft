use std::collections::HashMap;

use crate::{
    Error,
    arena::{Arena, Bindings, Idx},
    ast::{AstNodes, Expr, ExprId, Stmt, StmtId, Type, TypeId},
    diagnostics::Diagnostic,
    interner::{StrId, StrInterner},
};

type ScopeId = Idx<Scope>;

pub struct Symbol {
    pub name: StrId,
}

pub type SymbolId = Idx<Symbol>;

pub struct Scope {
    symbols: HashMap<StrId, SymbolId>,
    parent: Option<ScopeId>,
}

pub struct BindCtx {
    pub scopes: Arena<Scope>,
    pub symbols: Arena<Symbol>,
    pub interner: StrInterner,
    pub expr_bindings: Bindings<Expr>,
    pub stmt_bindings: Bindings<Stmt>,
    pub type_bindings: Bindings<Type>,
    pub number_type: SymbolId,
    pub str_type: SymbolId,
    pub bool_type: SymbolId,
}

pub struct Binder<'a> {
    ast: &'a AstNodes,
    diagnostics: &'a mut Vec<Diagnostic>,
    scopes: Arena<Scope>,
    type_scope: ScopeId,
    fn_scope: ScopeId,
    value_scope: ScopeId,
    symbols: Arena<Symbol>,
    interner: StrInterner,
    expr_bindings: Bindings<Expr>,
    stmt_bindings: Bindings<Stmt>,
    type_bindings: Bindings<Type>,
    has_error: bool,
    number_type: SymbolId,
    str_type: SymbolId,
    bool_type: SymbolId,
    loop_depth: u16,
    fn_depth: u16,
}

impl<'a> Binder<'a> {
    pub fn new(ast: &'a AstNodes, diagnostics: &'a mut Vec<Diagnostic>) -> Self {
        Self {
            ast,
            diagnostics,
            scopes: Arena::new(),
            type_scope: ScopeId::MAX,
            fn_scope: ScopeId::MAX,
            value_scope: ScopeId::MAX,
            symbols: Arena::new(),
            interner: StrInterner::new(),
            expr_bindings: ast.exprs.new_bindings(),
            stmt_bindings: ast.stmts.new_bindings(),
            type_bindings: ast.types.new_bindings(),
            has_error: false,
            number_type: SymbolId::MAX,
            str_type: SymbolId::MAX,
            bool_type: SymbolId::MAX,
            loop_depth: 0,
            fn_depth: 0,
        }
    }

    pub fn bind(mut self, nodes: &[StmtId]) -> Result<BindCtx, Error> {
        self.type_scope = self.create_scope(None);
        self.fn_scope = self.create_scope(None);
        self.value_scope = self.create_scope(None);

        self.declare_default_types();
        bind_block(&mut self, nodes, None);

        if self.has_error {
            return Err(Error::Bind);
        }
        Ok(BindCtx {
            scopes: self.scopes,
            symbols: self.symbols,
            interner: self.interner,
            expr_bindings: self.expr_bindings,
            stmt_bindings: self.stmt_bindings,
            type_bindings: self.type_bindings,
            number_type: self.number_type,
            str_type: self.str_type,
            bool_type: self.bool_type,
        })
    }

    fn declare_default_types(&mut self) {
        self.number_type = self.declare_default_type("number");
        self.str_type = self.declare_default_type("str");
        self.bool_type = self.declare_default_type("bool");
    }

    fn declare_default_type(&mut self, name: &str) -> SymbolId {
        let name = self.interner.intern(name);
        self.declare_type(name, StmtId::MAX)
    }

    fn declare_type(&mut self, name: StrId, declaration: StmtId) -> SymbolId {
        if let Some(s) = self.scopes[self.type_scope].symbols.get(&name) {
            self.diagnostics.push(Diagnostic {
                span: self.ast.stmts.span(declaration),
                message: format!(
                    "Another type with name '{}' already defined in this scope.",
                    self.interner.lookup(name)
                ),
            });
            self.has_error = true;
            return *s;
        }
        self.declare_symbol(self.type_scope, name, declaration)
    }

    fn declare_value(&mut self, name: StrId, declaration: StmtId) -> SymbolId {
        if let Some(s) = self.scopes[self.value_scope].symbols.get(&name) {
            self.diagnostics.push(Diagnostic {
                span: self.ast.stmts.span(declaration),
                message: format!(
                    "Another variable with name '{}' already defined in this scope.",
                    self.interner.lookup(name)
                ),
            });
            self.has_error = true;
            return *s;
        }
        self.declare_symbol(self.value_scope, name, declaration)
    }

    fn declare_fn(&mut self, name: StrId, declaration: StmtId) -> SymbolId {
        if let Some(s) = self.scopes[self.fn_scope].symbols.get(&name) {
            self.diagnostics.push(Diagnostic {
                span: self.ast.stmts.span(declaration),
                message: format!(
                    "Another function with name '{}' already defined in this scope.",
                    self.interner.lookup(name)
                ),
            });
            self.has_error = true;
            return *s;
        }
        self.declare_symbol(self.fn_scope, name, declaration)
    }

    fn declare_symbol(&mut self, scope: ScopeId, name: StrId, declaration: StmtId) -> SymbolId {
        let symbol = self.symbols.alloc(Symbol { name });
        self.scopes[scope].symbols.insert(name, symbol);
        if declaration != StmtId::MAX {
            self.stmt_bindings.bind(declaration, symbol);
        }
        symbol
    }

    fn lookup_type(&self, name: StrId) -> Option<SymbolId> {
        self.lookup(self.type_scope, name)
    }

    fn lookup_value(&self, name: StrId) -> Option<SymbolId> {
        self.lookup(self.value_scope, name)
            .or_else(|| self.lookup(self.fn_scope, name))
    }

    fn lookup(&self, scope: ScopeId, name: StrId) -> Option<SymbolId> {
        let mut curr = Some(scope);
        while let Some(scope) = curr {
            let scope = &self.scopes[scope];
            if let Some(symbol) = scope.symbols.get(&name) {
                return Some(*symbol);
            }
            curr = scope.parent;
        }
        None
    }

    fn with_item_scope<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let prev_type_scope = self.type_scope;
        let prev_fn_scope = self.fn_scope;
        self.type_scope = self.create_scope(Some(prev_type_scope));
        self.fn_scope = self.create_scope(Some(prev_fn_scope));
        let result = f(self);
        self.fn_scope = prev_fn_scope;
        self.type_scope = prev_type_scope;
        result
    }

    fn with_value_scope<R>(&mut self, inherit: bool, f: impl FnOnce(&mut Self) -> R) -> R {
        let prev_value_scope = self.value_scope;
        self.value_scope = self.create_scope(if inherit {
            Some(prev_value_scope)
        } else {
            None
        });
        let result = f(self);
        self.value_scope = prev_value_scope;
        result
    }

    fn create_scope(&mut self, parent: Option<ScopeId>) -> ScopeId {
        self.scopes.alloc(Scope {
            symbols: HashMap::new(),
            parent,
        })
    }
}

fn bind_block(b: &mut Binder, nodes: &[StmtId], expr: Option<ExprId>) {
    crate::stack_safe(|| {
        b.with_item_scope(|b| {
            for node in nodes {
                declare_item(b, *node);
            }
            b.with_value_scope(true, |b| {
                for node in nodes {
                    bind_statement(b, *node)
                }
                if let Some(expr) = expr {
                    bind_expr(b, expr);
                }
            })
        })
    })
}

fn declare_item(b: &mut Binder, stmt: StmtId) {
    match &b.ast.stmts[stmt] {
        Stmt::Fn(name, ..) => {
            let name = b.interner.intern(name);
            b.declare_fn(name, stmt);
        }
        Stmt::Struct(name, ..) => {
            let name = b.interner.intern(name);
            b.declare_type(name, stmt);
        }
        _ => (),
    }
}

fn bind_statement(b: &mut Binder, stmt: StmtId) {
    match b.ast.stmts[stmt] {
        Stmt::Let(ref name, ty, init) => {
            let name = b.interner.intern(name);
            bind_expr(b, init);
            if let Some(ty) = ty {
                bind_ty(b, ty);
            }
            b.declare_value(name, stmt);
        }
        Stmt::Fn(_, ref params, return_ty, body) => b.with_value_scope(false, |b| {
            for param in params {
                bind_statement(b, *param);
            }
            if let Some(return_ty) = return_ty {
                bind_ty(b, return_ty);
            }
            let prev_loop_depth = std::mem::replace(&mut b.loop_depth, 0);
            b.fn_depth += 1;
            bind_expr(b, body);
            b.fn_depth -= 1;
            b.loop_depth = prev_loop_depth;
        }),
        Stmt::Param(ref name, ty) => {
            bind_ty(b, ty);
            let name = b.interner.intern(name);
            b.declare_value(name, stmt);
        }
        Stmt::Expr(expr, _) => bind_expr(b, expr),
        Stmt::Struct(_, ref fields) => {
            for (_, ty) in fields {
                bind_ty(b, *ty)
            }
        }
    }
}

fn bind_expr(b: &mut Binder, node: ExprId) {
    match b.ast.exprs[node] {
        Expr::Variable(ref name) => {
            let name_id = b.interner.intern(name);
            if let Some(symbol) = b.lookup_value(name_id) {
                b.expr_bindings.bind(node, symbol);
            } else {
                b.diagnostics.push(Diagnostic {
                    span: b.ast.exprs.span(node),
                    message: format!("Cannot find value '{name}' in this scope."),
                });
                b.has_error = true;
            }
        }
        Expr::Assign(left, right) => {
            bind_expr(b, left);
            bind_expr(b, right);
        }
        Expr::Binary(left, _op, right) => {
            bind_expr(b, left);
            bind_expr(b, right);
        }
        Expr::Bool(_) | Expr::Number(_) | Expr::String(_) => {}
        Expr::If(cond, then_clause, else_clause) => {
            bind_expr(b, cond);
            bind_expr(b, then_clause);
            if let Some(else_clause) = else_clause {
                bind_expr(b, else_clause)
            }
        }
        Expr::While(cond, body) => {
            bind_expr(b, cond);
            b.loop_depth += 1;
            bind_expr(b, body);
            b.loop_depth -= 1;
        }
        Expr::Block(ref nodes, expr) => bind_block(b, nodes, expr),
        Expr::Negate(expr) | Expr::Not(expr) => bind_expr(b, expr),
        Expr::Call(callee, ref args) => {
            for arg in args {
                bind_expr(b, *arg);
            }
            bind_expr(b, callee);
        }
        Expr::Break | Expr::Continue => {
            if b.loop_depth == 0 {
                b.diagnostics.push(Diagnostic {
                    span: b.ast.exprs.span(node),
                    message: "Cannot use outside of a loop".into(),
                });
                b.has_error = true;
            }
        }
        Expr::Unit => {}
        Expr::Return(expr) => {
            if b.fn_depth == 0 {
                b.diagnostics.push(Diagnostic {
                    span: b.ast.exprs.span(node),
                    message: "Cannot 'return' outside of a function".into(),
                });
                b.has_error = true;
            }
            if let Some(expr) = expr {
                bind_expr(b, expr);
            }
        }
        Expr::Tuple(ref nodes) => {
            for node in nodes {
                bind_expr(b, *node);
            }
        }
        Expr::TupleField(target, _) => {
            bind_expr(b, target);
        }
        Expr::Struct(ref name, ..) => {
            let name_id = b.interner.intern(name);
            if let Some(symbol) = b.lookup_type(name_id) {
                b.expr_bindings.bind(node, symbol);
            } else {
                b.diagnostics.push(Diagnostic {
                    span: b.ast.exprs.span(node),
                    message: format!("Cannot find value '{name}' in this scope."),
                });
                b.has_error = true;
            }
        }
        Expr::StructField(target, _) => {
            bind_expr(b, target);
        }
        Expr::For(init, cond, incr, body) => b.with_value_scope(true, |b| {
            if let Some(init) = init {
                bind_statement(b, init);
            }
            if let Some(cond) = cond {
                bind_expr(b, cond);
            }
            if let Some(incr) = incr {
                bind_expr(b, incr);
            }
            b.loop_depth += 1;
            bind_expr(b, body);
            b.loop_depth -= 1;
        }),
    }
}

fn bind_ty(b: &mut Binder, node: TypeId) {
    match b.ast.types[node] {
        Type::Named(ref name) => {
            let name_id = b.interner.intern(name);
            if let Some(symbol) = b.lookup_type(name_id) {
                b.type_bindings.bind(node, symbol);
            } else {
                b.diagnostics.push(Diagnostic {
                    span: b.ast.types.span(node),
                    message: format!("Cannot find type '{name}' in this scope."),
                });
                b.has_error = true;
            }
        }
        Type::FnType(ref params, ret) => {
            for p in params {
                bind_ty(b, *p);
            }
            if let Some(ret) = ret {
                bind_ty(b, ret);
            }
        }
        Type::Unit => {}
        Type::Tuple(ref types) => {
            for t in types {
                bind_ty(b, *t);
            }
        }
    }
}
