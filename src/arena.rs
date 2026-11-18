use std::{
    hash::Hash,
    marker::PhantomData,
    ops::{Index, IndexMut},
};

#[derive(Debug)]
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
