use core::fmt;
use std::{
    hash::Hash,
    marker::PhantomData,
    ops::{Index, IndexMut},
};

use crate::{ast::Span, bind::SymbolId};

pub struct Idx<T> {
    idx: u32,
    ph: PhantomData<T>,
}

impl<T> Idx<T> {
    pub const MAX: Self = Self::new(u32::MAX as usize);

    const fn new(index: usize) -> Self {
        Idx {
            idx: index as u32,
            ph: PhantomData,
        }
    }
}

impl<T> fmt::Debug for Idx<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Idx({})", self.idx)
    }
}

impl<T> PartialEq for Idx<T> {
    fn eq(&self, other: &Self) -> bool {
        self.idx == other.idx
    }
}

impl<T> Eq for Idx<T> {}

impl<T> PartialOrd for Idx<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.idx.partial_cmp(&other.idx)
    }
}

impl<T> Ord for Idx<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.idx.cmp(&other.idx)
    }
}

impl<T> Hash for Idx<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.idx.hash(state);
    }
}

impl<T> Clone for Idx<T> {
    fn clone(&self) -> Self {
        Self {
            idx: self.idx,
            ph: PhantomData,
        }
    }
}
impl<T> Copy for Idx<T> {}

pub struct Arena<T> {
    buf: Vec<T>,
}

impl<T> Arena<T> {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn alloc(&mut self, value: T) -> Idx<T> {
        let idx = Idx::new(self.buf.len());
        self.buf.push(value);
        idx
    }
}

impl<T> Index<Idx<T>> for Arena<T> {
    type Output = T;

    fn index(&self, index: Idx<T>) -> &Self::Output {
        &self.buf[index.idx as usize]
    }
}

impl<T> IndexMut<Idx<T>> for Arena<T> {
    fn index_mut(&mut self, index: Idx<T>) -> &mut Self::Output {
        &mut self.buf[index.idx as usize]
    }
}

pub type AstIdx<T> = Idx<(T, Span)>;

pub struct AstArena<T> {
    buf: Vec<T>,
    spans: Vec<Span>,
}

impl<T> AstArena<T> {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            spans: Vec::new(),
        }
    }

    pub fn alloc(&mut self, value: T, span: Span) -> AstIdx<T> {
        let idx = AstIdx::new(self.buf.len());
        self.buf.push(value);
        self.spans.push(span);
        idx
    }

    pub fn span(&self, index: AstIdx<T>) -> Span {
        self.spans[index.idx as usize]
    }

    pub fn new_bindings(&self) -> Bindings<T> {
        Bindings {
            bindings: vec![None; self.len()],
            ph: PhantomData,
        }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn truncate(&mut self, len: usize) {
        self.buf.truncate(len);
        self.spans.truncate(len);
    }
}

impl<T> Index<AstIdx<T>> for AstArena<T> {
    type Output = T;

    fn index(&self, index: AstIdx<T>) -> &Self::Output {
        &self.buf[index.idx as usize]
    }
}

impl<T> IndexMut<AstIdx<T>> for AstArena<T> {
    fn index_mut(&mut self, index: AstIdx<T>) -> &mut Self::Output {
        &mut self.buf[index.idx as usize]
    }
}

pub struct Bindings<T> {
    bindings: Vec<Option<SymbolId>>,
    ph: PhantomData<T>,
}

impl<T> Bindings<T> {
    pub fn bind(&mut self, index: AstIdx<T>, symbol: SymbolId) {
        let prev = self.bindings[index.idx as usize].replace(symbol);
        debug_assert!(prev.is_none());
    }
}

impl<T> Index<AstIdx<T>> for Bindings<T> {
    type Output = Option<SymbolId>;

    fn index(&self, index: AstIdx<T>) -> &Self::Output {
        &self.bindings[index.idx as usize]
    }
}
