use std::rc::Rc;

use ena::unify::InPlaceUnificationTable;
use rustc_hash::FxHashMap;

use crate::{
    Error, FxIndexMap,
    ast::{AstNodes, BinaryOp, Expr, ExprId, Span, Stmt, StmtId, Type, TypeId},
    bind::{BindCtx, SymbolId},
    diagnostics::Diagnostics,
    interner::StrId,
    types::{CommonTypes, Ty, TyId},
};

pub struct TyContext {
    pub bind_ctx: BindCtx,
    pub types: InPlaceUnificationTable<TyId>,
    pub struct_properties: FxHashMap<SymbolId, FxIndexMap<StrId, TyId>>,
    pub expr_types: FxHashMap<ExprId, TyId>,
}

pub struct TypeChecker<'a> {
    ast: &'a AstNodes,
    diagnostics: &'a mut Diagnostics,
    bind_ctx: BindCtx,
    types: InPlaceUnificationTable<TyId>,
    common_types: CommonTypes,
    has_error: bool,
    containing_return_ty: Option<TyId>,
    type_bindings: FxHashMap<SymbolId, TyId>,
    value_bindings: FxHashMap<SymbolId, TyId>,
    struct_properties: FxHashMap<SymbolId, FxIndexMap<StrId, TyId>>,
    tuple_struct_properties: FxHashMap<SymbolId, Rc<[TyId]>>,
    expr_types: FxHashMap<ExprId, TyId>,
    type_params: FxHashMap<TyId, Vec<TyId>>,
    integer_literals: Vec<(ExprId, TyId)>,
    deferred_negation_checks: Vec<(ExprId, TyId)>,
}

impl<'a> TypeChecker<'a> {
    pub fn new(ast: &'a AstNodes, diagnostics: &'a mut Diagnostics, bind_ctx: BindCtx) -> Self {
        let mut types = InPlaceUnificationTable::new();
        let common_types = CommonTypes::new(&mut types);
        Self {
            ast,
            diagnostics,
            bind_ctx,
            types,
            common_types,
            has_error: false,
            containing_return_ty: None,
            type_bindings: FxHashMap::default(),
            value_bindings: FxHashMap::default(),
            struct_properties: FxHashMap::default(),
            tuple_struct_properties: FxHashMap::default(),
            expr_types: FxHashMap::default(),
            type_params: FxHashMap::default(),
            integer_literals: Vec::new(),
            deferred_negation_checks: Vec::new(),
        }
    }

    pub fn check(mut self, nodes: &[StmtId]) -> Result<TyContext, Error> {
        self.type_bindings
            .insert(self.bind_ctx.f64_type, self.common_types.f64);
        self.type_bindings
            .insert(self.bind_ctx.i64_type, self.common_types.i64);
        self.type_bindings
            .insert(self.bind_ctx.u64_type, self.common_types.u64);
        self.type_bindings
            .insert(self.bind_ctx.string_type, self.common_types.string);
        self.type_bindings
            .insert(self.bind_ctx.bool_type, self.common_types.boolean);

        let print_arg_ty = self
            .types
            .new_key(Ty::TypeParam(self.bind_ctx.print_function));
        let print_fn_ty = self
            .types
            .new_key(Ty::Fn([print_arg_ty].into(), self.common_types.unit));
        self.value_bindings
            .insert(self.bind_ctx.print_function, print_fn_ty);
        self.type_params.insert(print_fn_ty, vec![print_arg_ty]);

        check_scope(&mut self, nodes);
        deferred_check_integers(&mut self);
        if self.has_error {
            Err(Error::TypeCheck)
        } else {
            Ok(TyContext {
                bind_ctx: self.bind_ctx,
                types: self.types,
                struct_properties: self.struct_properties,
                expr_types: self.expr_types,
            })
        }
    }

