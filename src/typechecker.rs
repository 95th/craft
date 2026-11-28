use std::rc::Rc;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    Error, FxIndexMap,
    ast::{AstNodes, BinaryOp, Expr, ExprId, Span, Stmt, StmtId, Type, TypeId},
    bind::{BindCtxt, SymbolId},
    diagnostics::Diagnostics,
    interner::{StrId, StrInterner},
    types::{FnTy, StdTypes, Typ, TypId, TypTable},
};

pub struct TyCtxt {
    pub bcx: BindCtxt,
    pub types: TypTable,
    pub struct_properties: FxHashMap<SymbolId, FxIndexMap<StrId, TypId>>,
    pub tuple_struct_properties: FxHashMap<SymbolId, Rc<[TypId]>>,
    pub expr_types: FxHashMap<ExprId, TypId>,
    pub symbol_types: FxHashMap<SymbolId, TypId>,
}

pub struct TypeChecker<'a> {
    ast: &'a AstNodes,
    interner: &'a StrInterner,
    diagnostics: &'a mut Diagnostics,
    bcx: BindCtxt,
    types: TypTable,
    std: StdTypes,
    has_error: bool,
    containing_return_typ: Option<TypId>,
    symbol_types: FxHashMap<SymbolId, TypId>,
    struct_properties: FxHashMap<SymbolId, FxIndexMap<StrId, TypId>>,
    tuple_struct_properties: FxHashMap<SymbolId, Rc<[TypId]>>,
    expr_types: FxHashMap<ExprId, TypId>,
    integer_literals: Vec<(ExprId, TypId)>,
    deferred_negation_checks: Vec<(ExprId, TypId)>,
    diverges: bool,
}

