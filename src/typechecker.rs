use std::rc::Rc;

use ena::unify::InPlaceUnificationTable;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    Error, FxIndexMap,
    ast::{AstNodes, BinaryOp, Expr, ExprId, Span, Stmt, StmtId, Type, TypeId},
    bind::{BindCtxt, SymbolId},
    diagnostics::Diagnostics,
    interner::{StrId, StrInterner},
    types::{FnTy, StdTypes, Ty, TyId, TyTable},
};

pub struct TyCtxt {
    pub bcx: BindCtxt,
    pub types: TyTable,
    pub struct_properties: FxHashMap<SymbolId, FxIndexMap<StrId, TyId>>,
    pub tuple_struct_properties: FxHashMap<SymbolId, Rc<[TyId]>>,
    pub expr_types: FxHashMap<ExprId, TyId>,
    pub symbol_types: FxHashMap<SymbolId, TyId>,
}

pub struct TypeChecker<'a> {
    ast: &'a AstNodes,
    interner: &'a StrInterner,
    diagnostics: &'a mut Diagnostics,
    bcx: BindCtxt,
    types: TyTable,
    std: StdTypes,
    has_error: bool,
    containing_return_ty: Option<TyId>,
    symbol_types: FxHashMap<SymbolId, TyId>,
    struct_properties: FxHashMap<SymbolId, FxIndexMap<StrId, TyId>>,
    tuple_struct_properties: FxHashMap<SymbolId, Rc<[TyId]>>,
    expr_types: FxHashMap<ExprId, TyId>,
    integer_literals: Vec<(ExprId, TyId)>,
    deferred_negation_checks: Vec<(ExprId, TyId)>,
    diverges: bool,
}

impl<'a> TypeChecker<'a> {
    pub fn new(
        ast: &'a AstNodes,
        interner: &'a StrInterner,
        diagnostics: &'a mut Diagnostics,
        bcx: BindCtxt,
    ) -> Self {
        let mut types = InPlaceUnificationTable::new();
        let std = StdTypes::new(&mut types);
        Self {
            ast,
            interner,
            diagnostics,
            bcx,
            types,
            std,
            has_error: false,
            containing_return_ty: None,
            symbol_types: FxHashMap::default(),
            struct_properties: FxHashMap::default(),
            tuple_struct_properties: FxHashMap::default(),
            expr_types: FxHashMap::default(),
            integer_literals: Vec::new(),
            deferred_negation_checks: Vec::new(),
            diverges: false,
        }
    }

    pub fn check(mut self, nodes: &[StmtId]) -> Result<TyCtxt, Error> {
        self.register_default_types();
        self.register_default_functions();

        check_scope(&mut self, nodes, None);
        deferred_check_integers(&mut self);
        check_unbound_types(&mut self);
        if self.has_error {
            return Err(Error::TypeCheck);
        }
        Ok(TyCtxt {
            bcx: self.bcx,
            types: self.types,
            struct_properties: self.struct_properties,
            tuple_struct_properties: self.tuple_struct_properties,
            expr_types: self.expr_types,
            symbol_types: self.symbol_types,
        })
    }

    fn register_default_functions(&mut self) {
        {
            let arg = self.types.new_key(Ty::TypeParam(self.bcx.std.print));
            let params = Rc::<[TyId]>::from([arg]);
            let fn_ty = self.types.new_key(Ty::Fn(FnTy {
                type_params: params.clone(),
                params,
                ret: self.std.unit,
            }));
            self.symbol_types.insert(self.bcx.std.print, fn_ty);
        }

        {
            let fn_ty = self.types.new_key(Ty::Fn(FnTy {
                type_params: [].into(),
                params: [self.std.boolean].into(),
                ret: self.std.unit,
            }));
            self.symbol_types.insert(self.bcx.std.assert, fn_ty);
        }
    }

    fn register_default_types(&mut self) {
        self.symbol_types.insert(self.bcx.std.f64, self.std.f64);
        self.symbol_types.insert(self.bcx.std.i64, self.std.i64);
        self.symbol_types.insert(self.bcx.std.u64, self.std.u64);
        self.symbol_types
            .insert(self.bcx.std.string, self.std.string);
        self.symbol_types
            .insert(self.bcx.std.bool, self.std.boolean);
    }

    fn unify(&mut self, expected: TyId, actual: TyId, span: Span) {
        if self.can_unify(expected, actual) {
            expected.unify(actual, &mut self.types);
        } else {
            let expected = expected.display(self);
            let actual = actual.display(self);
            self.error(format!("expect `{expected}`, found `{actual}`"), span);
        }
    }

