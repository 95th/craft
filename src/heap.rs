pub struct Heap {
    data: Vec<u8>,
}

impl Heap {
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    pub fn alloc_struct(&mut self, data: &[u64]) -> usize {
        let data =
            unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 8) };
        self.alloc(ObjKind::Struct, data)
    }

    pub fn alloc_string(&mut self, data: &[u8]) -> usize {
        self.alloc(ObjKind::String, data)
    }

    pub fn data(&self, obj: usize) -> &[u8] {
        let i = obj + 5;
        let len = self.byte_len(obj);
        &self.data[i..i + len]
    }

    pub fn get_field(&self, obj: usize, field: usize) -> u64 {
        debug_assert_eq!(self.data[obj], ObjKind::Struct as u8);
        let i = obj + 5 + field * 8;
        let bytes = self.data[i..].first_chunk().unwrap();
        u64::from_ne_bytes(*bytes)
    }

    pub fn set_field(&mut self, obj: usize, field: usize, value: u64) {
        debug_assert_eq!(self.data[obj], ObjKind::Struct as u8);
        let i = obj + 5 + field * 8;
        let bytes = self.data[i..].first_chunk_mut().unwrap();
        *bytes = value.to_ne_bytes();
    }

    pub fn get_index(&self, obj: usize, field: usize) -> Option<u64> {
        debug_assert_eq!(self.data[obj], ObjKind::Struct as u8);
        if field * 8 >= self.byte_len(obj) {
            return None;
        }
        let i = obj + 5 + field * 8;
        let bytes = self.data[i..].first_chunk().unwrap();
        Some(u64::from_ne_bytes(*bytes))
    }

    #[must_use]
    pub fn set_index(&mut self, obj: usize, field: usize, value: u64) -> bool {
        debug_assert_eq!(self.data[obj], ObjKind::Struct as u8);
        if field * 8 >= self.byte_len(obj) {
            return false;
        }
        let i = obj + 5 + field * 8;
        let bytes = self.data[i..].first_chunk_mut().unwrap();
        *bytes = value.to_ne_bytes();
        true
    }

    fn alloc(&mut self, kind: ObjKind, data: &[u8]) -> usize {
        let offset = self.data.len();
        let len = data.len() as u32;
        self.data.reserve(5 + data.len());
        self.data.push(kind as u8);
        self.data.extend_from_slice(&len.to_ne_bytes());
        self.data.extend_from_slice(data);
        offset
    }

    fn byte_len(&self, obj: usize) -> usize {
        let chunk = self.data[obj + 1..].first_chunk().unwrap();
        u32::from_ne_bytes(*chunk) as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum ObjKind {
    String,
    Struct,
}

#[test]
fn test_heap() {
    let mut heap = Heap::new();

    let obj = heap.alloc(ObjKind::String, b"abc");
    assert_eq!(std::str::from_utf8(&heap.data(obj)), Ok("abc"));

    let obj2 = heap.alloc(ObjKind::String, b"als");
    assert_eq!(std::str::from_utf8(&heap.data(obj2)), Ok("als"));

    let obj3 = heap.alloc(ObjKind::Struct, &10_u64.to_ne_bytes());
    heap.set_field(obj3, 0, 20);
    assert_eq!(heap.get_field(obj3, 0), 20);

    let obj4 = heap.alloc(ObjKind::Struct, &10_u64.to_ne_bytes());
    assert_eq!(heap.set_index(obj4, 0, 20), true);
    assert_eq!(heap.set_index(obj4, 1, 20), false);
    assert_eq!(heap.get_index(obj4, 0), Some(20));
    assert_eq!(heap.get_index(obj4, 1), None);
}
