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
        let ptr = self.data.as_ptr();
        let len = unsafe { *(ptr.add(obj + 1) as *const [u8; 4]) };
        let len = u32::from_ne_bytes(len) as usize;
        let index = obj + 5;
        &self.data[index..index + len]
    }

    pub fn get_field(&self, obj: usize, field: usize) -> u64 {
        let ptr = self.data.as_ptr();
        debug_assert_eq!(unsafe { *ptr.add(obj) }, ObjKind::Struct as u8);
        let field = unsafe { *(ptr.add(obj + 5).add(field * 8) as *const [u8; 8]) };
        u64::from_ne_bytes(field)
    }

    pub fn set_field(&mut self, obj: usize, field: usize, value: u64) {
        let ptr = self.data.as_mut_ptr();
        debug_assert_eq!(unsafe { *ptr.add(obj) }, ObjKind::Struct as u8);
        unsafe {
            let field = ptr.add(obj + 5).add(field * 8) as *mut [u8; 8];
            *field = value.to_ne_bytes();
        };
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
}
