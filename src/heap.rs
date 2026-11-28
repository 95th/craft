use std::{
    debug_assert_matches, fmt, mem,
    ops::{Deref, DerefMut},
};

use bitvec::prelude::*;
use rustc_hash::FxHashMap;

use crate::arena::{Arena, Id};

const INITIAL_HEAP_SIZE: usize = 100 * 1024;

pub struct Heap {
    storage: HeapStorage,
    layouts: Layouts,
}

impl Heap {
    pub fn new(layouts: Layouts) -> Self {
        Self {
            storage: HeapStorage::with_capacity(INITIAL_HEAP_SIZE),
            layouts,
        }
    }

    pub fn can_allocate(&self, len: usize) -> bool {
        self.storage.can_allocate(len)
    }

    pub fn alloc_struct(&mut self, data: &[u64], layout: LayoutId) -> usize {
        let data =
            unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 8) };
        self.storage.alloc(data, layout)
    }

    pub fn alloc_string(&mut self, data: &[u8]) -> usize {
        self.storage.alloc(data, self.layouts.common.string)
    }

    pub fn data(&self, obj: usize) -> &[u8] {
        let header = self.storage.header(obj);
        let i = obj + HEADER_LEN;
        let len = header.len_bytes as usize;
        &self.storage[i..i + len]
    }

    pub fn get_field(&self, obj: usize, field: usize) -> u64 {
        debug_assert_matches!(
            self.layouts.of(self.storage.header(obj).layout),
            Layout::Struct(fields) if field < fields.len()
        );
        self.storage.get_field(obj, field)
    }

    pub fn set_field(&mut self, obj: usize, field: usize, value: u64) {
        debug_assert_matches!(
            self.layouts.of(self.storage.header(obj).layout),
            Layout::Struct(fields) if field < fields.len()
        );
        self.storage.set_field(obj, field, value);
    }

    pub fn get_index(&self, obj: usize, index: usize) -> Option<u64> {
        let header = self.storage.header(obj);
        debug_assert_matches!(self.layouts.of(header.layout), Layout::Array(..));
        if index * 8 >= header.len_bytes as usize {
            return None;
        }
        Some(self.storage.get_field(obj, index))
    }

    #[must_use]
    pub fn set_index(&mut self, obj: usize, index: usize, value: u64) -> bool {
        let header = self.storage.header(obj);
        debug_assert_matches!(self.layouts.of(header.layout), Layout::Array(..));
        if index * 8 >= header.len_bytes as usize {
            return false;
        }
        self.storage.set_field(obj, index, value);
        true
    }

    pub fn gc_and_reserve(
        &mut self,
        additional_len: usize,
        roots: Vec<(u64, GcRoot)>,
        mut callback: impl FnMut(u64, GcRoot),
    ) {
        let mut storage = HeapStorage::with_capacity(INITIAL_HEAP_SIZE);
        for (obj, root) in roots {
            let new_obj = self.storage.forward_obj(obj as usize, &mut storage);
            callback(new_obj, root);
        }

        let mut curr = 0;
        while curr < storage.len() {
            let header = storage.header(curr);
            let layout = self.layouts.of(header.layout);
            match layout {
                Layout::String => {}
                Layout::Array(is_ref) => {
                    if *is_ref {
                        for i in 0..header.len_bytes as usize / 8 {
                            let old = storage.get_field(curr, i);
                            let new = self.storage.forward_obj(old as usize, &mut storage);
                            storage.set_field(curr, i, new);
                        }
                    }
                }
                Layout::Struct(ref_map) => {
                    for i in 0..header.len_bytes as usize / 8 {
                        if ref_map[i] {
                            let old = storage.get_field(curr, i);
                            let new = self.storage.forward_obj(old as usize, &mut storage);
                            storage.set_field(curr, i, new);
                        }
                    }
                }
            }
            curr += HEADER_LEN + header.len_bytes as usize;
        }

        let needed = storage.len() + additional_len + HEADER_LEN;
        let new_capacity = if needed * 2 > self.storage.capacity() {
            std::cmp::max(needed * 2, self.storage.capacity() * 2)
        } else if needed * 2 < self.storage.capacity() / 4 {
            std::cmp::max(self.storage.capacity() / 2, INITIAL_HEAP_SIZE)
        } else {
            self.storage.capacity()
        };
        let to_reserve = new_capacity - storage.len();
        storage.reserve(to_reserve);

        if cfg!(feature = "trace") {
            println!(
                "GC: before: {}(capacity: {}), after: {}(capacity: {})",
                self.storage.len(),
                self.storage.capacity(),
                storage.len(),
                storage.capacity()
            );
        }

        self.storage = storage;
    }
}

struct HeapStorage {
    data: Vec<u8>,
}

