use crate::{
    arena::{Arena, Idx},
    bind::SymbolId,
};

pub type TypeId = Idx<Type>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Named(SymbolId),
    Fn(Vec<TypeId>, TypeId),
    Unit,
    Bool,
    Number,
    String,
    Var,
    Error,
}

pub struct CommonTypes {
    pub boolean: TypeId,
    pub number: TypeId,
    pub unit: TypeId,
    pub string: TypeId,
}

impl CommonTypes {
    pub fn new(types: &mut Arena<Type>) -> Self {
        Self {
            boolean: types.alloc(Type::Bool),
            number: types.alloc(Type::Number),
            unit: types.alloc(Type::Unit),
            string: types.alloc(Type::String),
        }
    }
}
