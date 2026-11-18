use core::fmt;
use std::collections::HashMap;

use crate::{
    arena::Arena,
    ast::{BinaryOp, Node, NodeArena, NodeId, Span},
    bind::{BindCtx, SymbolId},
    types::{CommonTypes, Type, TypeId},
};

pub struct TypeChecker<'a> {
    source: &'a str,
    nodes: &'a NodeArena,
    bind_ctx: BindCtx,
    types: Arena<Type>,
    common_types: CommonTypes,
    has_error: bool,
    pending_unifications: Vec<(TypeId, TypeId, Span)>,
    containing_return_ty: Option<TypeId>,
    binding_types: HashMap<SymbolId, TypeId>,
}

impl<'a> TypeChecker<'a> {
    pub fn new(source: &'a str, nodes: &'a NodeArena, bind_ctx: BindCtx) -> Self {
        let mut types = Arena::new();
        let common_types = CommonTypes::new(&mut types);
        Self {
            source,
            nodes,
            bind_ctx,
            types,
            common_types,
            has_error: false,
            pending_unifications: Vec::new(),
            containing_return_ty: None,
            binding_types: HashMap::new(),
        }
    }

    pub fn check(&mut self, nodes: &[NodeId]) {
        self.binding_types
            .insert(self.bind_ctx.number_type, self.common_types.number);
        self.binding_types
            .insert(self.bind_ctx.str_type, self.common_types.string);
        check_scope(self, nodes);
    }

    fn unify(&mut self, lhs: TypeId, rhs: TypeId, span: Span) {
        if lhs == rhs || self.types[lhs] == self.types[rhs] {
            return;
        }
        match (&self.types[lhs], &self.types[rhs]) {
            (Type::Named(a), Type::Named(b)) => {
                let a_name = self.bind_ctx.symbols[*a].name;
                let b_name = self.bind_ctx.symbols[*b].name;
                if a_name == b_name {
                    self.error(
                        "Type mismatch. Both types have the same name but they're not the same.",
                        span,
                    );
                } else {
                    self.error(
                        format!(
                            "Type mismatch: Expected '{}' but got '{}'",
                            self.bind_ctx.interner.lookup(a_name),
                            self.bind_ctx.interner.lookup(b_name),
                        ),
                        span,
                    );
                }
            }
            (Type::Var, Type::Var) => {
                self.pending_unifications.push((lhs, rhs, span));
            }
            (Type::Var, other) => self.types[lhs] = other.clone(),
            (other, Type::Var) => self.types[rhs] = other.clone(),
            _ => {
                self.error(
                    format!(
                        "Type mismatch: Expected '{}' but got '{}'",
                        self.display(lhs),
                        self.display(rhs)
                    ),
                    span,
                );
            }
        }
    }

    fn display(&self, ty: TypeId) -> impl fmt::Display {
        ty.display(&self.types, &self.bind_ctx.symbols, &self.bind_ctx.interner)
    }

    fn error(&mut self, message: impl Into<String>, span: Span) {
        self.has_error = true;
        println!(
            "Type mismatch at '{}': {}",
            span.text(self.source),
            message.into()
        );
    }
}

fn check_scope(c: &mut TypeChecker, nodes: &[NodeId]) {
    for node in nodes {
        declare_item(c, *node);
    }

    for node in nodes {
        check_node(c, *node);
    }
}

fn declare_item(c: &mut TypeChecker, node: NodeId) {
    match *node.get(c.nodes) {
        Node::Fn(_, ref params, return_ty, _) => {
            let param_tys = params.iter().map(|p| check_node(c, *p)).collect();
            let return_ty = return_ty
                .map(|t| check_node(c, t))
                .unwrap_or(c.common_types.unit);
            let fn_ty = c.types.alloc(Type::Fn(param_tys, return_ty));
            let fn_symbol = c.bind_ctx.bindings[node.idx()].unwrap();
            c.binding_types.insert(fn_symbol, fn_ty);
        }
        _ => {}
    }
}

