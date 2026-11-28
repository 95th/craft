use std::rc::Rc;

use ena::unify::{InPlaceUnificationTable, NoError, UnifyKey, UnifyValue};
use rustc_hash::FxHashMap;

use crate::bind::SymbolId;

pub type TypTable = InPlaceUnificationTable<TypId>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Typ {
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
    Fn(FnTy),
    Tuple(Rc<[TypId]>),
    Array(TypId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnTy {
    pub type_params: Rc<[TypId]>,
    pub params: Rc<[TypId]>,
    pub ret: TypId,
}

pub struct StdTypes {
    pub boolean: TypId,
    pub f64: TypId,
    pub i64: TypId,
    pub u64: TypId,
    pub unit: TypId,
    pub string: TypId,
}

impl StdTypes {
    pub fn new(table: &mut TypTable) -> Self {
        Self {
            boolean: table.new_key(Typ::Bool),
            f64: table.new_key(Typ::F64),
            i64: table.new_key(Typ::I64),
            u64: table.new_key(Typ::U64),
            unit: table.new_key(Typ::Unit),
            string: table.new_key(Typ::String),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypId(u32);

impl UnifyKey for TypId {
    type Value = Typ;
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

impl UnifyValue for Typ {
    type Error = NoError;

    fn unify_values(value1: &Self, value2: &Self) -> Result<Self, Self::Error> {
        match (value1, value2) {
            (Typ::Unknown, Typ::Unknown) => Ok(Typ::Unknown),
            (known, Typ::Unknown) | (Typ::Unknown, known) => Ok(known.clone()),

            (Typ::UnknownInt, Typ::UnknownInt) => Ok(Typ::UnknownInt),
            (known, Typ::UnknownInt) | (Typ::UnknownInt, known) => Ok(known.clone()),

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

impl TypId {
    pub fn canonicalize(self, table: &mut TypTable) -> CanonicalTy {
        match table.probe_value(self) {
            Typ::Never => CanonicalTy::Never,
            Typ::Unit => CanonicalTy::Unit,
            Typ::Bool => CanonicalTy::Bool,
            Typ::F64 => CanonicalTy::F64,
            Typ::I64 => CanonicalTy::I64,
            Typ::U64 => CanonicalTy::U64,
            Typ::String => CanonicalTy::String,
            Typ::Named(id) => CanonicalTy::Named(id),
            Typ::Fn(f) => {
                if cfg!(debug_assertions) {
                    // All type params must be fully resolved here.
                    for p in f.type_params.iter() {
                        assert!(!matches!(
                            table.probe_value(*p),
                            Typ::TypeParam(..) | Typ::Unknown | Typ::UnknownInt
                        ));
                    }
                }
                CanonicalTy::Fn {
                    params: f.params.iter().map(|t| t.canonicalize(table)).collect(),
                    ret: Box::new(f.ret.canonicalize(table)),
                }
            }
            Typ::Tuple(fields) => {
                CanonicalTy::Tuple(fields.iter().map(|t| t.canonicalize(table)).collect())
            }
            Typ::Array(element) => CanonicalTy::Array(Box::new(element.canonicalize(table))),
            Typ::Unknown | Typ::UnknownInt | Typ::TypeParam(..) => {
                unreachable!("Cannot canonicalize an unknown type.")
            }
        }
    }

    pub fn type_params(self, types: &mut TypTable) -> Option<Rc<[TypId]>> {
        match types.probe_value(self) {
            Typ::Fn(f) if !f.type_params.is_empty() => Some(f.type_params.clone()),
            _ => None,
        }
    }

    pub fn instantiate(self, types: &mut TypTable) -> Self {
        let Some(type_params) = self.type_params(types) else {
            return self;
        };

        let mut subst = FxHashMap::default();
        for t in type_params.iter() {
            subst.insert(*t, types.new_key(Typ::Unknown));
        }
        self.substitute(types, &subst)
    }

    pub fn substitute(self, types: &mut TypTable, subst: &FxHashMap<TypId, TypId>) -> TypId {
        let t = types.find(self);
        match types.probe_value(t) {
            Typ::Fn(f) => {
                let type_params = f
                    .type_params
                    .iter()
                    .map(|p| p.substitute(types, subst))
                    .collect();
                let params = f
                    .params
                    .iter()
                    .map(|p| p.substitute(types, subst))
                    .collect();
                let ret = f.ret.substitute(types, subst);
                types.new_key(Typ::Fn(FnTy {
                    type_params,
                    params,
                    ret,
                }))
            }
            Typ::Tuple(params) => {
                let params = params.iter().map(|p| p.substitute(types, subst)).collect();
                types.new_key(Typ::Tuple(params))
            }
            Typ::Array(el) => {
                let el = el.substitute(types, subst);
                types.new_key(Typ::Array(el))
            }
            Typ::TypeParam(_) => subst.get(&t).copied().unwrap_or(t),
            _ => t,
        }
    }

    pub fn unify(self, other: TypId, table: &mut TypTable) {
        if table.unioned(self, other) {
            return;
        }
        let rhs_typ = table.probe_value(other);
        if rhs_typ == Typ::Never {
            return;
        }
        let lhs_typ = table.probe_value(self);
        table.union(self, other);
        match (lhs_typ, rhs_typ) {
            (Typ::Tuple(lhs), Typ::Tuple(rhs)) if lhs.len() == rhs.len() => {
                for (lhs, rhs) in lhs.iter().zip(rhs.iter()) {
                    lhs.unify(*rhs, table);
                }
            }
            (Typ::Fn(lhs), Typ::Fn(rhs)) => {
                for (lhs, rhs) in lhs.params.iter().zip(rhs.params.iter()) {
                    lhs.unify(*rhs, table);
                }
                lhs.ret.unify(rhs.ret, table);
            }
            (Typ::Array(lhs_element), Typ::Array(rhs_element)) => {
                lhs_element.unify(rhs_element, table)
            }
            _ => {}
        }
    }

    pub fn debug(self, table: &mut TypTable) -> String {
        let mut buf = String::new();
        write_typ(self, table, &mut buf);
        buf
    }
}

fn write_typ(typ: TypId, table: &mut TypTable, buf: &mut String) {
    use std::fmt::Write;

    match table.probe_value(typ) {
        Typ::Named(symbol) => {
            write!(buf, "Named({symbol:?})").unwrap();
        }
        Typ::TypeParam(symbol) => {
            write!(buf, "TypeParam({symbol:?})").unwrap();
        }
        Typ::Fn(f) => {
            buf.push_str("fn");
            if !f.type_params.is_empty() {
                buf.push_str("<");
                let mut sep = "";
                for p in f.type_params.iter() {
                    buf.push_str(sep);
                    write_typ(*p, table, buf);
                    sep = ", ";
                }
                buf.push_str(">");
            }
            buf.push_str("(");
            let mut sep = "";
            for p in f.params.iter() {
                buf.push_str(sep);
                write_typ(*p, table, buf);
                sep = ", ";
            }
            buf.push_str(") -> ");
            write_typ(f.ret, table, buf);
        }
        Typ::Unit => buf.push_str("()"),
        Typ::Bool => buf.push_str("bool"),
        Typ::F64 => buf.push_str("f64"),
        Typ::I64 => buf.push_str("i64"),
        Typ::U64 => buf.push_str("u64"),
        Typ::UnknownInt => buf.push_str("UnknownInt"),
        Typ::String => buf.push_str("string"),
        Typ::Unknown => buf.push_str("Unknown"),
        Typ::Never => buf.push_str("never"),
        Typ::Tuple(types) => {
            buf.push_str("(");
            let mut sep = "";
            for t in types.iter() {
                buf.push_str(sep);
                write_typ(*t, table, buf);
                sep = ", ";
            }
            buf.push_str(")")
        }
        Typ::Array(element) => {
            buf.push_str("[");
            write_typ(element, table, buf);
            buf.push_str("]");
        }
    }
}
