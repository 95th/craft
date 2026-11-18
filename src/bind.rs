use std::collections::HashMap;

use crate::{
    ast::{Node, NodeArena, NodeId},
    interner::{StrId, StrInterner},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ScopeId(usize);

impl ScopeId {
    const ERROR: Self = Self(usize::MAX);
}

pub struct Symbol {
    pub kind: SymbolKind,
    pub name: StrId,
    pub declaration: NodeId,
}

pub enum SymbolKind {
    Fn,
    Var,
    Param,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolId(u32);

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Fn,
    Loop,
    Block,
}

struct Scope {
    kind: ScopeKind,
    symbols: HashMap<StrId, SymbolId>,
    parent: Option<ScopeId>,
}

pub struct Binder<'a> {
    nodes: &'a NodeArena,
    scopes: Vec<Scope>,
    value_scope: ScopeId,
    type_scope: ScopeId,
    symbols: Vec<Symbol>,
    interner: StrInterner,
    bindings: Vec<Option<SymbolId>>,
    has_error: bool,
}

impl<'a> Binder<'a> {
    pub fn new(nodes: &'a NodeArena) -> Self {
        Self {
            nodes,
            scopes: Vec::new(),
            value_scope: ScopeId::ERROR,
            type_scope: ScopeId::ERROR,
            symbols: Vec::new(),
            interner: StrInterner::new(),
            bindings: vec![None; nodes.len()],
            has_error: false,
        }
    }

    pub fn bind(&mut self, nodes: &[NodeId]) {
        self.value_scope = self.create_scope(ScopeKind::Block, None);
        self.type_scope = self.create_scope(ScopeKind::Block, None);
        bind_scope(self, nodes);
    }

    fn declare_type(&mut self, name: StrId, symbol: Symbol) {
        if self.lookup(self.type_scope, name).is_some() {
            println!(
                "Another type with name '{}' already defined in this scope.",
                self.interner.lookup(name)
            );
            self.has_error = true;
            return;
        }
        self.declare_symbol(self.type_scope, name, symbol);
    }

    fn declare_value(&mut self, name: StrId, symbol: Symbol) {
        if self.lookup(self.value_scope, name).is_some() {
            println!(
                "Another variable with name '{}' already defined in this scope.",
                self.interner.lookup(name)
            );
            self.has_error = true;
            return;
        }
        self.declare_symbol(self.value_scope, name, symbol);
    }

    fn declare_symbol(&mut self, scope: ScopeId, name: StrId, symbol: Symbol) {
        let symbol_id = self.create_symbol(symbol);
        self.scopes[scope.0 as usize]
            .symbols
            .insert(name, symbol_id);
    }

    fn lookup_type(&self, name: StrId) -> Option<SymbolId> {
        self.lookup(self.type_scope, name)
    }

    fn lookup_value(&self, name: StrId) -> Option<SymbolId> {
        self.lookup(self.value_scope, name)
    }

    fn lookup(&self, scope: ScopeId, name: StrId) -> Option<SymbolId> {
        let mut curr = Some(scope);
        while let Some(scope) = curr {
            let scope = &self.scopes[scope.0 as usize];
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
            let scope = &self.scopes[scope.0 as usize];
            if scope.kind == kind {
                return true;
            }
            curr = scope.parent;
        }
        false
    }

    fn create_symbol(&mut self, symbol: Symbol) -> SymbolId {
        let id = self.symbols.len();
        self.symbols.push(symbol);
        SymbolId(id as u32)
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
        let id = ScopeId(self.scopes.len());
        self.scopes.push(Scope {
            kind,
            symbols: HashMap::new(),
            parent,
        });
        id
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
            b.declare_value(
                name,
                Symbol {
                    kind: SymbolKind::Fn,
                    name,
                    declaration: node,
                },
            )
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
            b.declare_value(
                name,
                Symbol {
                    kind: SymbolKind::Var,
                    name,
                    declaration: node,
                },
            );
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
            b.declare_value(
                name,
                Symbol {
                    kind: SymbolKind::Param,
                    name,
                    declaration: node,
                },
            );
        }
        Node::ExprStatement(expr) => bind_expr(b, expr),
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
        Node::Block(ref nodes) => b.with_scope(ScopeKind::Block, |b| bind_scope(b, nodes)),
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