fn check_node(c: &mut TypeChecker, node: NodeId) -> TypeId {
    match *node.get(c.nodes) {
        Node::Bool(_) => c.common_types.boolean,
        Node::Number(_) => c.common_types.number,
        Node::String(_) => c.common_types.string,
        Node::Assign(lhs, rhs) => {
            let lhs_ty = check_node(c, lhs);
            let rhs_ty = check_node(c, rhs);
            c.unify(lhs_ty, rhs_ty, rhs.span(c.nodes));
            c.common_types.unit
        }
        Node::Binary(lhs, op, rhs) => {
            let lhs_ty = check_node(c, lhs);
            let rhs_ty = check_node(c, rhs);
            c.unify(lhs_ty, rhs_ty, rhs.span(c.nodes));
            match op {
                BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::BitOr
                | BinaryOp::BitAnd => {
                    c.unify(c.common_types.number, lhs_ty, lhs.span(c.nodes));
                    c.common_types.number
                }
                BinaryOp::Greater
                | BinaryOp::GreaterEqual
                | BinaryOp::Less
                | BinaryOp::LessEqual => {
                    c.unify(c.common_types.number, lhs_ty, lhs.span(c.nodes));
                    c.common_types.boolean
                }
                BinaryOp::Equal | BinaryOp::NotEqual => c.common_types.boolean,
                BinaryOp::Or | BinaryOp::And => {
                    c.unify(c.common_types.boolean, lhs_ty, lhs.span(c.nodes));
                    c.common_types.boolean
                }
            }
        }
        Node::Not(expr) => {
            let ty = check_node(c, expr);
            c.unify(c.common_types.boolean, ty, expr.span(c.nodes));
            c.common_types.boolean
        }
        Node::Negate(expr) => {
            let ty = check_node(c, expr);
            c.unify(c.common_types.number, ty, expr.span(c.nodes));
            c.common_types.number
        }
        Node::Let(_, ty, init) => {
            let init_ty = check_node(c, init);
            if let Some(ty) = ty {
                let ty = check_node(c, ty);
                c.unify(ty, init_ty, init.span(c.nodes));
            }
            let symbol = c.bind_ctx.bindings[node.idx()].unwrap();
            c.binding_types.insert(symbol, init_ty);
            c.common_types.unit
        }
        Node::Variable(..) => {
            let symbol = c.bind_ctx.bindings[node.idx()].unwrap();
            c.binding_types[&symbol]
        }
        Node::If(cond, then_clause, else_clause) => {
            let cond_ty = check_node(c, cond);
            c.unify(c.common_types.boolean, cond_ty, cond.span(c.nodes));
            let then_clause_ty = check_node(c, then_clause);
            if let Some(else_clause) = else_clause {
                let else_clause_ty = check_node(c, else_clause);
                c.unify(then_clause_ty, else_clause_ty, else_clause.span(c.nodes));
                then_clause_ty
            } else {
                c.unify(
                    c.common_types.unit,
                    then_clause_ty,
                    then_clause.span(c.nodes),
                );
                c.common_types.unit
            }
        }
        Node::Block(ref nodes, expr) => {
            check_scope(c, nodes);
            if let Some(expr) = expr {
                check_node(c, expr)
            } else {
                c.common_types.unit
            }
        }
        Node::Semi(expr) => {
            check_node(c, expr);
            c.common_types.unit
        }
        Node::While(cond, body) => {
            let cond_ty = check_node(c, cond);
            c.unify(c.common_types.boolean, cond_ty, cond.span(c.nodes));
            check_node(c, body);
            c.common_types.unit
        }
        Node::Break => c.types.alloc(Type::Var),
        Node::Continue => c.types.alloc(Type::Var),
        Node::Fn(_, _, _, body) => {
            let fn_symbol = c.bind_ctx.bindings[node.idx()].unwrap();
            let fn_ty = c.binding_types[&fn_symbol];
            let Type::Fn(_, return_ty) = c.types[fn_ty] else {
                unreachable!();
            };

            let prev_containing_return_ty = c.containing_return_ty.replace(return_ty);
            let body_ty = check_node(c, body);
            c.unify(return_ty, body_ty, body.span(c.nodes));
            c.containing_return_ty = prev_containing_return_ty;
            c.common_types.unit
        }
        Node::Param(_, ty) => {
            let ty = check_node(c, ty);
            let symbol = c.bind_ctx.bindings[node.idx()].unwrap();
            c.binding_types.insert(symbol, ty);
            ty
        }
        Node::Call(callee, ref args) => {
            let callee_ty = check_node(c, callee);
            let Type::Fn(params, ret_ty) = c.types[callee_ty].clone() else {
                c.error("Expect callee to be a function", node.span(c.nodes));
                return c.types.alloc(Type::Var);
            };
            if params.len() == args.len() {
                for (arg, param) in args.iter().zip(params.iter()) {
                    let arg_ty = check_node(c, *arg);
                    c.unify(*param, arg_ty, arg.span(c.nodes));
                }
            } else {
                c.error(
                    format!(
                        "Type mismatch: Expect {} arguments, got {}",
                        params.len(),
                        args.len()
                    ),
                    node.span(c.nodes),
                );
            }
            ret_ty
        }
        Node::Return(value) => {
            let value_ty = if let Some(value) = value {
                check_node(c, value)
            } else {
                c.common_types.unit
            };
            c.unify(
                value_ty,
                c.containing_return_ty.unwrap(),
                node.span(c.nodes),
            );
            c.types.alloc(Type::Var)
        }
    }
}
