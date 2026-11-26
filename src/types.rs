use std::rc::Rc;

use ena::unify::{InPlaceUnificationTable, NoError, UnifyKey, UnifyValue};

use crate::bind::SymbolId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
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
    Never,
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
    pub fn new(table: &mut InPlaceUnificationTable<TyId>) -> Self {
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
