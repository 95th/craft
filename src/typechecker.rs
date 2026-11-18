use core::fmt;
use std::collections::HashMap;

use crate::{
    Error,
    arena::Arena,
    ast::{AstNodes, BinaryOp, Expr, ExprId, Span, Stmt, StmtId, Type, TypeId},
    bind::{BindCtx, SymbolId},
    diagnostics::Diagnostic,
    interner::StrId,
    types::{CommonTypes, Ty, TyId},
};

pub struct TypeChecker<'a> {
    ast: &'a AstNodes,
    diagnostics: &'a mut Vec<Diagnostic>,
    bind_ctx: BindCtx,
    types: Arena<Ty>,
    common_types: CommonTypes,
    has_error: bool,
    containing_return_ty: Option<TyId>,
    binding_types: HashMap<SymbolId, TyId>,
    struct_properties: HashMap<SymbolId, HashMap<StrId, TyId>>,
}

impl<'a> TypeChecker<'a> {
    pub fn new(ast: &'a AstNodes, diagnostics: &'a mut Vec<Diagnostic>, bind_ctx: BindCtx) -> Self {
        let mut types = Arena::new();
        let common_types = CommonTypes::new(&mut types);
        Self {
            ast,
            diagnostics,
            bind_ctx,
            types,
            common_types,
            has_error: false,
            containing_return_ty: None,
            binding_types: HashMap::new(),
            struct_properties: HashMap::new(),
        }
    }

    pub fn check(&mut self, nodes: &[StmtId]) -> Result<(), Error> {
        self.binding_types
            .insert(self.bind_ctx.number_type, self.common_types.number);
        self.binding_types
            .insert(self.bind_ctx.str_type, self.common_types.string);
        self.binding_types
            .insert(self.bind_ctx.bool_type, self.common_types.boolean);
        check_scope(self, nodes);
        if self.has_error {
            Err(Error::TypeCheck)
        } else {
            Ok(())
        }
    }

