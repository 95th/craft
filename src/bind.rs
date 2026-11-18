use std::collections::HashMap;

use crate::{
    arena::{Arena, Idx},
    ast::{Node, NodeArena, NodeId},
    interner::{StrId, StrInterner},
};

type ScopeId = Idx<Scope>;

pub struct Symbol {
    pub kind: SymbolKind,
    pub name: StrId,
    pub declaration: NodeId,
}

pub enum SymbolKind {
    Fn,
    Var,
    Param,
    Type,
}

pub type SymbolId = Idx<Symbol>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    Fn,
    Loop,
    Block,
}

pub struct Scope {
    kind: ScopeKind,
    symbols: HashMap<StrId, SymbolId>,
    parent: Option<ScopeId>,
}

pub struct BindCtx {
    pub scopes: Arena<Scope>,
    pub symbols: Arena<Symbol>,
    pub interner: StrInterner,
    pub bindings: Vec<Option<SymbolId>>,
    pub number_type: SymbolId,
    pub str_type: SymbolId,
}

pub struct Binder<'a> {
    nodes: &'a NodeArena,
    scopes: Arena<Scope>,
    value_scope: ScopeId,
    type_scope: ScopeId,
    symbols: Arena<Symbol>,
    interner: StrInterner,
    bindings: Vec<Option<SymbolId>>,
    has_error: bool,
    number_type: SymbolId,
    str_type: SymbolId,
}

impl<'a> Binder<'a> {
    pub fn new(nodes: &'a NodeArena) -> Self {
        Self {
            nodes,
            scopes: Arena::new(),
            value_scope: ScopeId::MAX,
            type_scope: ScopeId::MAX,
            symbols: Arena::new(),
            interner: StrInterner::new(),
            bindings: vec![None; nodes.len()],
            has_error: false,
            number_type: SymbolId::MAX,
            str_type: SymbolId::MAX,
        }
    }

    pub fn bind(mut self, nodes: &[NodeId]) -> Option<BindCtx> {
        self.value_scope = self.create_scope(ScopeKind::Block, None);
        self.type_scope = self.create_scope(ScopeKind::Block, None);
        self.declare_default_types();
        bind_scope(&mut self, nodes);
        if self.has_error {
            return None;
        }
        Some(BindCtx {
            scopes: self.scopes,
            symbols: self.symbols,
            interner: self.interner,
            bindings: self.bindings,
            number_type: self.number_type,
            str_type: self.str_type,
        })
    }

    fn declare_default_types(&mut self) {
        self.number_type = self.declare_default_type("number");
        self.str_type = self.declare_default_type("str");
    }

    fn declare_default_type(&mut self, name: &str) -> SymbolId {
        let name = self.interner.intern(name);
        self.declare_type(name, SymbolKind::Type, NodeId::ERROR)
    }

    fn declare_type(&mut self, name: StrId, kind: SymbolKind, declaration: NodeId) -> SymbolId {
        if let Some(s) = self.scopes[self.type_scope].symbols.get(&name) {
            println!(
                "Another type with name '{}' already defined in this scope.",
                self.interner.lookup(name)
            );
            self.has_error = true;
            return *s;
        }
        self.declare_symbol(self.type_scope, name, kind, declaration)
    }

    fn declare_value(&mut self, name: StrId, kind: SymbolKind, declaration: NodeId) -> SymbolId {
        if let Some(s) = self.scopes[self.value_scope].symbols.get(&name) {
            println!(
                "Another variable with name '{}' already defined in this scope.",
                self.interner.lookup(name)
            );
            self.has_error = true;
            return *s;
        }
        self.declare_symbol(self.value_scope, name, kind, declaration)
    }

    fn declare_symbol(
        &mut self,
        scope: ScopeId,
        name: StrId,
        kind: SymbolKind,
        declaration: NodeId,
    ) -> SymbolId {
        let symbol = self.symbols.alloc(Symbol {
            kind,
            name,
            declaration,
        });
        if declaration != NodeId::ERROR {
            self.bindings[declaration.idx()] = Some(symbol);
        }
        self.scopes[scope].symbols.insert(name, symbol);
        symbol
    }

    fn lookup_type(&self, name: StrId) -> Option<SymbolId> {
        let mut curr = Some(self.type_scope);
        while let Some(scope) = curr {
            let scope = &self.scopes[scope];
            if let Some(symbol) = scope.symbols.get(&name) {
                return Some(*symbol);
            }
            curr = scope.parent;
        }
        None
    }

    fn lookup_value(&self, name: StrId) -> Option<SymbolId> {
        let mut curr = Some(self.value_scope);
        while let Some(scope) = curr {
            let scope = &self.scopes[scope];
            if let Some(symbol) = scope.symbols.get(&name) {
                return Some(*symbol);
            }
            if scope.kind == ScopeKind::Fn {
                break;
            }
            curr = scope.parent;
        }
        None
    }