impl<'a> TypeChecker<'a> {
    pub fn new(
        ast: &'a AstNodes,
        interner: &'a StrInterner,
        diagnostics: &'a mut Diagnostics,
        bcx: BindCtxt,
    ) -> Self {
        let mut types = TypTable::new();
        let std = StdTypes::new(&mut types);
        Self {
            ast,
            interner,
            diagnostics,
            bcx,
            types,
            std,
            has_error: false,
            containing_return_typ: None,
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
            let arg = self.types.new_key(Typ::TypeParam(self.bcx.std.print));
            let params = Rc::<[TypId]>::from([arg]);
            let fn_typ = self.types.new_key(Typ::Fn(FnTy {
                type_params: params.clone(),
                params,
                ret: self.std.unit,
            }));
            self.symbol_types.insert(self.bcx.std.print, fn_typ);
        }

        {
            let fn_typ = self.types.new_key(Typ::Fn(FnTy {
                type_params: [].into(),
                params: [self.std.boolean].into(),
                ret: self.std.unit,
            }));
            self.symbol_types.insert(self.bcx.std.assert, fn_typ);
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

    fn unify(&mut self, expected: TypId, actual: TypId, span: Span) {
        if self.can_unify(expected, actual) {
            expected.unify(actual, &mut self.types);
        } else {
            let expected = expected.display(self);
            let actual = actual.display(self);
            self.error(format!("expect `{expected}`, found `{actual}`"), span);
        }
    }

    fn can_unify(&mut self, expected: TypId, actual: TypId) -> bool {
        if self.types.unioned(expected, actual) {
            return true;
        }
        match (
            self.types.probe_value(expected),
            self.types.probe_value(actual),
        ) {
            (Typ::Tuple(a), Typ::Tuple(b)) => self.can_unify_list(&a, &b),
            (Typ::Fn(a), Typ::Fn(b)) => {
                self.can_unify_list(&a.params, &b.params) && self.can_unify(a.ret, b.ret)
            }
            (Typ::Array(a), Typ::Array(b)) => self.can_unify(a, b),
            (Typ::Unknown, _) | (_, Typ::Unknown) => true,
            (Typ::UnknownInt, Typ::UnknownInt) => true,
            (Typ::UnknownInt, Typ::I64 | Typ::U64) | (Typ::I64 | Typ::U64, Typ::UnknownInt) => true,
            (_, Typ::Never) => true,
            _ => false,
        }
    }

    fn can_unify_list(&mut self, expected: &[TypId], actual: &[TypId]) -> bool {
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
    for (expr, typ) in c.expr_types.iter() {
        let typ = c.types.find(*typ);
        if !seen.insert(typ) {
            continue;
        }
        if has_unknown_typ(&mut c.types, typ) {
            let span = c.ast.exprs.span(*expr);
            c.diagnostics.push("Cannot infer type", span);
            c.has_error = true;
        }
    }
}

fn has_unknown_typ(types: &mut TypTable, typ: TypId) -> bool {
    match types.probe_value(typ) {
        Typ::Unknown | Typ::UnknownInt => true,
        Typ::Unit
        | Typ::Bool
        | Typ::F64
        | Typ::I64
        | Typ::U64
        | Typ::String
        | Typ::Never
        | Typ::Named(..)
        | Typ::TypeParam(..) => false,
        Typ::Fn(f) => {
            has_unknown_typ(types, f.ret)
                || f.params.iter().any(|t| has_unknown_typ(types, *t))
                || f.type_params.iter().any(|t| has_unknown_typ(types, *t))
        }
        Typ::Tuple(params) => params.iter().any(|t| has_unknown_typ(types, *t)),
        Typ::Array(element) => has_unknown_typ(types, element),
    }
}

fn deferred_check_integers(c: &mut TypeChecker) {
    for (expr, typ) in std::mem::take(&mut c.integer_literals) {
        match c.types.probe_value(typ) {
            Typ::UnknownInt => {
                c.types.union_value(typ, Typ::I64);
                check_literal_range_i64(c, expr);
            }
            Typ::I64 => check_literal_range_i64(c, expr),
            Typ::U64 => check_literal_range_u64(c, expr),
            _ => unreachable!(),
        }
    }

    for (expr, typ) in std::mem::take(&mut c.deferred_negation_checks) {
        if let Typ::U64 = c.types.probe_value(typ) {
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

fn check_scope(c: &mut TypeChecker, nodes: &[StmtId], trailing_expr: Option<ExprId>) -> TypId {
    let prev_diverges = std::mem::take(&mut c.diverges);
    for node in nodes {
        declare_item(c, *node);
    }
    for node in nodes {
        check_stmt(c, *node);
    }
    let typ = trailing_expr.map(|expr| check_expr(c, expr));
    let diverges = std::mem::replace(&mut c.diverges, prev_diverges);
    if let Some(typ) = typ {
        typ
    } else if diverges {
        c.types.new_key(Typ::Unknown)
    } else {
        c.std.unit
    }
}

fn declare_item(c: &mut TypeChecker, node: StmtId) {
    match c.ast.stmts[node] {
        Stmt::Fn(ref f) => {
            let type_params = f.type_params.iter().map(|p| check_stmt(c, *p)).collect();
            let params = f.params.iter().map(|p| check_stmt(c, *p)).collect();
            let ret = f.return_typ.map(|t| check_typ(c, t)).unwrap_or(c.std.unit);
            let typ = c.types.new_key(Typ::Fn(FnTy {
                type_params,
                params,
                ret,
            }));
            let symbol = c.ast.stmts.value_binding(node);
            c.symbol_types.insert(symbol, typ);
        }
        Stmt::Struct(..) => {
            let symbol = c.ast.stmts.type_binding(node);
            let typ = c.types.new_key(Typ::Named(symbol));
            c.symbol_types.insert(symbol, typ);
        }
        Stmt::TupleStruct { .. } => {
            let symbol = c.ast.stmts.type_binding(node);
            let typ = c.types.new_key(Typ::Named(symbol));
            c.symbol_types.insert(symbol, typ);
        }
        _ => {}
    }
}

fn check_stmt(c: &mut TypeChecker, node: StmtId) -> TypId {
    match c.ast.stmts[node] {
        Stmt::Let { typ, init, .. } => {
            let init_typ = check_expr(c, init);
            if let Some(typ) = typ {
                let typ = check_typ(c, typ);
                c.unify(typ, init_typ, c.ast.exprs.span(init));
            }
            let symbol = c.ast.stmts.value_binding(node);
            c.symbol_types.insert(symbol, init_typ);
            c.std.unit
        }
        Stmt::Fn(ref f) => {
            let fn_symbol = c.ast.stmts.value_binding(node);
            let fn_typ = c.symbol_types[&fn_symbol];
            let Typ::Fn(fn_typ) = c.types.probe_value(fn_typ) else {
                unreachable!();
            };

            let prev_containing_return_typ = c.containing_return_typ.replace(fn_typ.ret);
            let body_typ = check_expr(c, f.body);
            c.unify(fn_typ.ret, body_typ, c.ast.exprs.span(f.body));
            c.containing_return_typ = prev_containing_return_typ;
            c.std.unit
        }
        Stmt::Param { typ, .. } => {
            let typ = check_typ(c, typ);
            let symbol = c.ast.stmts.value_binding(node);
            c.symbol_types.insert(symbol, typ);
            typ
        }
        Stmt::Expr { expr, semi } => {
            let typ = check_expr(c, expr);
            if semi { c.std.unit } else { typ }
        }
        Stmt::Struct(ref s) => {
            let symbol = c.ast.stmts.type_binding(node);
            let mut properties = FxIndexMap::default();
            for (&name, &typ) in s.fields.iter() {
                let typ = check_typ(c, typ);
                properties.insert(name, typ);
            }
            c.struct_properties.insert(symbol, properties);
            c.std.unit
        }
        Stmt::TupleStruct(ref s) => {
            let type_symbol = c.ast.stmts.type_binding(node);
            let mut arg_tys = Vec::new();
            for typ in s.fields.iter() {
                let typ = check_typ(c, *typ);
                arg_tys.push(typ);
            }
            let arg_tys: Rc<[TypId]> = arg_tys.into();
            c.tuple_struct_properties
                .insert(type_symbol, arg_tys.clone());

            let struct_typ = c.symbol_types[&type_symbol];
            let fn_typ = c.types.new_key(Typ::Fn(FnTy {
                type_params: [].into(),
                params: arg_tys,
                ret: struct_typ,
            }));
            let value_symbol = c.ast.stmts.value_binding(node);
            c.symbol_types.insert(value_symbol, fn_typ);
            c.std.unit
        }
        Stmt::TypeParam(..) => {
            let symbol = c.ast.stmts.type_binding(node);
            let typ = c.types.new_key(Typ::TypeParam(symbol));
            c.symbol_types.insert(symbol, typ);
            typ
        }
    }
}

fn check_expr(c: &mut TypeChecker, node: ExprId) -> TypId {
    let typ = check_expr_inner(c, node);
    let prev = c.expr_types.insert(node, typ);
    debug_assert_eq!(prev, None);
    typ
}

fn check_expr_inner(c: &mut TypeChecker, node: ExprId) -> TypId {
    match c.ast.exprs[node] {
        Expr::Bool(_) => c.std.boolean,
        Expr::Int(_) => {
            let typ = c.types.new_key(Typ::UnknownInt);
            c.integer_literals.push((node, typ));
            typ
        }
        Expr::Number(_) => c.std.f64,
        Expr::String(_) => c.std.string,
        Expr::Assign { lhs, rhs } => {
            let lhs_typ = check_expr(c, lhs);
            let rhs_typ = check_expr(c, rhs);
            c.unify(lhs_typ, rhs_typ, c.ast.exprs.span(rhs));
            c.std.unit
        }
        Expr::Binary { lhs, op, rhs } => match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                let lhs_typ = check_expr(c, lhs);
                let rhs_typ = check_expr(c, rhs);
                c.unify(lhs_typ, rhs_typ, c.ast.exprs.span(rhs));
                match c.types.probe_value(lhs_typ) {
                    Typ::I64 | Typ::U64 | Typ::F64 | Typ::UnknownInt => lhs_typ,
                    _ => {
                        let message = format!(
                            "expect `i64`, `u64` or `f64`, found `{}`",
                            lhs_typ.display(c)
                        );
                        c.error(message, c.ast.exprs.span(lhs));
                        c.types.new_key(Typ::Unknown)
                    }
                }
            }
            BinaryOp::BitOr | BinaryOp::BitAnd => {
                let lhs_typ = check_expr(c, lhs);
                let rhs_typ = check_expr(c, rhs);
                c.unify(lhs_typ, rhs_typ, c.ast.exprs.span(rhs));
                match c.types.probe_value(lhs_typ) {
                    Typ::I64 | Typ::U64 | Typ::UnknownInt => lhs_typ,
                    _ => {
                        let message =
                            format!("expect `i64` or `u64`, found `{}`", lhs_typ.display(c));
                        c.error(message, c.ast.exprs.span(lhs));
                        c.types.new_key(Typ::Unknown)
                    }
                }
            }
            BinaryOp::Greater | BinaryOp::GreaterEqual | BinaryOp::Less | BinaryOp::LessEqual => {
                let lhs_typ = check_expr(c, lhs);
                let rhs_typ = check_expr(c, rhs);
                c.unify(lhs_typ, rhs_typ, c.ast.exprs.span(rhs));
                match c.types.probe_value(lhs_typ) {
                    Typ::I64 | Typ::U64 | Typ::F64 | Typ::UnknownInt => {}
                    _ => {
                        let message = format!(
                            "expect `i64`, `u64` or `f64`, found `{}`",
                            lhs_typ.display(c)
                        );
                        c.error(message, c.ast.exprs.span(lhs));
                    }
                }
                c.std.boolean
            }
            BinaryOp::Equal | BinaryOp::NotEqual => {
                let lhs_typ = check_expr(c, lhs);
                let rhs_typ = check_expr(c, rhs);
                c.unify(lhs_typ, rhs_typ, c.ast.exprs.span(rhs));
                c.std.boolean
            }
            BinaryOp::Or | BinaryOp::And => {
                let lhs_typ = check_expr(c, lhs);
                c.unify(c.std.boolean, lhs_typ, c.ast.exprs.span(lhs));
                let prev_diverges = std::mem::take(&mut c.diverges);
                let rhs_typ = check_expr(c, rhs);
                c.unify(c.std.boolean, rhs_typ, c.ast.exprs.span(rhs));
                c.diverges = prev_diverges;
                c.std.boolean
            }
        },
        Expr::Not(expr) => {
            let typ = check_expr(c, expr);
            c.unify(c.std.boolean, typ, c.ast.exprs.span(expr));
            c.std.boolean
        }
        Expr::Negate(expr) => {
            let typ = check_expr(c, expr);
            match c.types.probe_value(typ) {
                Typ::I64 | Typ::F64 => typ,
                Typ::UnknownInt => {
                    c.deferred_negation_checks.push((expr, typ));
                    typ
                }
                _ => {
                    let message = format!("expect `i64` or `f64`, found `{}`", typ.display(c));
                    c.error(message, c.ast.exprs.span(expr));
                    c.types.new_key(Typ::Unknown)
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
            let cond_typ = check_expr(c, cond);
            c.unify(c.std.boolean, cond_typ, c.ast.exprs.span(cond));
            let then_clause_typ = check_expr(c, then_clause);
            if let Some(else_clause) = else_clause {
                let else_clause_typ = check_expr(c, else_clause);
                c.unify(
                    then_clause_typ,
                    else_clause_typ,
                    c.ast.exprs.span(else_clause),
                );
                then_clause_typ
            } else {
                c.unify(c.std.unit, then_clause_typ, c.ast.exprs.span(then_clause));
                then_clause_typ
            }
        }
        Expr::Block { ref stmts, expr } => crate::stack_safe(|| check_scope(c, stmts, expr)),
        Expr::While { cond, body } => {
            let cond_typ = check_expr(c, cond);
            c.unify(c.std.boolean, cond_typ, c.ast.exprs.span(cond));
            let body_typ = check_expr(c, body);
            c.unify(c.std.unit, body_typ, c.ast.exprs.span(body));
            c.std.unit
        }
        Expr::Break => {
            c.diverges = true;
            c.types.new_key(Typ::Never)
        }
        Expr::Continue => {
            c.diverges = true;
            c.types.new_key(Typ::Never)
        }
        Expr::Call { callee, ref args } => {
            let callee_typ = check_expr(c, callee);
            let Typ::Fn(f) = c.types.probe_value(callee_typ) else {
                c.error("call expression requires function", c.ast.exprs.span(node));
                return c.types.new_key(Typ::Unknown);
            };
            if args.len() != f.params.len() {
                c.error(
                    format!("expect {} arguments, found {}", f.params.len(), args.len()),
                    c.ast.exprs.span(node),
                );
                return c.types.new_key(Typ::Unknown);
            }
            for (&arg, param) in args.iter().zip(f.params.iter()) {
                let arg_typ = check_expr(c, arg);
                c.unify(*param, arg_typ, c.ast.exprs.span(arg));
            }
            f.ret
        }
        Expr::Return(value) => {
            let value_typ = if let Some(value) = value {
                check_expr(c, value)
            } else {
                c.std.unit
            };
            c.unify(
                value_typ,
                c.containing_return_typ.unwrap(),
                c.ast.exprs.span(node),
            );
            c.diverges = true;
            c.types.new_key(Typ::Never)
        }
        Expr::Unit => c.std.unit,
        Expr::Tuple(ref types) => {
            let types = types.iter().map(|t| check_expr(c, *t)).collect();
            c.types.new_key(Typ::Tuple(types))
        }
        Expr::TupleField { obj, index } => {
            let expr_typ = check_expr(c, obj);
            let types = match c.types.probe_value(expr_typ) {
                Typ::Tuple(types) => types,
                Typ::Named(symbol) => {
                    if let Some(fields) = c.tuple_struct_properties.get(&symbol) {
                        fields.clone()
                    } else {
                        c.error("expect a tuple", c.ast.exprs.span(obj));
                        return c.types.new_key(Typ::Unknown);
                    }
                }
                _ => {
                    c.error("expect a tuple", c.ast.exprs.span(obj));
                    return c.types.new_key(Typ::Unknown);
                }
            };
            if index as usize >= types.len() {
                c.error("unknown field", c.ast.exprs.span(node));
                return c.types.new_key(Typ::Unknown);
            }
            types[index as usize]
        }
        Expr::Struct(ref s) => {
            let symbol = c.ast.exprs.value_binding(node);
            let declared = c.struct_properties[&symbol].clone();
            for (name, &value) in s.fields.iter() {
                let typ = check_expr(c, value);
                if let Some(&declared_typ) = declared.get(name) {
                    c.unify(declared_typ, typ, c.ast.exprs.span(value));
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
            let expr_typ = check_expr(c, obj);
            let Typ::Named(symbol) = c.types.probe_value(expr_typ) else {
                c.error("expect a struct instance", c.ast.exprs.span(obj));
                return c.types.new_key(Typ::Unknown);
            };
            let Some(properties) = c.struct_properties.get(&symbol) else {
                c.error("expect a struct instance", c.ast.exprs.span(obj));
                return c.types.new_key(Typ::Unknown);
            };

            let Some(&typ) = properties.get(&name) else {
                c.error("unknown field", c.ast.exprs.span(obj));
                return c.types.new_key(Typ::Unknown);
            };
            typ
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
                let typ = check_expr(c, cond);
                c.unify(c.std.boolean, typ, c.ast.exprs.span(cond));
            }
            if let Some(incr) = incr {
                check_expr(c, incr);
            }
            let body_typ = check_expr(c, body);
            c.unify(c.std.unit, body_typ, c.ast.exprs.span(body));
            c.std.unit
        }
        Expr::Array(ref elements) => {
            let element_typ = c.types.new_key(Typ::Unknown);
            for &expr in elements {
                let typ = check_expr(c, expr);
                c.unify(element_typ, typ, c.ast.exprs.span(expr));
            }
            c.types.new_key(Typ::Array(element_typ))
        }
        Expr::ArrayRepeat { expr, count } => {
            let element_typ = check_expr(c, expr);
            let count_typ = check_expr(c, count);
            c.unify(c.std.u64, count_typ, c.ast.exprs.span(count));
            c.types.new_key(Typ::Array(element_typ))
        }
        Expr::ArrayIndex { array, index } => {
            let array_typ = check_expr(c, array);
            let index_typ = check_expr(c, index);
            c.unify(c.std.u64, index_typ, c.ast.exprs.span(index));

            match c.types.probe_value(array_typ) {
                Typ::Array(element_typ) => element_typ,
                _ => {
                    c.error("expect array", c.ast.exprs.span(array));
                    c.types.new_key(Typ::Unknown)
                }
            }
        }
    }
}

fn check_typ(c: &mut TypeChecker, node: TypeId) -> TypId {
    match c.ast.types[node] {
        Type::FnType { ref params, ret } => {
            let params = params.iter().map(|t| check_typ(c, *t)).collect();
            let ret = ret.map(|t| check_typ(c, t)).unwrap_or(c.std.unit);
            c.types.new_key(Typ::Fn(FnTy {
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
            let types = items.iter().map(|t| check_typ(c, *t)).collect();
            c.types.new_key(Typ::Tuple(types))
        }
        Type::Array(element) => {
            let typ = check_typ(c, element);
            c.types.new_key(Typ::Array(typ))
        }
    }
}

impl TypId {
    pub fn display(self, c: &mut TypeChecker) -> String {
        let mut buf = String::new();
        write_typ(self, c, &mut buf);
        buf
    }
}

fn write_typ(typ: TypId, c: &mut TypeChecker, buf: &mut String) {
    match c.types.probe_value(typ) {
        Typ::Named(symbol) | Typ::TypeParam(symbol) => {
            buf.push_str(c.interner.lookup(c.bcx.symbols[symbol].name))
        }
        Typ::Fn(f) => {
            buf.push_str("fn");
            if !f.type_params.is_empty() {
                buf.push_str("<");
                let mut sep = "";
                for p in f.type_params.iter() {
                    buf.push_str(sep);
                    write_typ(*p, c, buf);
                    sep = ", ";
                }
                buf.push_str(">");
            }
            buf.push_str("(");
            let mut sep = "";
            for p in f.params.iter() {
                buf.push_str(sep);
                write_typ(*p, c, buf);
                sep = ", ";
            }
            buf.push_str(") -> ");
            write_typ(f.ret, c, buf);
        }
        Typ::Unit => buf.push_str("()"),
        Typ::Bool => buf.push_str("bool"),
        Typ::F64 => buf.push_str("f64"),
        Typ::I64 => buf.push_str("i64"),
        Typ::U64 => buf.push_str("u64"),
        Typ::UnknownInt => buf.push_str("{integer}"),
        Typ::String => buf.push_str("string"),
        Typ::Unknown => buf.push_str("<cannot infer>"),
        Typ::Never => buf.push_str("never"),
        Typ::Tuple(types) => {
            buf.push_str("(");
            let mut sep = "";
            for t in types.iter() {
                buf.push_str(sep);
                write_typ(*t, c, buf);
                sep = ", ";
            }
            buf.push_str(")")
        }
        Typ::Array(element) => {
            buf.push_str("[");
            write_typ(element, c, buf);
            buf.push_str("]");
        }
    }
}