    fn can_unify(&mut self, expected: TyId, actual: TyId) -> bool {
        if self.types.unioned(expected, actual) {
            return true;
        }
        match (
            self.types.probe_value(expected),
            self.types.probe_value(actual),
        ) {
            (Ty::Tuple(a), Ty::Tuple(b)) => self.can_unify_list(&a, &b),
            (Ty::Fn(a), Ty::Fn(b)) => {
                self.can_unify_list(&a.params, &b.params) && self.can_unify(a.ret, b.ret)
            }
            (Ty::Array(a), Ty::Array(b)) => self.can_unify(a, b),
            (Ty::Unknown, _) | (_, Ty::Unknown) => true,
            (Ty::UnknownInt, Ty::UnknownInt) => true,
            (Ty::UnknownInt, Ty::I64 | Ty::U64) | (Ty::I64 | Ty::U64, Ty::UnknownInt) => true,
            (_, Ty::Never) => true,
            _ => false,
        }
    }

    fn can_unify_list(&mut self, expected: &[TyId], actual: &[TyId]) -> bool {
        expected.len() == actual.len()
            && expected
                .iter()
                .zip(actual.iter())
                .all(|(a, b)| self.can_unify(*a, *b))
    }

    fn error(&mut self, message: impl Into<String>, span: Span) {
        self.has_error = true;
        self.diagnostics.push(message, span);
    }
}

fn check_unbound_types(c: &mut TypeChecker) {
    let mut seen = FxHashSet::default();
    for (expr, ty) in c.expr_types.iter() {
        let ty = c.types.find(*ty);
        if !seen.insert(ty) {
            continue;
        }
        if has_unknown_type(&mut c.types, ty) {
            let span = c.ast.exprs.span(*expr);
            c.diagnostics.push("Cannot infer type", span);
            c.has_error = true;
        }
    }
}

