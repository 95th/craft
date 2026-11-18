use core::fmt;
use std::{
    cell::Cell,
    hash::Hash,
    marker::PhantomData,
    ops::{Index, IndexMut},
};

use crate::{ast::Span, bind::SymbolId};

pub struct Id<T> {
    idx: u32,
    ph: PhantomData<T>,
}

impl<T> Id<T> {
    pub const MAX: Self = Self::new(u32::MAX as usize);

    const fn new(index: usize) -> Self {
        Id {
            idx: index as u32,
            ph: PhantomData,
        }
    }
}

impl<T> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Idx({})", self.idx)
    }
}

impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.idx == other.idx
    }
}

impl<T> Eq for Id<T> {}

impl<T> PartialOrd for Id<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.idx.partial_cmp(&other.idx)
    }
}

impl<T> Ord for Id<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.idx.cmp(&other.idx)
    }
}

impl<T> Hash for Id<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.idx.hash(state);
    }
}

impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        Self {
            idx: self.idx,
            ph: PhantomData,
        }
    }
}
impl<T> Copy for Id<T> {}

pub struct Arena<T> {
    buf: Vec<T>,
}

impl<T> Arena<T> {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn alloc(&mut self, value: T) -> Id<T> {
        let idx = Id::new(self.buf.len());
        self.buf.push(value);
        idx
    }
}

impl<T> Index<Id<T>> for Arena<T> {
    type Output = T;

    fn index(&self, index: Id<T>) -> &Self::Output {
        &self.buf[index.idx as usize]
    }
}

impl<T> IndexMut<Id<T>> for Arena<T> {
    fn index_mut(&mut self, index: Id<T>) -> &mut Self::Output {
        &mut self.buf[index.idx as usize]
    }
}

pub type AstId<T> = Id<(T, Span)>;

pub struct AstArena<T> {
    nodes: Vec<T>,
    spans: Vec<Span>,
    type_bindings: Vec<Cell<SymbolId>>,
    value_bindings: Vec<Cell<SymbolId>>,
}

impl<T> AstArena<T> {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            spans: Vec::new(),
            type_bindings: Vec::new(),
            value_bindings: Vec::new(),
        }
    }

    pub fn alloc(&mut self, value: T, span: Span) -> AstId<T> {
        let idx = AstId::new(self.nodes.len());
        self.nodes.push(value);
        self.spans.push(span);
        self.type_bindings.push(Cell::new(SymbolId::MAX));
        self.value_bindings.push(Cell::new(SymbolId::MAX));
        idx
    }

    pub fn span(&self, index: AstId<T>) -> Span {
        self.spans[index.idx as usize]
    }

    pub fn bind_type(&self, index: AstId<T>, symbol: SymbolId) {
        let prev = self.type_bindings[index.idx as usize].replace(symbol);
        debug_assert_eq!(prev, SymbolId::MAX);
    }

    pub fn type_binding(&self, index: AstId<T>) -> SymbolId {
        self.type_bindings[index.idx as usize].get()
    }

    pub fn bind_value(&self, index: AstId<T>, symbol: SymbolId) {
        let prev = self.value_bindings[index.idx as usize].replace(symbol);
        debug_assert_eq!(prev, SymbolId::MAX);
    }

    pub fn value_binding(&self, index: AstId<T>) -> SymbolId {
        self.value_bindings[index.idx as usize].get()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn truncate(&mut self, len: usize) {
        self.nodes.truncate(len);
        self.spans.truncate(len);
        self.type_bindings.truncate(len);
        self.value_bindings.truncate(len);
    }
}

impl<T> Index<AstId<T>> for AstArena<T> {
    type Output = T;

    fn index(&self, index: AstId<T>) -> &Self::Output {
        &self.nodes[index.idx as usize]
    }
}

impl<T> IndexMut<AstId<T>> for AstArena<T> {
    fn index_mut(&mut self, index: AstId<T>) -> &mut Self::Output {
        &mut self.nodes[index.idx as usize]
    }
}
