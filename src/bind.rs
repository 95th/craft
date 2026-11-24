use rustc_hash::FxHashMap;

use crate::{
    Error,
    arena::{Arena, Id},
    ast::{AstNodes, Expr, ExprId, Stmt, StmtId, Type, TypeId},
    diagnostics::Diagnostics,
    interner::{StrId, StrInterner},
};

type ScopeId = Id<Scope>;

pub enum SymbolKind {
    Let,
    Fn,
    Param,
    TypeParam,
    Struct,
    Builtin,
}

pub struct Symbol {
    pub name: StrId,
    pub kind: SymbolKind,
    pub declaration: StmtId,
}

pub type SymbolId = Id<Symbol>;

pub struct Scope {
    symbols: FxHashMap<StrId, SymbolId>,
    parent: Option<ScopeId>,
}

pub struct StdSymbols {
    pub f64: SymbolId,
    pub i64: SymbolId,
    pub u64: SymbolId,
    pub string: SymbolId,
    pub bool: SymbolId,
    pub print: SymbolId,
    pub assert: SymbolId,
}

pub struct BindCtxt {
    pub symbols: Arena<Symbol>,
    pub std: StdSymbols,
}

pub struct Binder<'a> {
    ast: &'a AstNodes,
    diagnostics: &'a mut Diagnostics,
    scopes: Arena<Scope>,
    type_scope: ScopeId,
    fn_scope: ScopeId,
    value_scope: ScopeId,
    symbols: Arena<Symbol>,
    interner: &'a mut StrInterner,
    has_error: bool,
    std: StdSymbols,
    loop_depth: u32,
    fn_depth: u32,
    local_count: u32,
}

impl<'a> Binder<'a> {
    pub fn new(
        ast: &'a AstNodes,
        interner: &'a mut StrInterner,
        diagnostics: &'a mut Diagnostics,
    ) -> Self {
        Self {
            ast,
            diagnostics,
            scopes: Arena::new(),
            type_scope: ScopeId::MAX,
            fn_scope: ScopeId::MAX,
            value_scope: ScopeId::MAX,
            symbols: Arena::new(),
            interner,
            has_error: false,
            std: StdSymbols {
                f64: SymbolId::MAX,
                i64: SymbolId::MAX,
                u64: SymbolId::MAX,
                string: SymbolId::MAX,
                bool: SymbolId::MAX,
                print: SymbolId::MAX,
                assert: SymbolId::MAX,
            },
            loop_depth: 0,
            fn_depth: 0,
            local_count: 0,
        }
    }

    pub fn bind(mut self, nodes: &[StmtId]) -> Result<BindCtxt, Error> {
        self.type_scope = self.create_scope(None);
        self.fn_scope = self.create_scope(None);
        self.value_scope = self.create_scope(None);

        self.declare_default_types();
        self.declare_default_functions();
        bind_block(&mut self, nodes, None);

        if self.has_error {
            return Err(Error::Bind);
        }
        Ok(BindCtxt {
            symbols: self.symbols,
            std: self.std,
        })
    }

    fn declare_default_types(&mut self) {
        self.std.i64 = self.declare_default_type("i64");
        self.std.u64 = self.declare_default_type("u64");
        self.std.f64 = self.declare_default_type("f64");
        self.std.string = self.declare_default_type("string");
        self.std.bool = self.declare_default_type("bool");
    }

    fn declare_default_type(&mut self, name: &str) -> SymbolId {
        let name = self.interner.intern(name);
        self.declare_type(name, SymbolKind::Builtin, StmtId::MAX)
    }

    fn declare_default_functions(&mut self) {
        self.std.print = self.declare_default_function("print");
        self.std.assert = self.declare_default_function("assert");
    }

    fn declare_default_function(&mut self, name: &str) -> SymbolId {
        let name = self.interner.intern(name);
        self.declare_fn(name, StmtId::MAX)
    }