fn has_unknown_type(types: &mut TyTable, ty: TyId) -> bool {
    match types.probe_value(ty) {
        Ty::Unknown | Ty::UnknownInt => true,
        Ty::Unit
        | Ty::Bool
        | Ty::F64
        | Ty::I64
        | Ty::U64
        | Ty::String
        | Ty::Never
        | Ty::Named(..)
        | Ty::TypeParam(..) => false,
        Ty::Fn(f) => {
            has_unknown_type(types, f.ret)
                || f.params.iter().any(|t| has_unknown_type(types, *t))
                || f.type_params.iter().any(|t| has_unknown_type(types, *t))
        }
        Ty::Tuple(params) => params.iter().any(|t| has_unknown_type(types, *t)),
        Ty::Array(element) => has_unknown_type(types, element),
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

fn check_scope(c: &mut TypeChecker, nodes: &[StmtId], trailing_expr: Option<ExprId>) -> TyId {
    let prev_diverges = std::mem::take(&mut c.diverges);
    for node in nodes {
        declare_item(c, *node);
    }
    for node in nodes {
        check_stmt(c, *node);
    }
    let ty = trailing_expr.map(|expr| check_expr(c, expr));
    let diverges = std::mem::replace(&mut c.diverges, prev_diverges);
    if let Some(ty) = ty {
        ty
    } else if diverges {
        c.types.new_key(Ty::Unknown)
    } else {
        c.std.unit
    }
}

fn declare_item(c: &mut TypeChecker, node: StmtId) {
    match c.ast.stmts[node] {
        Stmt::Fn(ref f) => {
            let type_params = f.type_params.iter().map(|p| check_stmt(c, *p)).collect();
            let params = f.params.iter().map(|p| check_stmt(c, *p)).collect();
            let ret = f.return_ty.map(|t| check_type(c, t)).unwrap_or(c.std.unit);
            let fn_ty = c.types.new_key(Ty::Fn(FnTy {
                type_params,
                params,
                ret,
            }));
            let fn_symbol = c.ast.stmts.value_binding(node);
            c.symbol_types.insert(fn_symbol, fn_ty);
        }
        Stmt::Struct(..) => {
            let struct_symbol = c.ast.stmts.type_binding(node);
            let struct_ty = c.types.new_key(Ty::Named(struct_symbol));
            c.symbol_types.insert(struct_symbol, struct_ty);
        }
        Stmt::TupleStruct { .. } => {
            let struct_symbol = c.ast.stmts.type_binding(node);
            let struct_ty = c.types.new_key(Ty::Named(struct_symbol));
            c.symbol_types.insert(struct_symbol, struct_ty);
        }
        _ => {}
    }
}

fn check_stmt(c: &mut TypeChecker, node: StmtId) -> TyId {
    match c.ast.stmts[node] {
        Stmt::Let { ty, init, .. } => {
            let init_ty = check_expr(c, init);
            if let Some(ty) = ty {
                let ty = check_type(c, ty);
                c.unify(ty, init_ty, c.ast.exprs.span(init));
            }
            let symbol = c.ast.stmts.value_binding(node);
            c.symbol_types.insert(symbol, init_ty);
            c.std.unit
        }
        Stmt::Fn(ref f) => {
            let fn_symbol = c.ast.stmts.value_binding(node);
            let fn_ty = c.symbol_types[&fn_symbol];
            let Ty::Fn(fn_ty) = c.types.probe_value(fn_ty) else {
                unreachable!();
            };

            let prev_containing_return_ty = c.containing_return_ty.replace(fn_ty.ret);
            let body_ty = check_expr(c, f.body);
            c.unify(fn_ty.ret, body_ty, c.ast.exprs.span(f.body));
            c.containing_return_ty = prev_containing_return_ty;
            c.std.unit
        }
        Stmt::Param { ty, .. } => {
            let ty = check_type(c, ty);
            let symbol = c.ast.stmts.value_binding(node);
            c.symbol_types.insert(symbol, ty);
            ty
        }
        Stmt::Expr { expr, semi } => {
            let ty = check_expr(c, expr);
            if semi { c.std.unit } else { ty }
        }
        Stmt::Struct(ref s) => {
            let symbol = c.ast.stmts.type_binding(node);
            let mut properties = FxIndexMap::default();
            for (&name, &ty) in s.fields.iter() {
                let ty = check_type(c, ty);
                properties.insert(name, ty);
            }
            c.struct_properties.insert(symbol, properties);
            c.std.unit
        }
        Stmt::TupleStruct(ref s) => {
            let type_symbol = c.ast.stmts.type_binding(node);
            let mut arg_tys = Vec::new();
            for ty in s.fields.iter() {
                let ty = check_type(c, *ty);
                arg_tys.push(ty);
            }
            let arg_tys: Rc<[TyId]> = arg_tys.into();
            c.tuple_struct_properties
                .insert(type_symbol, arg_tys.clone());

            let struct_ty = c.symbol_types[&type_symbol];
            let fn_ty = c.types.new_key(Ty::Fn(FnTy {
                type_params: [].into(),
                params: arg_tys,
                ret: struct_ty,
            }));
            let value_symbol = c.ast.stmts.value_binding(node);
            c.symbol_types.insert(value_symbol, fn_ty);
            c.std.unit
        }
        Stmt::TypeParam(..) => {
            let symbol = c.ast.stmts.type_binding(node);
            let ty = c.types.new_key(Ty::TypeParam(symbol));
            c.symbol_types.insert(symbol, ty);
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
        Expr::Bool(_) => c.std.boolean,
        Expr::Int(_) => {
            let ty = c.types.new_key(Ty::UnknownInt);
            c.integer_literals.push((node, ty));
            ty
        }
        Expr::Number(_) => c.std.f64,
        Expr::String(_) => c.std.string,
        Expr::Assign { lhs, rhs } => {
            let lhs_ty = check_expr(c, lhs);
            let rhs_ty = check_expr(c, rhs);
            c.unify(lhs_ty, rhs_ty, c.ast.exprs.span(rhs));
            c.std.unit
        }
        Expr::Binary { lhs, op, rhs } => match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                let lhs_ty = check_expr(c, lhs);
                let rhs_ty = check_expr(c, rhs);
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
                let lhs_ty = check_expr(c, lhs);
                let rhs_ty = check_expr(c, rhs);
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
            BinaryOp::Greater | BinaryOp::GreaterEqual | BinaryOp::Less | BinaryOp::LessEqual => {
                let lhs_ty = check_expr(c, lhs);
                let rhs_ty = check_expr(c, rhs);
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
                c.std.boolean
            }
            BinaryOp::Equal | BinaryOp::NotEqual => {
                let lhs_ty = check_expr(c, lhs);
                let rhs_ty = check_expr(c, rhs);
                c.unify(lhs_ty, rhs_ty, c.ast.exprs.span(rhs));
                c.std.boolean
            }
            BinaryOp::Or | BinaryOp::And => {
                let lhs_ty = check_expr(c, lhs);
                c.unify(c.std.boolean, lhs_ty, c.ast.exprs.span(lhs));
                let prev_diverges = std::mem::take(&mut c.diverges);
                let rhs_ty = check_expr(c, rhs);
                c.unify(c.std.boolean, rhs_ty, c.ast.exprs.span(rhs));
                c.diverges = prev_diverges;
                c.std.boolean
            }
        },
        Expr::Not(expr) => {
            let ty = check_expr(c, expr);
            c.unify(c.std.boolean, ty, c.ast.exprs.span(expr));
            c.std.boolean
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
            c.symbol_types[&symbol].instantiate(&mut c.types)
        }
        Expr::If {
            cond,
            then_clause,
            else_clause,
        } => {
            let cond_ty = check_expr(c, cond);
            c.unify(c.std.boolean, cond_ty, c.ast.exprs.span(cond));
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
                c.unify(c.std.unit, then_clause_ty, c.ast.exprs.span(then_clause));
                then_clause_ty
            }
        }
        Expr::Block { ref stmts, expr } => crate::stack_safe(|| check_scope(c, stmts, expr)),
        Expr::While { cond, body } => {
            let cond_ty = check_expr(c, cond);
            c.unify(c.std.boolean, cond_ty, c.ast.exprs.span(cond));
            let body_ty = check_expr(c, body);
            c.unify(c.std.unit, body_ty, c.ast.exprs.span(body));
            c.std.unit
        }
        Expr::Break => {
            c.diverges = true;
            c.types.new_key(Ty::Never)
        }
        Expr::Continue => {
            c.diverges = true;
            c.types.new_key(Ty::Never)
        }
        Expr::Call { callee, ref args } => {
            let callee_ty = check_expr(c, callee);
            let Ty::Fn(f) = c.types.probe_value(callee_ty) else {
                c.error("call expression requires function", c.ast.exprs.span(node));
                return c.types.new_key(Ty::Unknown);
            };
            if args.len() != f.params.len() {
                c.error(
                    format!("expect {} arguments, found {}", f.params.len(), args.len()),
                    c.ast.exprs.span(node),
                );
                return c.types.new_key(Ty::Unknown);
            }
            for (&arg, param) in args.iter().zip(f.params.iter()) {
                let arg_ty = check_expr(c, arg);
                c.unify(*param, arg_ty, c.ast.exprs.span(arg));
            }
            f.ret
        }
        Expr::Return(value) => {
            let value_ty = if let Some(value) = value {
                check_expr(c, value)
            } else {
                c.std.unit
            };
            c.unify(
                value_ty,
                c.containing_return_ty.unwrap(),
                c.ast.exprs.span(node),
            );
            c.diverges = true;
            c.types.new_key(Ty::Never)
        }
        Expr::Unit => c.std.unit,
        Expr::Tuple(ref types) => {
            let types = types.iter().map(|t| check_expr(c, *t)).collect();
            c.types.new_key(Ty::Tuple(types))
        }
        Expr::TupleField { obj, index } => {
            let expr_ty = check_expr(c, obj);
            let types = match c.types.probe_value(expr_ty) {
                Ty::Tuple(types) => types,
                Ty::Named(symbol) => {
                    if let Some(fields) = c.tuple_struct_properties.get(&symbol) {
                        fields.clone()
                    } else {
                        c.error("expect a tuple", c.ast.exprs.span(obj));
                        return c.types.new_key(Ty::Unknown);
                    }
                }
                _ => {
                    c.error("expect a tuple", c.ast.exprs.span(obj));
                    return c.types.new_key(Ty::Unknown);
                }
            };
            if index as usize >= types.len() {
                c.error("unknown field", c.ast.exprs.span(node));
                return c.types.new_key(Ty::Unknown);
            }
            types[index as usize]
        }
        Expr::Struct(ref s) => {
            let symbol = c.ast.exprs.value_binding(node);
            let declared = c.struct_properties[&symbol].clone();
            for (name, &value) in s.fields.iter() {
                let ty = check_expr(c, value);
                if let Some(&declared_ty) = declared.get(name) {
                    c.unify(declared_ty, ty, c.ast.exprs.span(value));
                } else {
                    c.error("unknown field", c.ast.exprs.span(value));
                }
            }

            let mut missing = Vec::new();
            for (name, _) in declared {
                if !s.fields.contains_key(&name) {
                    missing.push(c.interner.lookup(name));
                }
            }
            if !missing.is_empty() {
                missing.sort();
                c.error(
                    format!("missing: {}", missing.join(", ")),
                    c.ast.exprs.span(node),
                );
            }

            c.symbol_types[&symbol]
        }
        Expr::StructField { obj, name } => {
            let expr_ty = check_expr(c, obj);
            let Ty::Named(symbol) = c.types.probe_value(expr_ty) else {
                c.error("expect a struct instance", c.ast.exprs.span(obj));
                return c.types.new_key(Ty::Unknown);
            };
            let Some(properties) = c.struct_properties.get(&symbol) else {
                c.error("expect a struct instance", c.ast.exprs.span(obj));
                return c.types.new_key(Ty::Unknown);
            };

            let Some(&ty) = properties.get(&name) else {
                c.error("unknown field", c.ast.exprs.span(obj));
                return c.types.new_key(Ty::Unknown);
            };
            ty
        }
        Expr::For {
            init,
            cond,
            incr,
            body,
        } => {
            if let Some(init) = init {
                check_stmt(c, init);
            }
            if let Some(cond) = cond {
                let ty = check_expr(c, cond);
                c.unify(c.std.boolean, ty, c.ast.exprs.span(cond));
            }
            if let Some(incr) = incr {
                check_expr(c, incr);
            }
            let body_ty = check_expr(c, body);
            c.unify(c.std.unit, body_ty, c.ast.exprs.span(body));
            c.std.unit
        }
        Expr::Array(ref elements) => {
            let element_ty = c.types.new_key(Ty::Unknown);
            for &expr in elements {
                let ty = check_expr(c, expr);
                c.unify(element_ty, ty, c.ast.exprs.span(expr));
            }
            c.types.new_key(Ty::Array(element_ty))
        }
        Expr::ArrayRepeat { expr, count } => {
            let element_ty = check_expr(c, expr);
            let count_ty = check_expr(c, count);
            c.unify(c.std.u64, count_ty, c.ast.exprs.span(count));
            c.types.new_key(Ty::Array(element_ty))
        }
        Expr::ArrayIndex { array, index } => {
            let array_ty = check_expr(c, array);
            let index_ty = check_expr(c, index);
            c.unify(c.std.u64, index_ty, c.ast.exprs.span(index));

            match c.types.probe_value(array_ty) {
                Ty::Array(element_ty) => element_ty,
                _ => {
                    c.error("expect array", c.ast.exprs.span(array));
                    c.types.new_key(Ty::Unknown)
                }
            }
        }
    }
}

fn check_type(c: &mut TypeChecker, node: TypeId) -> TyId {
    match c.ast.types[node] {
        Type::FnType { ref params, ret } => {
            let params = params.iter().map(|t| check_type(c, *t)).collect();
            let ret = ret.map(|t| check_type(c, t)).unwrap_or(c.std.unit);
            c.types.new_key(Ty::Fn(FnTy {
                type_params: [].into(),
                params,
                ret,
            }))
        }
        Type::Unit => c.std.unit,
        Type::Named(..) => {
            let symbol = c.ast.types.type_binding(node);
            c.symbol_types[&symbol]
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

impl TyId {
    pub fn display(self, c: &mut TypeChecker) -> String {
        let mut buf = String::new();
        write_type(self, c, &mut buf);
        buf
    }
}

fn write_type(ty: TyId, c: &mut TypeChecker, buf: &mut String) {
    match c.types.probe_value(ty) {
        Ty::Named(symbol) | Ty::TypeParam(symbol) => {
            buf.push_str(c.interner.lookup(c.bcx.symbols[symbol].name))
        }
        Ty::Fn(f) => {
            buf.push_str("fn");
            if !f.type_params.is_empty() {
                buf.push_str("<");
                let mut sep = "";
                for p in f.type_params.iter() {
                    buf.push_str(sep);
                    write_type(*p, c, buf);
                    sep = ", ";
                }
                buf.push_str(">");
            }
            buf.push_str("(");
            let mut sep = "";
            for p in f.params.iter() {
                buf.push_str(sep);
                write_type(*p, c, buf);
                sep = ", ";
            }
            buf.push_str(") -> ");
            write_type(f.ret, c, buf);
        }
        Ty::Unit => buf.push_str("()"),
        Ty::Bool => buf.push_str("bool"),
        Ty::F64 => buf.push_str("f64"),
        Ty::I64 => buf.push_str("i64"),
        Ty::U64 => buf.push_str("u64"),
        Ty::UnknownInt => buf.push_str("{integer}"),
        Ty::String => buf.push_str("string"),
        Ty::Unknown => buf.push_str("<cannot infer>"),
        Ty::Never => buf.push_str("never"),
        Ty::Tuple(types) => {
            buf.push_str("(");
            let mut sep = "";
            for t in types.iter() {
                buf.push_str(sep);
                write_type(*t, c, buf);
                sep = ", ";
            }
            buf.push_str(")")
        }
        Ty::Array(element) => {
            buf.push_str("[");
            write_type(element, c, buf);
            buf.push_str("]");
        }
    }
}