    fn in_value_scope_of(&self, kind: ScopeKind) -> bool {
        let mut curr = Some(self.value_scope);
        while let Some(scope) = curr {
            let scope = &self.scopes[scope];
            if scope.kind == kind {
                return true;
            }
            curr = scope.parent;
        }
        false
    }

    fn with_scope<R>(&mut self, kind: ScopeKind, f: impl FnOnce(&mut Self) -> R) -> R {
        let prev_value_scope = self.value_scope;
        let prev_type_scope = self.type_scope;

        self.value_scope = self.create_scope(kind, Some(prev_value_scope));
        self.type_scope = self.create_scope(kind, Some(prev_type_scope));
        let result = f(self);

        self.value_scope = prev_value_scope;
        self.type_scope = prev_type_scope;
        result
    }

    fn create_scope(&mut self, kind: ScopeKind, parent: Option<ScopeId>) -> ScopeId {
        self.scopes.alloc(Scope {
            kind,
            symbols: HashMap::new(),
            parent,
        })
    }
}

fn bind_scope(b: &mut Binder, nodes: &[NodeId]) {
    for node in nodes {
        declare_item(b, *node);
    }

    for node in nodes {
        bind_statement(b, *node)
    }
}

fn declare_item(b: &mut Binder, node: NodeId) {
    match node.get(b.nodes) {
        Node::Fn(name, ..) => {
            let name = b.interner.intern(name);
            b.declare_value(name, SymbolKind::Fn, node);
        }
        _ => {}
    }
}

fn bind_statement(b: &mut Binder, node: NodeId) {
    match *node.get(b.nodes) {
        Node::Let(ref name, ty, init) => {
            bind_expr(b, init);
            if let Some(ty) = ty {
                bind_ty(b, ty);
            }
            let name = b.interner.intern(name);
            b.declare_value(name, SymbolKind::Var, node);
        }
        Node::Fn(_, ref params, return_ty, body) => b.with_scope(ScopeKind::Fn, |b| {
            for param in params {
                bind_statement(b, *param);
            }
            if let Some(return_ty) = return_ty {
                bind_ty(b, return_ty);
            }
            bind_expr(b, body);
        }),
        Node::Param(ref name, ty) => {
            bind_ty(b, ty);
            let name = b.interner.intern(name);
            b.declare_value(name, SymbolKind::Param, node);
        }
        Node::Semi(expr) => bind_expr(b, expr),
        _ => bind_expr(b, node),
    }
}

fn bind_expr(b: &mut Binder, node: NodeId) {
    match *node.get(b.nodes) {
        Node::Variable(ref name) => {
            let name_id = b.interner.intern(name);
            if let Some(symbol) = b.lookup_value(name_id) {
                b.bindings[node.idx()] = Some(symbol);
            } else {
                println!("Cannot find value '{name}' in this scope.");
                b.has_error = true;
            }
        }
        Node::Assign(left, right) => {
            bind_expr(b, left);
            bind_expr(b, right);
        }
        Node::Binary(left, _op, right) => {
            bind_expr(b, left);
            bind_expr(b, right);
        }
        Node::Bool(_) | Node::Number(_) | Node::String(_) => {}
        Node::If(cond, then_clause, else_clause) => {
            bind_expr(b, cond);
            bind_expr(b, then_clause);
            if let Some(else_clause) = else_clause {
                bind_expr(b, else_clause)
            }
        }
        Node::Block(ref nodes, expr) => b.with_scope(ScopeKind::Block, |b| {
            bind_scope(b, nodes);
            if let Some(expr) = expr {
                bind_expr(b, expr);
            }
        }),
        Node::Negate(expr) | Node::Not(expr) => bind_expr(b, expr),
        Node::Call(callee, ref args) => {
            for arg in args {
                bind_expr(b, *arg);
            }
            bind_expr(b, callee);
        }
        Node::Break | Node::Continue => {
            if !b.in_value_scope_of(ScopeKind::Loop) {
                println!("Cannot '{}' outside of a loop", node.display(b.nodes));
                b.has_error = true;
            }
        }
        Node::Return(expr) => {
            if !b.in_value_scope_of(ScopeKind::Fn) {
                println!("Cannot 'return' outside of a function");
                b.has_error = true;
            }
            if let Some(expr) = expr {
                bind_expr(b, expr);
            }
        }
        ref node => unreachable!("{node:?}"),
    }
}

fn bind_ty(b: &mut Binder, node: NodeId) {
    match *node.get(b.nodes) {
        Node::Variable(ref name) => {
            let name_id = b.interner.intern(name);
            if let Some(symbol) = b.lookup_type(name_id) {
                b.bindings[node.idx()] = Some(symbol);
            } else {
                println!("Cannot find type '{name}' in this scope.");
                b.has_error = true;
            }
        }
        _ => unreachable!(),
    }
}
