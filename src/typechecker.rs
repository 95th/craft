use core::fmt;
use std::collections::HashMap;

use crate::{
    arena::Arena,
    ast::{BinaryOp, Node, NodeArena, NodeId, Span},
    bind::{BindCtx, SymbolId},
    interner::StrId,
    types::{CommonTypes, Type, TypeId},
};

pub struct TypeChecker<'a> {
    source: &'a str,
    nodes: &'a NodeArena,
    bind_ctx: BindCtx,
    types: Arena<Type>,
    common_types: CommonTypes,
    has_error: bool,
    containing_return_ty: Option<TypeId>,
    binding_types: HashMap<SymbolId, TypeId>,
    struct_properties: HashMap<SymbolId, HashMap<StrId, TypeId>>,
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
            containing_return_ty: None,
            binding_types: HashMap::new(),
            struct_properties: HashMap::new(),
        }
    }

    pub fn check(&mut self, nodes: &[NodeId]) {
        self.binding_types
            .insert(self.bind_ctx.number_type, self.common_types.number);
        self.binding_types
            .insert(self.bind_ctx.str_type, self.common_types.string);
        self.binding_types
            .insert(self.bind_ctx.bool_type, self.common_types.boolean);
        check_scope(self, nodes);
    }

    fn unify(&mut self, lhs: TypeId, rhs: TypeId, span: Span) {
        if !self.try_unify(lhs, rhs) {
            self.error(
                format!(
                    "Type mismatch: Expected '{}' but got '{}'",
                    lhs.display(self),
                    rhs.display(self)
                ),
                span,
            );
        }
    }

    fn try_unify(&mut self, lhs: TypeId, rhs: TypeId) -> bool {
        if lhs == rhs || self.types[lhs] == self.types[rhs] {
            return true;
        }
        match (&self.types[lhs], &self.types[rhs]) {
            (Type::Tuple(a), Type::Tuple(b)) if a.len() == b.len() => {
                let mut result = true;
                for (a, b) in a.clone().iter().zip(b.clone().iter()) {
                    if !self.try_unify(*a, *b) {
                        result = false;
                    }
                }
                result
            }
            (&Type::Fn(ref a_params, a_ret), &Type::Fn(ref b_params, b_ret)) => {
                if a_params.len() != b_params.len() {
                    return false;
                }
                let mut result = true;
                for (a, b) in a_params.clone().iter().zip(b_params.clone().iter()) {
                    if !self.try_unify(*a, *b) {
                        result = false;
                    }
                }
                if !self.try_unify(a_ret, b_ret) {
                    result = false;
                }
                result
            }
            (Type::Var, Type::Var) => false,
            (Type::Var, other) => {
                self.types[lhs] = other.clone();
                true
            }
            (other, Type::Var) => {
                self.types[rhs] = other.clone();
                true
            }
            _ => false,
        }
    }

    fn error(&mut self, message: impl Into<String>, span: Span) {
        self.has_error = true;
        println!("Error at '{}': {}", span.text(self.source), message.into());
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
        Node::Struct(_, ref fields) => {
            let mut properties = HashMap::new();
            for (name, ty) in fields {
                let name = c.bind_ctx.interner.intern(name);
                let ty = check_node(c, *ty);
                properties.insert(name, ty);
            }
            let struct_symbol = c.bind_ctx.bindings[node.idx()].unwrap();
            let struct_ty = c.types.alloc(Type::Named(struct_symbol));
            c.binding_types.insert(struct_symbol, struct_ty);
            c.struct_properties.insert(struct_symbol, properties);
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
            let arg_tys = args.iter().map(|a| check_node(c, *a)).collect();
            let ret_ty = c.types.alloc(Type::Var);
            let expect_ty = c.types.alloc(Type::Fn(arg_tys, ret_ty));
            c.unify(callee_ty, expect_ty, node.span(c.nodes));
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
        Node::FnType(ref params, ret) => {
            let params = params.iter().map(|t| check_node(c, *t)).collect();
            let ret = ret.map(|t| check_node(c, t)).unwrap_or(c.common_types.unit);
            c.types.alloc(Type::Fn(params, ret))
        }
        Node::Unit => c.common_types.unit,
        Node::Tuple(ref types) => {
            let types = types.iter().map(|t| check_node(c, *t)).collect();
            c.types.alloc(Type::Tuple(types))
        }
        Node::TupleField(expr, index) => {
            let expr_ty = check_node(c, expr);
            let Type::Tuple(ref types) = c.types[expr_ty] else {
                c.error("Expect expr to be a tuple", expr.span(c.nodes));
                return c.types.alloc(Type::Var);
            };
            if index as usize >= types.len() {
                c.error(format!("Field {index} not found"), node.span(c.nodes));
                return c.types.alloc(Type::Var);
            }
            types[index as usize]
        }
        Node::Struct(_, ref fields) => {
            let symbol = c.bind_ctx.bindings[node.idx()].unwrap();
            let declared = c.struct_properties[&symbol].clone();
            for (field_name, field_ty) in fields {
                let name = c.bind_ctx.interner.intern(field_name);
                let ty = check_node(c, *field_ty);
                if let Some(declared_ty) = declared.get(&name) {
                    c.unify(*declared_ty, ty, field_ty.span(c.nodes));
                } else {
                    c.error(
                        format!("Unknown field: {field_name}"),
                        field_ty.span(c.nodes),
                    );
                }
            }

            for (name, _) in declared {
                let name = c.bind_ctx.interner.lookup(name);
                if !fields.contains_key(name) {
                    c.error(format!("Missing field: {name}"), node.span(c.nodes));
                }
            }

            c.binding_types[&symbol]
        }
        Node::StructField(expr, ref name) => {
            let expr_ty = check_node(c, expr);
            let Type::Named(symbol) = c.types[expr_ty] else {
                c.error("Expect expr to be a struct instance", expr.span(c.nodes));
                return c.types.alloc(Type::Var);
            };
            let Some(properties) = c.struct_properties.get(&symbol) else {
                c.error("Expect expr to be a struct instance", expr.span(c.nodes));
                return c.types.alloc(Type::Var);
            };

            let name_id = c.bind_ctx.interner.intern(name);
            let Some(ty) = properties.get(&name_id) else {
                c.error(
                    format!("Properties does not exist: '{name}'"),
                    expr.span(c.nodes),
                );
                return c.types.alloc(Type::Var);
            };
            *ty
        }
    }
}

impl TypeId {
    pub fn display<'a>(self, c: &'a TypeChecker) -> impl fmt::Display {
        DisplayType(self, c)
    }
}

struct DisplayType<'a>(TypeId, &'a TypeChecker<'a>);

impl fmt::Display for DisplayType<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self(ty, c) = *self;
        match c.types[ty] {
            Type::Named(symbol) => write!(
                f,
                "{}",
                c.bind_ctx.interner.lookup(c.bind_ctx.symbols[symbol].name)
            ),
            Type::Fn(ref params, ret) => {
                write!(f, "fn(")?;
                let mut sep = "";
                for p in params {
                    write!(f, "{sep}{}", p.display(c))?;
                    sep = ", ";
                }
                write!(f, ") -> {}", ret.display(c))
            }
            Type::Unit => write!(f, "()"),
            Type::Bool => write!(f, "bool"),
            Type::Number => write!(f, "number"),
            Type::String => write!(f, "string"),
            Type::Var => write!(f, "<cannot infer>"),
            Type::Tuple(ref types) => {
                write!(f, "(")?;
                let mut sep = "";
                for t in types {
                    write!(f, "{sep}{}", t.display(c))?;
                    sep = ", ";
                }
                write!(f, ")")
            }
        }
    }
}
