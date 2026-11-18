use core::fmt;

use crate::{
    arena::{Arena, Idx},
    bind::{Symbol, SymbolId},
    interner::StrInterner,
};

pub type TypeId = Idx<Type>;

#[derive(Clone, PartialEq, Eq)]
pub enum Type {
    Named(SymbolId),
    Fn(Vec<TypeId>, TypeId),
    Unit,
    Bool,
    Number,
    String,
    Var,
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

impl TypeId {
    pub fn display<'a>(
        self,
        types: &'a Arena<Type>,
        symbols: &'a Arena<Symbol>,
        interner: &'a StrInterner,
    ) -> impl fmt::Display {
        DisplayType {
            ty: self,
            types,
            symbols,
            interner,
        }
    }
}

struct DisplayType<'a> {
    ty: TypeId,
    types: &'a Arena<Type>,
    symbols: &'a Arena<Symbol>,
    interner: &'a StrInterner,
}

impl fmt::Display for DisplayType<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            ty,
            types,
            symbols,
            interner,
        } = *self;
        match types[ty] {
            Type::Named(symbol) => write!(f, "{}", interner.lookup(symbols[symbol].name)),
            Type::Fn(ref params, ret) => {
                write!(f, "fn(")?;
                let mut sep = "";
                for p in params {
                    write!(f, "{sep}{}", p.display(types, symbols, interner))?;
                    sep = ", ";
                }
                write!(f, ") -> {}", ret.display(types, symbols, interner))
            }
            Type::Unit => write!(f, "()"),
            Type::Bool => write!(f, "bool"),
            Type::Number => write!(f, "number"),
            Type::String => write!(f, "string"),
            Type::Var => write!(f, "<cannot infer>"),
        }
    }
}