    fn declare_type(&mut self, name: StrId, kind: SymbolKind, declaration: StmtId) -> SymbolId {
        if let Some(s) = self.scopes[self.type_scope].symbols.get(&name) {
            self.diagnostics.push(
                "already defined in this scope.",
                self.ast.stmts.span(declaration),
            );
            self.has_error = true;
            return *s;
        }
        let symbol = self.declare_symbol(self.type_scope, name, kind, declaration);
        if declaration != StmtId::MAX {
            self.ast.stmts.bind_type(declaration, symbol);
        }
        symbol
    }

    fn declare_value(&mut self, name: StrId, kind: SymbolKind, declaration: StmtId) -> SymbolId {
        if let Some(s) = self.scopes[self.value_scope].symbols.get(&name) {
            self.diagnostics.push(
                "already defined in this scope.",
                self.ast.stmts.span(declaration),
            );
            self.has_error = true;
            return *s;
        }
        let symbol = self.declare_symbol(self.value_scope, name, kind, declaration);
        self.local_count += 1;
        if declaration != StmtId::MAX {
            self.ast.stmts.bind_value(declaration, symbol);
        }
        symbol
    }

    fn declare_fn(&mut self, name: StrId, declaration: StmtId) -> SymbolId {
        if let Some(s) = self.scopes[self.fn_scope].symbols.get(&name) {
            self.diagnostics.push(
                "already defined in this scope.",
                self.ast.stmts.span(declaration),
            );
            self.has_error = true;
            return *s;
        }
        let symbol = self.declare_symbol(self.fn_scope, name, SymbolKind::Fn, declaration);
        if declaration != StmtId::MAX {
            self.ast.stmts.bind_value(declaration, symbol);
        }
        symbol
    }

    fn declare_symbol(
        &mut self,
        scope: ScopeId,
        name: StrId,
        kind: SymbolKind,
        declaration: StmtId,
    ) -> SymbolId {
        let symbol = self.symbols.alloc(Symbol {
            name,
            kind,
            declaration,
        });
        self.scopes[scope].symbols.insert(name, symbol);
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
        let prev_local_count = self.local_count;
        self.value_scope = self.create_scope(if inherit {
            Some(prev_value_scope)
        } else {
            None
        });
        let result = f(self);
        self.value_scope = prev_value_scope;
        self.local_count = prev_local_count;
        result
    }

