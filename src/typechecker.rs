use std::collections::HashMap;

use crate::{
    arena::Arena,
    ast::{BinaryOp, Node, NodeArena, NodeId},
    bind::{BindCtx, SymbolId},
    types::{CommonTypes, Type, TypeId},
};

pub struct TypeChecker<'a> {
    nodes: &'a NodeArena,
    bind_ctx: BindCtx,
    types: Arena<Type>,
    common_types: CommonTypes,
    has_error: bool,
    pending_unifications: Vec<(TypeId, TypeId)>,
    containing_return_ty: Option<TypeId>,
    binding_types: HashMap<SymbolId, TypeId>,
}

impl<'a> TypeChecker<'a> {
    pub fn new(nodes: &'a NodeArena, bind_ctx: BindCtx) -> Self {
        let mut types = Arena::new();
        let common_types = CommonTypes::new(&mut types);
        Self {
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

    fn unify(&mut self, lhs: TypeId, rhs: TypeId) {
        if lhs == rhs || self.types[lhs] == self.types[rhs] {
            return;
        }
        match (&self.types[lhs], &self.types[rhs]) {
            (Type::Named(a), Type::Named(b)) => {
                let a_name = self.bind_ctx.symbols[*a].name;
                let b_name = self.bind_ctx.symbols[*b].name;
                if a_name == b_name {
                    println!(
                        "Type mismatch. Both types have the same name but they're not the same."
                    );
                } else {
                    println!(
                        "Type mismatch: Expected '{}' but got '{}'",
                        self.bind_ctx.interner.lookup(a_name),
                        self.bind_ctx.interner.lookup(b_name),
                    );
                }
                self.has_error = true;
            }
            (&Type::Fn(ref params_a, ret_a), &Type::Fn(ref params_b, ret_b)) => {
                if params_a.len() != params_b.len() {
                    println!(
                        "Type mismatch, expected {} params, got: {}",
                        params_a.len(),
                        params_b.len()
                    );
                } else {
                    for (a, b) in params_a.clone().into_iter().zip(params_b.clone()) {
                        self.unify(a, b);
                    }
                }
                self.unify(ret_a, ret_b);
            }
            (Type::Var, Type::Var) => {
                self.pending_unifications.push((lhs, rhs));
            }
            (Type::Var, other) => self.types[lhs] = other.clone(),
            (other, Type::Var) => self.types[rhs] = other.clone(),
            (a, b) => {
                println!("Type mismatch: Expected '{a:?}' but got '{b:?}'");
                self.has_error = true;
            }
        }
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
        Node::Fn(..) | Node::Let(..) => {
            let symbol = c.bind_ctx.bindings[node.idx()].unwrap();
            c.binding_types.insert(symbol, c.types.alloc(Type::Var));
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
            let lhs = check_node(c, lhs);
            let rhs = check_node(c, rhs);
            c.unify(lhs, rhs);
            c.common_types.unit
        }
        Node::Binary(lhs, op, rhs) => {
            let lhs = check_node(c, lhs);
            let rhs = check_node(c, rhs);
            c.unify(lhs, rhs);
            match op {
                BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::BitOr
                | BinaryOp::BitAnd => {
                    c.unify(lhs, c.common_types.number);
                    c.common_types.number
                }
                BinaryOp::Greater
                | BinaryOp::GreaterEqual
                | BinaryOp::Less
                | BinaryOp::LessEqual => {
                    c.unify(lhs, c.common_types.number);
                    c.common_types.boolean
                }
                BinaryOp::Equal | BinaryOp::NotEqual => c.common_types.boolean,
                BinaryOp::Or | BinaryOp::And => {
                    c.unify(lhs, c.common_types.boolean);
                    c.common_types.boolean
                }
            }
        }
        Node::Not(expr) => {
            let ty = check_node(c, expr);
            c.unify(ty, c.common_types.boolean);
            c.common_types.boolean
        }
        Node::Negate(expr) => {
            let ty = check_node(c, expr);
            c.unify(ty, c.common_types.number);
            c.common_types.number
        }
        Node::Let(_, ty, init) => {
            let init = check_node(c, init);
            if let Some(ty) = ty {
                let ty = check_node(c, ty);
                c.unify(ty, init);
            }
            let symbol = c.bind_ctx.bindings[node.idx()].unwrap();
            c.unify(init, c.binding_types[&symbol]);
            c.common_types.unit
        }
        Node::Variable(..) => {
            let symbol = c.bind_ctx.bindings[node.idx()].unwrap();
            c.binding_types[&symbol]
        }
        Node::If(cond, then_clause, else_clause) => {
            let cond = check_node(c, cond);
            c.unify(cond, c.common_types.boolean);
            let then_clause = check_node(c, then_clause);
            if let Some(else_clause) = else_clause {
                let else_clause = check_node(c, else_clause);
                c.unify(then_clause, else_clause);
                then_clause
            } else {
                c.unify(then_clause, c.common_types.unit);
                c.common_types.unit
            }
        }
        Node::Block(ref nodes) => {
            check_scope(c, nodes);
            c.common_types.unit
        }
        Node::Semi(expr) => {
            check_node(c, expr);
            c.common_types.unit
        }
        Node::While(cond, body) => {
            let cond = check_node(c, cond);
            c.unify(cond, c.common_types.boolean);
            check_node(c, body);
            c.common_types.unit
        }
        Node::Break => c.types.alloc(Type::Var),
        Node::Continue => c.types.alloc(Type::Var),
        Node::Fn(_, ref params, return_ty, body) => {
            let params = params.iter().map(|p| check_node(c, *p)).collect();
            let return_ty = return_ty
                .map(|t| check_node(c, t))
                .unwrap_or(c.common_types.unit);
            let fn_ty = c.types.alloc(Type::Fn(params, return_ty));
            let fn_symbol = c.bind_ctx.bindings[node.idx()].unwrap();
            c.unify(c.binding_types[&fn_symbol], fn_ty);

            let prev_containing_return_ty = c.containing_return_ty.replace(return_ty);
            check_node(c, body);
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
            let callee = check_node(c, callee);
            let args = args.iter().map(|arg| check_node(c, *arg)).collect();
            let ret = c.types.alloc(Type::Var);
            let expected_callee = c.types.alloc(Type::Fn(args, ret));
            c.unify(callee, expected_callee);
            ret
        }
        Node::Return(value) => {
            let value = if let Some(value) = value {
                check_node(c, value)
            } else {
                c.common_types.unit
            };
            c.unify(value, c.containing_return_ty.unwrap());
            c.types.alloc(Type::Var)
        }
    }
}
