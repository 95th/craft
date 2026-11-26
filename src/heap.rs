use std::{debug_assert_matches, fmt, mem};

use bitvec::prelude::*;
use rustc_hash::FxHashMap;

use crate::arena::{Arena, Id};

pub struct Heap {
    layouts: Layouts,
    data: Vec<u8>,
}

impl Heap {
    pub fn new(layouts: Layouts) -> Self {
        Self {
            layouts,
            data: Vec::new(),
        }
    }

    pub fn alloc_struct(&mut self, data: &[u64], layout: LayoutId) -> usize {
        let data =
            unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 8) };

        self.alloc(data, layout)
    }

    pub fn alloc_string(&mut self, data: &[u8]) -> usize {
        self.alloc(data, self.layouts.common.string)
    }

    pub fn data(&self, obj: usize) -> &[u8] {
        let header = self.header(obj);
        let i = obj + HEADER_LEN;
        let len = header.len as usize;
        &self.data[i..i + len]
    }

    pub fn get_field(&self, obj: usize, field: usize) -> u64 {
        debug_assert_matches!(
            self.layouts.of(self.header(obj).layout),
            Layout::Struct(fields) if field < fields.len()
        );
        let i = obj + HEADER_LEN + field * 8;
        let bytes = self.data[i..].first_chunk().unwrap();
        u64::from_ne_bytes(*bytes)
    }

    pub fn set_field(&mut self, obj: usize, field: usize, value: u64) {
        debug_assert_matches!(
            self.layouts.of(self.header(obj).layout),
            Layout::Struct(fields) if field < fields.len()
        );
        let i = obj + HEADER_LEN + field * 8;
        let bytes = self.data[i..].first_chunk_mut().unwrap();
        *bytes = value.to_ne_bytes();
    }

    pub fn get_index(&self, obj: usize, field: usize) -> Option<u64> {
        let header = self.header(obj);
        debug_assert_matches!(self.layouts.of(header.layout), Layout::Array(..));
        if field * 8 >= header.len as usize {
            return None;
        }
        let i = obj + HEADER_LEN + field * 8;
        let bytes = self.data[i..].first_chunk().unwrap();
        Some(u64::from_ne_bytes(*bytes))
    }

    #[must_use]
    pub fn set_index(&mut self, obj: usize, field: usize, value: u64) -> bool {
        let header = self.header(obj);
        debug_assert_matches!(self.layouts.of(header.layout), Layout::Array(..));
        if field * 8 >= header.len as usize {
            return false;
        }
        let i = obj + HEADER_LEN + field * 8;
        let bytes = self.data[i..].first_chunk_mut().unwrap();
        *bytes = value.to_ne_bytes();
        true
    }

    fn alloc(&mut self, data: &[u8], layout: LayoutId) -> usize {
        let header = ObjHeader {
            forwarded: false,
            layout,
            len: data.len() as u32,
        };
        let index = self.data.len();
        self.data.reserve(HEADER_LEN + data.len());
        self.data.extend_from_slice(&header.to_data());
        self.data.extend_from_slice(data);
        index
    }

    fn header(&self, obj: usize) -> ObjHeader {
        let data: [u8; HEADER_LEN] = *self.data[obj..].first_chunk().unwrap();
        ObjHeader::from_data(data)
    }
}

const HEADER_LEN: usize = mem::size_of::<ObjHeader>();

#[derive(Clone, Copy)]
#[repr(packed)]
struct ObjHeader {
    forwarded: bool,
    layout: LayoutId,
    len: u32,
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