    fn create_scope(&mut self, parent: Option<ScopeId>) -> ScopeId {
        self.scopes.alloc(Scope {
            symbols: FxHashMap::default(),
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
        Stmt::Fn(f) => {
            b.declare_fn(f.name, stmt);
        }
        Stmt::Struct(s) => {
            b.declare_type(s.name, SymbolKind::Struct, stmt);
        }
        Stmt::TupleStruct(s) => {
            b.declare_type(s.name, SymbolKind::Struct, stmt);
            b.declare_fn(s.name, stmt);
        }
        _ => (),
    }
}

fn bind_statement(b: &mut Binder, stmt: StmtId) {
    match b.ast.stmts[stmt] {
        Stmt::Let { name, ty, init } => {
            bind_expr(b, init);
            if let Some(ty) = ty {
                bind_ty(b, ty);
            }
            b.declare_value(name, SymbolKind::Let, stmt);
        }
        Stmt::Fn(ref f) => b.with_item_scope(|b| {
            for t in f.type_params.iter() {
                bind_statement(b, *t);
            }
            b.with_value_scope(false, |b| {
                let prev_local_count = std::mem::replace(&mut b.local_count, 0);
                for param in f.params.iter() {
                    bind_statement(b, *param);
                }
                if let Some(return_ty) = f.return_ty {
                    bind_ty(b, return_ty);
                }
                let prev_loop_depth = std::mem::replace(&mut b.loop_depth, 0);
                b.fn_depth += 1;
                bind_expr(b, f.body);
                b.fn_depth -= 1;
                b.loop_depth = prev_loop_depth;
                b.local_count = prev_local_count;
            })
        }),
        Stmt::Param { name, ty } => {
            bind_ty(b, ty);
            b.declare_value(name, SymbolKind::Param, stmt);
        }
        Stmt::TypeParam(name) => {
            b.declare_type(name, SymbolKind::TypeParam, stmt);
        }
        Stmt::Expr { expr, .. } => bind_expr(b, expr),
        Stmt::Struct(ref s) => {
            for ty in s.fields.values() {
                bind_ty(b, *ty)
            }
        }
        Stmt::TupleStruct(ref s) => {
            for ty in s.fields.iter() {
                bind_ty(b, *ty)
            }
        }
    }
}

fn bind_expr(b: &mut Binder, node: ExprId) {
    match b.ast.exprs[node] {
        Expr::Variable(name) => {
            if let Some(symbol) = b.lookup_value(name) {
                b.ast.exprs.bind_value(node, symbol);
            } else {
                b.diagnostics
                    .push("not found in this scope.", b.ast.exprs.span(node));
                b.has_error = true;
            }
        }
        Expr::Assign { lhs, rhs } => {
            bind_expr(b, lhs);
            bind_expr(b, rhs);
        }
        Expr::Binary { lhs, rhs, .. } => {
            bind_expr(b, lhs);
            bind_expr(b, rhs);
        }
        Expr::Unit | Expr::Bool(_) | Expr::Number(_) | Expr::Int(_) | Expr::String(_) => {}
        Expr::If {
            cond,
            then_clause,
            else_clause,
        } => {
            bind_expr(b, cond);
            bind_expr(b, then_clause);
            if let Some(else_clause) = else_clause {
                bind_expr(b, else_clause)
            }
        }
        Expr::While { cond, body } => {
            bind_expr(b, cond);
            b.loop_depth += 1;
            bind_expr(b, body);
            b.loop_depth -= 1;
        }
        Expr::Block { ref stmts, expr } => bind_block(b, stmts, expr),
        Expr::Negate(expr) | Expr::Not(expr) => bind_expr(b, expr),
        Expr::Call { callee, ref args } => {
            for arg in args {
                bind_expr(b, *arg);
            }
            bind_expr(b, callee);
        }
        Expr::Break | Expr::Continue => {
            if b.loop_depth == 0 {
                b.diagnostics
                    .push("cannot use outside of a loop", b.ast.exprs.span(node));
                b.has_error = true;
            }
        }
        Expr::Return(expr) => {
            if b.fn_depth == 0 {
                b.diagnostics
                    .push("Cannot use outside of a function", b.ast.exprs.span(node));
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
        Expr::TupleField { obj, .. } => {
            bind_expr(b, obj);
        }
        Expr::Struct(ref s) => {
            if let Some(symbol) = b.lookup_type(s.name) {
                b.ast.exprs.bind_value(node, symbol);
            } else {
                b.diagnostics
                    .push("not found in this scope.", b.ast.exprs.span(node));
                b.has_error = true;
            }

            for expr in s.fields.values() {
                bind_expr(b, *expr);
            }
        }
        Expr::StructField { obj, .. } => {
            bind_expr(b, obj);
        }
        Expr::For {
            init,
            cond,
            incr,
            body,
        } => b.with_value_scope(true, |b| {
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
        Expr::Array(ref elements) => {
            for e in elements {
                bind_expr(b, *e);
            }
        }
        Expr::ArrayRepeat { expr, count } => {
            bind_expr(b, expr);
            bind_expr(b, count);
        }
        Expr::ArrayIndex { array, index } => {
            bind_expr(b, array);
            bind_expr(b, index);
        }
    }
}

fn bind_ty(b: &mut Binder, node: TypeId) {
    match b.ast.types[node] {
        Type::Named(name) => {
            if let Some(symbol) = b.lookup_type(name) {
                b.ast.types.bind_type(node, symbol);
            } else {
                b.diagnostics
                    .push("not found in this scope.", b.ast.types.span(node));
                b.has_error = true;
            }
        }
        Type::FnType { ref params, ret } => {
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
        Type::Array(t) => bind_ty(b, t),
    }
}