impl HeapStorage {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity),
        }
    }

    fn can_allocate(&self, len_bytes: usize) -> bool {
        self.len() + len_bytes + HEADER_LEN <= self.capacity()
    }

    fn header(&self, obj: usize) -> ObjHeader {
        let data: [u8; HEADER_LEN] = *self[obj..].first_chunk().unwrap();
        ObjHeader::from_data(data)
    }

    fn forward_obj(&mut self, obj: usize, target: &mut Self) -> u64 {
        let header = self.header(obj);
        if header.forwarded {
            return self.forward_address(obj);
        }

        let new_obj = target.len() as u64;
        target.extend_from_slice(&self[obj..obj + HEADER_LEN + header.len_bytes as usize]);
        new_obj
    }

    fn forward_address(&self, obj: usize) -> u64 {
        debug_assert!(self.header(obj).forwarded);
        let address = self[obj + 1..obj + HEADER_LEN].first_chunk().unwrap();
        u64::from_ne_bytes(*address)
    }

    fn set_field(&mut self, obj: usize, index: usize, value: u64) {
        let i = obj + HEADER_LEN + index * 8;
        let bytes = self[i..].first_chunk_mut().unwrap();
        *bytes = value.to_ne_bytes();
    }

    fn get_field(&self, obj: usize, index: usize) -> u64 {
        let i = obj + HEADER_LEN + index * 8;
        let bytes = self[i..].first_chunk().unwrap();
        u64::from_ne_bytes(*bytes)
    }

    fn alloc(&mut self, data: &[u8], layout: LayoutId) -> usize {
        let header = ObjHeader {
            forwarded: false,
            layout,
            len_bytes: data.len() as u32,
            _padding: 0,
        };
        let index = self.data.len();
        self.data.reserve(HEADER_LEN + data.len());
        self.data.extend_from_slice(&header.to_data());
        self.data.extend_from_slice(data);
        index
    }
}

impl Deref for HeapStorage {
    type Target = Vec<u8>;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl DerefMut for HeapStorage {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

const HEADER_LEN: usize = mem::size_of::<ObjHeader>();

const _: () = {
    assert!(HEADER_LEN == 9);
};

#[derive(Clone, Copy)]
pub enum GcRoot {
    StringPool(usize),
    Stack(usize),
}

#[derive(Clone, Copy)]
#[repr(packed)]
struct ObjHeader {
    forwarded: bool,
    layout: LayoutId,
    len_bytes: u32,
    _padding: u16,
}

impl ObjHeader {
    fn to_data(self) -> [u8; HEADER_LEN] {
        unsafe { mem::transmute(self) }
    }

    fn from_data(data: [u8; HEADER_LEN]) -> Self {
        unsafe { mem::transmute(data) }
    }
}

pub type LayoutId = u16;

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Layout {
    String,
    Array(bool),
    Struct(BitVec),
}

struct CommonLayouts {
    string: LayoutId,
}

pub struct LayoutBuilder {
    arena: Arena<Layout>,
    common: CommonLayouts,
    cache: FxHashMap<Layout, LayoutId>,
}

pub struct Layouts {
    arena: Arena<Layout>,
    common: CommonLayouts,
}

impl LayoutBuilder {
    pub fn new() -> Self {
        let mut arena = Arena::new();
        let common = CommonLayouts {
            string: arena.alloc(Layout::String).to_idx() as LayoutId,
        };
        Self {
            arena,
            common,
            cache: FxHashMap::default(),
        }
    }

    pub fn alloc(&mut self, layout: Layout) -> LayoutId {
        if let Some(&id) = self.cache.get(&layout) {
            return id;
        }
        let id = self.arena.alloc(layout.clone());
        let id = id.to_idx() as u16;
        self.cache.insert(layout, id);
        id
    }

    pub fn build(self) -> Layouts {
        Layouts {
            arena: self.arena,
            common: self.common,
        }
    }

    pub fn of(&self, id: LayoutId) -> &Layout {
        &self.arena[Id::from_idx(id as u32)]
    }
}

impl Layouts {
    fn of(&self, id: LayoutId) -> &Layout {
        &self.arena[Id::from_idx(id as u32)]
    }
}

impl fmt::Debug for Layout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String => write!(f, "String"),
            Self::Array(is_ref) => write!(f, "[{}]", *is_ref as u8),
            Self::Struct(refs) => write!(f, "struct {refs}"),
        }
    }
}

#[test]
fn test_heap() {
    let mut builder = LayoutBuilder::new();
    let struct_layout = builder.alloc(Layout::Struct(bitvec![0; 1]));
    let array_layout = builder.alloc(Layout::Array(false));
    let layouts = builder.build();

    let mut heap = Heap::new(layouts);

    let obj = heap.alloc_string(b"abc");
    assert_eq!(std::str::from_utf8(&heap.data(obj)), Ok("abc"));

    let obj2 = heap.alloc_string(b"als");
    assert_eq!(std::str::from_utf8(&heap.data(obj2)), Ok("als"));

    let obj3 = heap.alloc_struct(&[10], struct_layout);
    heap.set_field(obj3, 0, 20);
    assert_eq!(heap.get_field(obj3, 0), 20);

    let obj4 = heap.alloc_struct(&[10], array_layout);
    assert_eq!(heap.set_index(obj4, 0, 20), true);
    assert_eq!(heap.set_index(obj4, 1, 20), false);
    assert_eq!(heap.get_index(obj4, 0), Some(20));
    assert_eq!(heap.get_index(obj4, 1), None);
}