    fn unify(&mut self, lhs: TyId, rhs: TyId, span: Span) {
        if self.can_unify(lhs, rhs) {
            self.unify_recursively(lhs, rhs);
        } else {
            let lhs = lhs.display(self);
            let rhs = rhs.display(self);
            self.error(format!("expect `{lhs}`, found `{rhs}`"), span);
        }
    }

    fn can_unify(&mut self, lhs: TyId, rhs: TyId) -> bool {
        if self.types.unioned(lhs, rhs) {
            return true;
        }
        match (self.types.probe_value(lhs), self.types.probe_value(rhs)) {
            (Ty::Tuple(a), Ty::Tuple(b)) if a.len() == b.len() => {
                let mut result = true;
                for (a, b) in a.iter().zip(b.iter()) {
                    if !self.can_unify(*a, *b) {
                        result = false;
                    }
                }
                result
            }
            (Ty::Fn(.., a_params, a_ret), Ty::Fn(.., b_params, b_ret)) => {
                if a_params.len() != b_params.len() {
                    return false;
                }
                let mut result = true;
                for (a, b) in a_params.iter().zip(b_params.iter()) {
                    if !self.can_unify(*a, *b) {
                        result = false;
                    }
                }
                if !self.can_unify(a_ret, b_ret) {
                    result = false;
                }
                result
            }
            (Ty::Array(a), Ty::Array(b)) => self.can_unify(a, b),
            (Ty::Unknown, _) | (_, Ty::Unknown) => true,
            (Ty::UnknownInt, Ty::I64 | Ty::U64)
            | (Ty::I64 | Ty::U64, Ty::UnknownInt)
            | (Ty::UnknownInt, Ty::UnknownInt) => true,
            _ => false,
        }
    }

    fn unify_recursively(&mut self, lhs: TyId, rhs: TyId) {
        if self.types.unioned(lhs, rhs) {
            return;
        }
        let lhs_ty = self.types.probe_value(lhs);
        let rhs_ty = self.types.probe_value(rhs);
        self.types.union(lhs, rhs);
        match (lhs_ty, rhs_ty) {
            (Ty::Tuple(a), Ty::Tuple(b)) if a.len() == b.len() => {
                for (a, b) in a.iter().zip(b.iter()) {
                    self.unify_recursively(*a, *b);
                }
            }
            (Ty::Fn(.., a_params, a_ret), Ty::Fn(.., b_params, b_ret)) => {
                for (a, b) in a_params.iter().zip(b_params.iter()) {
                    self.unify_recursively(*a, *b);
                }
                self.unify_recursively(a_ret, b_ret);
            }
            (Ty::Array(a), Ty::Array(b)) => self.unify_recursively(a, b),
            _ => {}
        }
    }

    fn error(&mut self, message: impl Into<String>, span: Span) {
        self.has_error = true;
        self.diagnostics.push(message, span);
    }
}

fn deferred_check_integers(c: &mut TypeChecker) {
    for (expr, ty) in std::mem::take(&mut c.integer_literals) {
        match c.types.probe_value(ty) {
            Ty::UnknownInt => {
                c.types.union_value(ty, Ty::I64);
                check_literal_range_i64(c, expr);
            }
            Ty::I64 => check_literal_range_i64(c, expr),
            Ty::U64 => check_literal_range_u64(c, expr),
            _ => unreachable!(),
        }
    }

    for (expr, ty) in std::mem::take(&mut c.deferred_negation_checks) {
        if let Ty::U64 = c.types.probe_value(ty) {
            c.error("expect `i64` or `f64`, found `u64`", c.ast.exprs.span(expr));
        }
    }
}

fn check_literal_range_i64(c: &mut TypeChecker, expr: ExprId) {
    let Expr::Int(n) = c.ast.exprs[expr] else {
        unreachable!();
    };
    if n < i64::MIN as i128 || n > i64::MAX as i128 {
        c.error("literal out of range", c.ast.exprs.span(expr));
    }
}

