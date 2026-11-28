use std::rc::Rc;

use ena::unify::{InPlaceUnificationTable, NoError, UnifyKey, UnifyValue};
use rustc_hash::FxHashMap;

use crate::bind::SymbolId;

pub type TyTable = InPlaceUnificationTable<TyId>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    Never,
    Unknown,
    UnknownInt,
    Unit,
    Bool,
    F64,
    I64,
    U64,
    String,
    Named(SymbolId),
    TypeParam(SymbolId),
    Fn(Rc<[TyId]>, TyId),
    Tuple(Rc<[TyId]>),
    Array(TyId),
}

pub struct StdTypes {
    pub boolean: TyId,
    pub f64: TyId,
    pub i64: TyId,
    pub u64: TyId,
    pub unit: TyId,
    pub string: TyId,
}

impl StdTypes {
    pub fn new(table: &mut TyTable) -> Self {
        Self {
            boolean: table.new_key(Ty::Bool),
            f64: table.new_key(Ty::F64),
            i64: table.new_key(Ty::I64),
            u64: table.new_key(Ty::U64),
            unit: table.new_key(Ty::Unit),
            string: table.new_key(Ty::String),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TyId(u32);

impl UnifyKey for TyId {
    type Value = Ty;
    fn index(&self) -> u32 {
        self.0
    }
    fn from_index(u: u32) -> Self {
        Self(u)
    }
    fn tag() -> &'static str {
        "TyVar"
    }
}

impl UnifyValue for Ty {
    type Error = NoError;

    fn unify_values(value1: &Self, value2: &Self) -> Result<Self, Self::Error> {
        match (value1, value2) {
            (Ty::Unknown, Ty::Unknown) => Ok(Ty::Unknown),
            (known, Ty::Unknown) | (Ty::Unknown, known) => Ok(known.clone()),

            (Ty::UnknownInt, Ty::UnknownInt) => Ok(Ty::UnknownInt),
            (known, Ty::UnknownInt) | (Ty::UnknownInt, known) => Ok(known.clone()),

            // We must've checked the compatibility in can_unify. Simply take any of them.
            _ => Ok(value1.clone()),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum CanonicalTy {
    Never,
    Unit,
    Bool,
    F64,
    I64,
    U64,
    String,
    Named(SymbolId),
    Fn {
        params: Vec<CanonicalTy>,
        ret: Box<CanonicalTy>,
    },
    Tuple(Vec<CanonicalTy>),
    Array(Box<CanonicalTy>),
}

impl TyId {
    pub fn canonicalize(self, table: &mut TyTable) -> CanonicalTy {
        match table.probe_value(self) {
            Ty::Never => CanonicalTy::Never,
            Ty::Unit => CanonicalTy::Unit,
            Ty::Bool => CanonicalTy::Bool,
            Ty::F64 => CanonicalTy::F64,
            Ty::I64 => CanonicalTy::I64,
            Ty::U64 => CanonicalTy::U64,
            Ty::String => CanonicalTy::String,
            Ty::Named(id) => CanonicalTy::Named(id),
            Ty::Fn(params, ret) => CanonicalTy::Fn {
                params: params.iter().map(|t| t.canonicalize(table)).collect(),
                ret: Box::new(ret.canonicalize(table)),
            },
            Ty::Tuple(fields) => {
                CanonicalTy::Tuple(fields.iter().map(|t| t.canonicalize(table)).collect())
            }
            Ty::Array(element) => CanonicalTy::Array(Box::new(element.canonicalize(table))),
            Ty::Unknown | Ty::UnknownInt | Ty::TypeParam(..) => {
                unreachable!("Cannot canonicalize an unknown type.")
            }
        }
    }

    pub fn instantiate(self, types: &mut TyTable, type_params: &[TyId]) -> Self {
        let mut subst = FxHashMap::default();
        for t in type_params {
            subst.insert(*t, types.new_key(Ty::Unknown));
        }
        self.substitute(types, &subst)
    }

    pub fn substitute(self, types: &mut TyTable, subst: &FxHashMap<TyId, TyId>) -> TyId {
        let t = types.find(self);
        match types.probe_value(t) {
            Ty::Fn(params, ret) => {
                let params = params.iter().map(|p| p.substitute(types, subst)).collect();
                let ret = ret.substitute(types, subst);
                types.new_key(Ty::Fn(params, ret))
            }
            Ty::Tuple(params) => {
                let params = params.iter().map(|p| p.substitute(types, subst)).collect();
                types.new_key(Ty::Tuple(params))
            }
            Ty::Array(el) => {
                let el = el.substitute(types, subst);
                types.new_key(Ty::Array(el))
            }
            Ty::TypeParam(_) => subst.get(&t).copied().unwrap_or(t),
            _ => t,
        }
    }

    pub fn unify(self, other: TyId, table: &mut TyTable) {
        if table.unioned(self, other) {
            return;
        }
        let rhs_ty = table.probe_value(other);
        if rhs_ty == Ty::Never {
            return;
        }
        let lhs_ty = table.probe_value(self);
        table.union(self, other);
        match (lhs_ty, rhs_ty) {
            (Ty::Tuple(lhs), Ty::Tuple(rhs)) if lhs.len() == rhs.len() => {
                for (lhs, rhs) in lhs.iter().zip(rhs.iter()) {
                    lhs.unify(*rhs, table);
                }
            }
            (Ty::Fn(lhs_params, lhs_ret), Ty::Fn(rhs_params, rhs_ret)) => {
                for (lhs_param, rhs_param) in lhs_params.iter().zip(rhs_params.iter()) {
                    lhs_param.unify(*rhs_param, table);
                }
                lhs_ret.unify(rhs_ret, table);
            }
            (Ty::Array(lhs_element), Ty::Array(rhs_element)) => {
                lhs_element.unify(rhs_element, table)
            }
            _ => {}
        }
    }

    pub fn debug(self, table: &mut TyTable) -> String {
        let mut buf = String::new();
        write_type(self, table, &mut buf);
        buf
    }
}

fn write_type(ty: TyId, table: &mut TyTable, buf: &mut String) {
    use std::fmt::Write;

    match table.probe_value(ty) {
        Ty::Named(symbol) => {
            write!(buf, "Named({symbol:?})").unwrap();
        }
        Ty::TypeParam(symbol) => {
            write!(buf, "TypeParam({symbol:?})").unwrap();
        }
        Ty::Fn(params, ret) => {
            buf.push_str("fn(");
            let mut sep = "";
            for p in params.iter() {
                buf.push_str(sep);
                write_type(*p, table, buf);
                sep = ", ";
            }
            buf.push_str(") -> ");
            write_type(ret, table, buf);
        }
        Ty::Unit => buf.push_str("()"),
        Ty::Bool => buf.push_str("bool"),
        Ty::F64 => buf.push_str("f64"),
        Ty::I64 => buf.push_str("i64"),
        Ty::U64 => buf.push_str("u64"),
        Ty::UnknownInt => buf.push_str("UnknownInt"),
        Ty::String => buf.push_str("string"),
        Ty::Unknown => buf.push_str("Unknown"),
        Ty::Never => buf.push_str("never"),
        Ty::Tuple(types) => {
            buf.push_str("(");
            let mut sep = "";
            for t in types.iter() {
                buf.push_str(sep);
                write_type(*t, table, buf);
                sep = ", ";
            }
            buf.push_str(")")
        }
        Ty::Array(element) => {
            buf.push_str("[");
            write_type(element, table, buf);
            buf.push_str("]");
        }
    }
}
