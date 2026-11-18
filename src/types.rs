use crate::{
    arena::{Arena, Id},
    bind::SymbolId,
};

pub type TyId = Id<Ty>;

#[derive(Clone, PartialEq, Eq)]
pub enum Ty {
    Named(SymbolId),
    TypeParam(SymbolId),
    Fn(Vec<TyId>, TyId),
    Unit,
    Bool,
    Number,
    String,
    Var,
    Tuple(Vec<TyId>),
}

pub struct CommonTypes {
    pub boolean: TyId,
    pub number: TyId,
    pub unit: TyId,
    pub string: TyId,
}

impl CommonTypes {
    pub fn new(types: &mut Arena<Ty>) -> Self {
        Self {
            boolean: types.alloc(Ty::Bool),
            number: types.alloc(Ty::Number),
            unit: types.alloc(Ty::Unit),
            string: types.alloc(Ty::String),
        }
    }
}