fn check_literal_range_u64(c: &mut TypeChecker, expr: ExprId) {
    let Expr::Int(n) = c.ast.exprs[expr] else {
        unreachable!();
    };
    if n < 0 || n > u64::MAX as i128 {
        c.error("literal out of range", c.ast.exprs.span(expr));
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
        Stmt::Fn(_, ref type_params, ref params, ret, _) => {
            let type_params: Vec<TyId> = type_params.iter().map(|p| check_stmt(c, *p)).collect();
            let params = params.iter().map(|p| check_stmt(c, *p)).collect();
            let ret = ret.map(|t| check_type(c, t)).unwrap_or(c.common_types.unit);
            let fn_ty = c.types.new_key(Ty::Fn(params, ret));
            let fn_symbol = c.ast.stmts.value_binding(node);
            c.value_bindings.insert(fn_symbol, fn_ty);
            if !type_params.is_empty() {
                c.type_params.insert(fn_ty, type_params);
            }
        }
        Stmt::Struct(..) => {
            let struct_symbol = c.ast.stmts.type_binding(node);
            let struct_ty = c.types.new_key(Ty::Named(struct_symbol));
            c.type_bindings.insert(struct_symbol, struct_ty);
        }
        Stmt::TupleStruct(..) => {
            let struct_symbol = c.ast.stmts.type_binding(node);
            let struct_ty = c.types.new_key(Ty::Named(struct_symbol));
            c.type_bindings.insert(struct_symbol, struct_ty);
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
            let symbol = c.ast.stmts.value_binding(node);
            c.value_bindings.insert(symbol, init_ty);
            c.common_types.unit
        }
        Stmt::Fn(.., body) => {
            let fn_symbol = c.ast.stmts.value_binding(node);
            let fn_ty = c.value_bindings[&fn_symbol];
            let Ty::Fn(.., return_ty) = c.types.probe_value(fn_ty) else {
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
            let symbol = c.ast.stmts.value_binding(node);
            c.value_bindings.insert(symbol, ty);
            ty
        }
        Stmt::Expr(expr, semi) => {
            let ty = check_expr(c, expr);
            if semi { c.common_types.unit } else { ty }
        }
        Stmt::Struct(.., ref fields) => {
            let symbol = c.ast.stmts.type_binding(node);
            let mut properties = FxIndexMap::default();
            for (name, ty) in fields.iter() {
                let name = c.bind_ctx.interner.intern(name);
                let ty = check_type(c, *ty);
                properties.insert(name, ty);
            }
            c.struct_properties.insert(symbol, properties);
            c.common_types.unit
        }
        Stmt::TupleStruct(.., ref types) => {
            let type_symbol = c.ast.stmts.type_binding(node);
            let mut arg_tys = Vec::new();
            for ty in types {
                let ty = check_type(c, *ty);
                arg_tys.push(ty);
            }
            let arg_tys: Rc<[TyId]> = arg_tys.into();
            c.tuple_struct_properties
                .insert(type_symbol, arg_tys.clone());

            let struct_ty = c.type_bindings[&type_symbol];
            let fn_ty = c.types.new_key(Ty::Fn(arg_tys, struct_ty));
            let value_symbol = c.ast.stmts.value_binding(node);
            c.value_bindings.insert(value_symbol, fn_ty);
            c.common_types.unit
        }
        Stmt::TypeParam(..) => {
            let symbol = c.ast.stmts.type_binding(node);
            let ty = c.types.new_key(Ty::TypeParam(symbol));
            c.type_bindings.insert(symbol, ty);
            ty
        }
    }
}

fn check_expr(c: &mut TypeChecker, node: ExprId) -> TyId {
    let ty = check_expr_inner(c, node);
    let prev = c.expr_types.insert(node, ty);
    debug_assert_eq!(prev, None);
    ty
}

fn check_expr_inner(c: &mut TypeChecker, node: ExprId) -> TyId {
    match c.ast.exprs[node] {
        Expr::Bool(_) => c.common_types.boolean,
        Expr::Int(_) => {
            let ty = c.types.new_key(Ty::UnknownInt);
            c.integer_literals.push((node, ty));
            ty
        }
        Expr::Number(_) => c.common_types.f64,
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
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                    c.unify(lhs_ty, rhs_ty, c.ast.exprs.span(rhs));
                    match c.types.probe_value(lhs_ty) {
                        Ty::I64 | Ty::U64 | Ty::F64 | Ty::UnknownInt => lhs_ty,
                        _ => {
                            let message = format!(
                                "expect `i64`, `u64` or `f64`, found `{}`",
                                lhs_ty.display(c)
                            );
                            c.error(message, c.ast.exprs.span(lhs));
                            c.types.new_key(Ty::Unknown)
                        }
                    }
                }
                BinaryOp::BitOr | BinaryOp::BitAnd => {
                    c.unify(lhs_ty, rhs_ty, c.ast.exprs.span(rhs));
                    match c.types.probe_value(lhs_ty) {
                        Ty::I64 | Ty::U64 | Ty::UnknownInt => lhs_ty,
                        _ => {
                            let message =
                                format!("expect `i64` or `u64`, found `{}`", lhs_ty.display(c));
                            c.error(message, c.ast.exprs.span(lhs));
                            c.types.new_key(Ty::Unknown)
                        }
                    }
                }
                BinaryOp::Greater
                | BinaryOp::GreaterEqual
                | BinaryOp::Less
                | BinaryOp::LessEqual => {
                    c.unify(lhs_ty, rhs_ty, c.ast.exprs.span(rhs));
                    match c.types.probe_value(lhs_ty) {
                        Ty::I64 | Ty::U64 | Ty::F64 | Ty::UnknownInt => {}
                        _ => {
                            let message = format!(
                                "expect `i64`, `u64` or `f64`, found `{}`",
                                lhs_ty.display(c)
                            );
                            c.error(message, c.ast.exprs.span(lhs));
                        }
                    }
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
            match c.types.probe_value(ty) {
                Ty::I64 | Ty::F64 => ty,
                Ty::UnknownInt => {
                    c.deferred_negation_checks.push((expr, ty));
                    ty
                }
                _ => {
                    let message = format!("expect `i64` or `f64`, found `{}`", ty.display(c));
                    c.error(message, c.ast.exprs.span(expr));
                    c.types.new_key(Ty::Unknown)
                }
            }
        }
        Expr::Variable(..) => {
            let symbol = c.ast.exprs.value_binding(node);
            instantiate(c, c.value_bindings[&symbol])
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
        Expr::Break => c.types.new_key(Ty::Unknown),
        Expr::Continue => c.types.new_key(Ty::Unknown),
        Expr::Call(callee, ref args) => {
            let callee_ty = check_expr(c, callee);
            let Ty::Fn(params, ret) = c.types.probe_value(callee_ty) else {
                c.error("call expression requires function", c.ast.exprs.span(node));
                return c.types.new_key(Ty::Unknown);
            };
            if args.len() != params.len() {
                c.error(
                    format!("expect {} arguments, found {}", params.len(), args.len()),
                    c.ast.exprs.span(node),
                );
                return c.types.new_key(Ty::Unknown);
            }
            for (&arg, param) in args.iter().zip(params.iter()) {
                let arg_ty = check_expr(c, arg);
                c.unify(*param, arg_ty, c.ast.exprs.span(arg));
            }
            ret
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
            c.types.new_key(Ty::Unknown)
        }
        Expr::Unit => c.common_types.unit,
        Expr::Tuple(ref types) => {
            let types = types.iter().map(|t| check_expr(c, *t)).collect();
            c.types.new_key(Ty::Tuple(types))
        }
        Expr::TupleField(expr, index) => {
            let expr_ty = check_expr(c, expr);
            let types = match c.types.probe_value(expr_ty) {
                Ty::Tuple(types) => types,
                Ty::Named(symbol) => {
                    if let Some(fields) = c.tuple_struct_properties.get(&symbol) {
                        fields.clone()
                    } else {
                        c.error("expect a tuple", c.ast.exprs.span(expr));
                        return c.types.new_key(Ty::Unknown);
                    }
                }
                _ => {
                    c.error("expect a tuple", c.ast.exprs.span(expr));
                    return c.types.new_key(Ty::Unknown);
                }
            };
            if index as usize >= types.len() {
                c.error("unknown field", c.ast.exprs.span(node));
                return c.types.new_key(Ty::Unknown);
            }
            types[index as usize]
        }
        Expr::Struct(_, ref fields) => {
            let symbol = c.ast.exprs.value_binding(node);
            let declared = c.struct_properties[&symbol].clone();
            for (name, &value) in fields {
                let name = c.bind_ctx.interner.intern(name);
                let ty = check_expr(c, value);
                if let Some(&declared_ty) = declared.get(&name) {
                    c.unify(declared_ty, ty, c.ast.exprs.span(value));
                } else {
                    c.error("unknown field", c.ast.exprs.span(value));
                }
            }

            let mut missing = Vec::new();
            for (name, _) in declared {
                let name = c.bind_ctx.interner.lookup(name);
                if !fields.contains_key(name) {
                    missing.push(name);
                }
            }
            if !missing.is_empty() {
                missing.sort();
                c.error(
                    format!("missing: {}", missing.join(", ")),
                    c.ast.exprs.span(node),
                );
            }

            c.type_bindings[&symbol]
        }
        Expr::StructField(expr, ref name) => {
            let expr_ty = check_expr(c, expr);
            let Ty::Named(symbol) = c.types.probe_value(expr_ty) else {
                c.error("expect a struct instance", c.ast.exprs.span(expr));
                return c.types.new_key(Ty::Unknown);
            };
            let Some(properties) = c.struct_properties.get(&symbol) else {
                c.error("expect a struct instance", c.ast.exprs.span(expr));
                return c.types.new_key(Ty::Unknown);
            };

            let name_id = c.bind_ctx.interner.intern(name);
            let Some(&ty) = properties.get(&name_id) else {
                c.error("unknown field", c.ast.exprs.span(expr));
                return c.types.new_key(Ty::Unknown);
            };
            ty
        }
        Expr::For(init, cond, incr, body) => {
            if let Some(init) = init {
                check_stmt(c, init);
            }
            if let Some(cond) = cond {
                let ty = check_expr(c, cond);
                c.unify(c.common_types.boolean, ty, c.ast.exprs.span(cond));
            }
            if let Some(incr) = incr {
                check_expr(c, incr);
            }
            check_expr(c, body);
            c.common_types.unit
        }
        Expr::Array(ref elements) => {
            let element_ty = c.types.new_key(Ty::Unknown);
            for &expr in elements {
                let ty = check_expr(c, expr);
                c.unify(element_ty, ty, c.ast.exprs.span(expr));
            }
            c.types.new_key(Ty::Array(element_ty))
        }
        Expr::ArrayRepeat(expr, count) => {
            let element_ty = check_expr(c, expr);
            let count_ty = check_expr(c, count);
            c.unify(c.common_types.u64, count_ty, c.ast.exprs.span(count));
            c.types.new_key(Ty::Array(element_ty))
        }
        Expr::ArrayIndex(expr, index) => {
            let array_ty = check_expr(c, expr);
            let index_ty = check_expr(c, index);
            c.unify(c.common_types.u64, index_ty, c.ast.exprs.span(index));

            match c.types.probe_value(array_ty) {
                Ty::Array(element_ty) => element_ty,
                _ => {
                    c.error("expect array", c.ast.exprs.span(expr));
                    c.types.new_key(Ty::Unknown)
                }
            }
        }
    }
}

fn check_type(c: &mut TypeChecker, node: TypeId) -> TyId {
    match c.ast.types[node] {
        Type::FnType(ref params, ret) => {
            let params = params.iter().map(|t| check_type(c, *t)).collect();
            let ret = ret.map(|t| check_type(c, t)).unwrap_or(c.common_types.unit);
            c.types.new_key(Ty::Fn(params, ret))
        }
        Type::Unit => c.common_types.unit,
        Type::Named(..) => {
            let symbol = c.ast.types.type_binding(node);
            c.type_bindings[&symbol]
        }
        Type::Tuple(ref items) => {
            let types = items.iter().map(|t| check_type(c, *t)).collect();
            c.types.new_key(Ty::Tuple(types))
        }
        Type::Array(element) => {
            let ty = check_type(c, element);
            c.types.new_key(Ty::Array(ty))
        }
    }
}

fn instantiate(c: &mut TypeChecker, ty: TyId) -> TyId {
    if let Some(type_params) = c.type_params.get(&ty) {
        let mut subst = FxHashMap::default();
        for t in type_params {
            subst.insert(*t, c.types.new_key(Ty::Unknown));
        }
        substitute(c, ty, &subst)
    } else {
        ty
    }
}

fn substitute(c: &mut TypeChecker, ty: TyId, subst: &FxHashMap<TyId, TyId>) -> TyId {
    match c.types.probe_value(ty) {
        Ty::Fn(params, ret) => {
            let params = params.iter().map(|p| substitute(c, *p, subst)).collect();
            let ret = substitute(c, ret, subst);
            c.types.new_key(Ty::Fn(params, ret))
        }
        Ty::Tuple(params) => {
            let params = params.iter().map(|p| substitute(c, *p, subst)).collect();
            c.types.new_key(Ty::Tuple(params))
        }
        Ty::Array(el) => {
            let el = substitute(c, el, subst);
            c.types.new_key(Ty::Array(el))
        }
        Ty::TypeParam(_) => subst.get(&ty).copied().unwrap_or(ty),
        _ => ty,
    }
}

impl TyId {
    pub fn display(self, c: &mut TypeChecker) -> String {
        let mut buf = String::new();
        print_type(self, c, &mut buf);
        buf
    }
}

fn print_type(ty: TyId, c: &mut TypeChecker, buf: &mut String) {
    match c.types.probe_value(ty) {
        Ty::Named(symbol) | Ty::TypeParam(symbol) => {
            buf.push_str(c.bind_ctx.interner.lookup(c.bind_ctx.symbols[symbol].name))
        }
        Ty::Fn(params, ret) => {
            buf.push_str("fn(");
            if let Some(type_params) = c.type_params.get(&ty).cloned() {
                buf.push_str("<");
                let mut sep = "";
                for p in type_params {
                    buf.push_str(sep);
                    print_type(p, c, buf);
                    sep = ", ";
                }
                buf.push_str(">");
            }
            let mut sep = "";
            for p in params.iter() {
                buf.push_str(sep);
                print_type(*p, c, buf);
                sep = ", ";
            }
            buf.push_str(") -> ");
            print_type(ret, c, buf);
        }
        Ty::Unit => buf.push_str("()"),
        Ty::Bool => buf.push_str("bool"),
        Ty::F64 => buf.push_str("f64"),
        Ty::I64 => buf.push_str("i64"),
        Ty::U64 => buf.push_str("u64"),
        Ty::UnknownInt => buf.push_str("{integer}"),
        Ty::String => buf.push_str("string"),
        Ty::Unknown => buf.push_str("<cannot infer>"),
        Ty::Tuple(types) => {
            buf.push_str("(");
            let mut sep = "";
            for t in types.iter() {
                buf.push_str(sep);
                print_type(*t, c, buf);
                sep = ", ";
            }
            buf.push_str(")")
        }
        Ty::Array(element) => {
            buf.push_str("[");
            print_type(element, c, buf);
            buf.push_str("]");
        }
    }
}