    fn unify(&mut self, lhs: TyId, rhs: TyId, span: Span) {
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

    fn try_unify(&mut self, lhs: TyId, rhs: TyId) -> bool {
        if lhs == rhs || self.types[lhs] == self.types[rhs] {
            return true;
        }
        match (&self.types[lhs], &self.types[rhs]) {
            (Ty::Tuple(a), Ty::Tuple(b)) if a.len() == b.len() => {
                let mut result = true;
                for (a, b) in a.clone().iter().zip(b.clone().iter()) {
                    if !self.try_unify(*a, *b) {
                        result = false;
                    }
                }
                result
            }
            (&Ty::Fn(ref a_params, a_ret), &Ty::Fn(ref b_params, b_ret)) => {
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
            (Ty::Var, Ty::Var) => false,
            (Ty::Var, other) => {
                self.types[lhs] = other.clone();
                true
            }
            (other, Ty::Var) => {
                self.types[rhs] = other.clone();
                true
            }
            _ => false,
        }
    }

    fn error(&mut self, message: impl Into<String>, span: Span) {
        self.has_error = true;
        self.diagnostics.push(Diagnostic {
            span,
            message: message.into(),
        });
    }
}

fn check_scope(c: &mut TypeChecker, nodes: &[StmtId]) {
    for node in nodes {
        declare_item(c, *node);
    }

    for node in nodes {
        check_stmt(c, *node);
    }
}

fn declare_item(c: &mut TypeChecker, node: StmtId) {
    match c.ast.stmts[node] {
        Stmt::Fn(_, ref params, return_ty, _) => {
            let param_tys = params.iter().map(|p| check_stmt(c, *p)).collect();
            let return_ty = return_ty
                .map(|t| check_type(c, t))
                .unwrap_or(c.common_types.unit);
            let fn_ty = c.types.alloc(Ty::Fn(param_tys, return_ty));
            let fn_symbol = c.bind_ctx.stmt_bindings[node].unwrap();
            c.binding_types.insert(fn_symbol, fn_ty);
        }
        Stmt::Struct(_, ref fields) => {
            let mut properties = HashMap::new();
            for (name, ty) in fields {
                let name = c.bind_ctx.interner.intern(name);
                let ty = check_type(c, *ty);
                properties.insert(name, ty);
            }
            let struct_symbol = c.bind_ctx.stmt_bindings[node].unwrap();
            let struct_ty = c.types.alloc(Ty::Named(struct_symbol));
            c.binding_types.insert(struct_symbol, struct_ty);
            c.struct_properties.insert(struct_symbol, properties);
        }
        _ => {}
    }
}

fn check_stmt(c: &mut TypeChecker, node: StmtId) -> TyId {
    match c.ast.stmts[node] {
        Stmt::Let(_, ty, init) => {
            let init_ty = check_expr(c, init);
            if let Some(ty) = ty {
                let ty = check_type(c, ty);
                c.unify(ty, init_ty, c.ast.exprs.span(init));
            }
            let symbol = c.bind_ctx.stmt_bindings[node].unwrap();
            c.binding_types.insert(symbol, init_ty);
            c.common_types.unit
        }
        Stmt::Fn(_, _, _, body) => {
            let fn_symbol = c.bind_ctx.stmt_bindings[node].unwrap();
            let fn_ty = c.binding_types[&fn_symbol];
            let Ty::Fn(_, return_ty) = c.types[fn_ty] else {
                unreachable!();
            };

            let prev_containing_return_ty = c.containing_return_ty.replace(return_ty);
            let body_ty = check_expr(c, body);
            c.unify(return_ty, body_ty, c.ast.exprs.span(body));
            c.containing_return_ty = prev_containing_return_ty;
            c.common_types.unit
        }
        Stmt::Param(_, ty) => {
            let ty = check_type(c, ty);
            let symbol = c.bind_ctx.stmt_bindings[node].unwrap();
            c.binding_types.insert(symbol, ty);
            ty
        }
        Stmt::Expr(expr, semi) => {
            let ty = check_expr(c, expr);
            if semi { c.common_types.unit } else { ty }
        }
        Stmt::Struct(..) => c.common_types.unit,
    }
}

fn check_expr(c: &mut TypeChecker, node: ExprId) -> TyId {
    match c.ast.exprs[node] {
        Expr::Bool(_) => c.common_types.boolean,
        Expr::Number(_) => c.common_types.number,
        Expr::String(_) => c.common_types.string,
        Expr::Assign(lhs, rhs) => {
            let lhs_ty = check_expr(c, lhs);
            let rhs_ty = check_expr(c, rhs);
            c.unify(lhs_ty, rhs_ty, c.ast.exprs.span(rhs));
            c.common_types.unit
        }
        Expr::Binary(lhs, op, rhs) => {
            let lhs_ty = check_expr(c, lhs);
            let rhs_ty = check_expr(c, rhs);
            match op {
                BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::BitOr
                | BinaryOp::BitAnd => {
                    c.unify(c.common_types.number, lhs_ty, c.ast.exprs.span(lhs));
                    c.unify(c.common_types.number, rhs_ty, c.ast.exprs.span(rhs));
                    c.common_types.number
                }
                BinaryOp::Greater
                | BinaryOp::GreaterEqual
                | BinaryOp::Less
                | BinaryOp::LessEqual => {
                    c.unify(c.common_types.number, lhs_ty, c.ast.exprs.span(lhs));
                    c.unify(c.common_types.number, rhs_ty, c.ast.exprs.span(rhs));
                    c.common_types.boolean
                }
                BinaryOp::Equal | BinaryOp::NotEqual => {
                    c.unify(lhs_ty, rhs_ty, c.ast.exprs.span(rhs));
                    c.common_types.boolean
                }
                BinaryOp::Or | BinaryOp::And => {
                    c.unify(c.common_types.boolean, lhs_ty, c.ast.exprs.span(lhs));
                    c.unify(c.common_types.boolean, rhs_ty, c.ast.exprs.span(rhs));
                    c.common_types.boolean
                }
            }
        }
        Expr::Not(expr) => {
            let ty = check_expr(c, expr);
            c.unify(c.common_types.boolean, ty, c.ast.exprs.span(expr));
            c.common_types.boolean
        }
        Expr::Negate(expr) => {
            let ty = check_expr(c, expr);
            c.unify(c.common_types.number, ty, c.ast.exprs.span(expr));
            c.common_types.number
        }
        Expr::Variable(..) => {
            let symbol = c.bind_ctx.expr_bindings[node].unwrap();
            c.binding_types[&symbol]
        }
        Expr::If(cond, then_clause, else_clause) => {
            let cond_ty = check_expr(c, cond);
            c.unify(c.common_types.boolean, cond_ty, c.ast.exprs.span(cond));
            let then_clause_ty = check_expr(c, then_clause);
            if let Some(else_clause) = else_clause {
                let else_clause_ty = check_expr(c, else_clause);
                c.unify(
                    then_clause_ty,
                    else_clause_ty,
                    c.ast.exprs.span(else_clause),
                );
                then_clause_ty
            } else {
                c.unify(
                    c.common_types.unit,
                    then_clause_ty,
                    c.ast.exprs.span(then_clause),
                );
                c.common_types.unit
            }
        }
        Expr::Block(ref nodes, expr) => crate::stack_safe(|| {
            check_scope(c, nodes);
            if let Some(expr) = expr {
                check_expr(c, expr)
            } else {
                c.common_types.unit
            }
        }),
        Expr::While(cond, body) => {
            let cond_ty = check_expr(c, cond);
            c.unify(c.common_types.boolean, cond_ty, c.ast.exprs.span(cond));
            check_expr(c, body);
            c.common_types.unit
        }
        Expr::Break => c.types.alloc(Ty::Var),
        Expr::Continue => c.types.alloc(Ty::Var),
        Expr::Call(callee, ref args) => {
            let callee_ty = check_expr(c, callee);
            let arg_tys = args.iter().map(|a| check_expr(c, *a)).collect();
            let ret_ty = c.types.alloc(Ty::Var);
            let expect_ty = c.types.alloc(Ty::Fn(arg_tys, ret_ty));
            c.unify(callee_ty, expect_ty, c.ast.exprs.span(node));
            ret_ty
        }
        Expr::Return(value) => {
            let value_ty = if let Some(value) = value {
                check_expr(c, value)
            } else {
                c.common_types.unit
            };
            c.unify(
                value_ty,
                c.containing_return_ty.unwrap(),
                c.ast.exprs.span(node),
            );
            c.types.alloc(Ty::Var)
        }
        Expr::Unit => c.common_types.unit,
        Expr::Tuple(ref types) => {
            let types = types.iter().map(|t| check_expr(c, *t)).collect();
            c.types.alloc(Ty::Tuple(types))
        }
        Expr::TupleField(expr, index) => {
            let expr_ty = check_expr(c, expr);
            let Ty::Tuple(ref types) = c.types[expr_ty] else {
                c.error("Expect expr to be a tuple", c.ast.exprs.span(expr));
                return c.types.alloc(Ty::Var);
            };
            if index as usize >= types.len() {
                c.error(format!("Field {index} not found"), c.ast.exprs.span(node));
                return c.types.alloc(Ty::Var);
            }
            types[index as usize]
        }
        Expr::Struct(_, ref fields) => {
            let symbol = c.bind_ctx.expr_bindings[node].unwrap();
            let declared = c.struct_properties[&symbol].clone();
            for (field_name, &value) in fields {
                let name = c.bind_ctx.interner.intern(field_name);
                let ty = check_expr(c, value);
                if let Some(declared_ty) = declared.get(&name) {
                    c.unify(*declared_ty, ty, c.ast.exprs.span(value));
                } else {
                    c.error(
                        format!("Unknown field: {field_name}"),
                        c.ast.exprs.span(value),
                    );
                }
            }

            for (name, _) in declared {
                let name = c.bind_ctx.interner.lookup(name);
                if !fields.contains_key(name) {
                    c.error(format!("Missing field: {name}"), c.ast.exprs.span(node));
                }
            }

            c.binding_types[&symbol]
        }
        Expr::StructField(expr, ref name) => {
            let expr_ty = check_expr(c, expr);
            let Ty::Named(symbol) = c.types[expr_ty] else {
                c.error(
                    "Expect expr to be a struct instance",
                    c.ast.exprs.span(expr),
                );
                return c.types.alloc(Ty::Var);
            };
            let Some(properties) = c.struct_properties.get(&symbol) else {
                c.error(
                    "Expect expr to be a struct instance",
                    c.ast.exprs.span(expr),
                );
                return c.types.alloc(Ty::Var);
            };

            let name_id = c.bind_ctx.interner.intern(name);
            let Some(ty) = properties.get(&name_id) else {
                c.error(
                    format!("Properties does not exist: '{name}'"),
                    c.ast.exprs.span(expr),
                );
                return c.types.alloc(Ty::Var);
            };
            *ty
        }
        Expr::For(init, cond, incr, body) => {
            if let Some(init) = init {
                check_stmt(c, init);
            }
            if let Some(cond) = cond {
                check_expr(c, cond);
            }
            if let Some(incr) = incr {
                check_expr(c, incr);
            }
            check_expr(c, body);
            c.common_types.unit
        }
    }
}

fn check_type(c: &mut TypeChecker, node: TypeId) -> TyId {
    match c.ast.types[node] {
        Type::FnType(ref params, ret) => {
            let params = params.iter().map(|t| check_type(c, *t)).collect();
            let ret = ret.map(|t| check_type(c, t)).unwrap_or(c.common_types.unit);
            c.types.alloc(Ty::Fn(params, ret))
        }
        Type::Unit => c.common_types.unit,
        Type::Named(..) => {
            let symbol = c.bind_ctx.type_bindings[node].unwrap();
            c.binding_types[&symbol]
        }
        Type::Tuple(ref items) => {
            let types = items.iter().map(|t| check_type(c, *t)).collect();
            c.types.alloc(Ty::Tuple(types))
        }
    }
}

impl TyId {
    pub fn display<'a>(self, c: &'a TypeChecker) -> impl fmt::Display {
        DisplayType(self, c)
    }
}

struct DisplayType<'a>(TyId, &'a TypeChecker<'a>);

impl fmt::Display for DisplayType<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self(ty, c) = *self;
        match c.types[ty] {
            Ty::Named(symbol) => write!(
                f,
                "{}",
                c.bind_ctx.interner.lookup(c.bind_ctx.symbols[symbol].name)
            ),
            Ty::Fn(ref params, ret) => {
                write!(f, "fn(")?;
                let mut sep = "";
                for p in params {
                    write!(f, "{sep}{}", p.display(c))?;
                    sep = ", ";
                }
                write!(f, ") -> {}", ret.display(c))
            }
            Ty::Unit => write!(f, "()"),
            Ty::Bool => write!(f, "bool"),
            Ty::Number => write!(f, "number"),
            Ty::String => write!(f, "string"),
            Ty::Var => write!(f, "<cannot infer>"),
            Ty::Tuple(ref types) => {
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
