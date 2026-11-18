use std::rc::Rc;

use ena::unify::{InPlaceUnificationTable, NoError, UnifyKey, UnifyValue};

use crate::bind::SymbolId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    Unknown,
    Unit,
    Bool,
    Number,
    String,
    Named(SymbolId),
    TypeParam(SymbolId),
    Fn(Rc<[TyId]>, TyId),
    Tuple(Rc<[TyId]>),
    Array(TyId),
}

pub struct CommonTypes {
    pub boolean: TyId,
    pub number: TyId,
    pub unit: TyId,
    pub string: TyId,
}

impl CommonTypes {
    pub fn new(table: &mut InPlaceUnificationTable<TyId>) -> Self {
        Self {
            boolean: table.new_key(Ty::Bool),
            number: table.new_key(Ty::Number),
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
            // We must've checked the compatibility in try_unify. Simply take any of them.
            _ => Ok(value1.clone()),
        }
    }
}
