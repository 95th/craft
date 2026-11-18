pub struct Heap {
    data: Vec<u8>,
}

impl Heap {
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    pub fn alloc(&mut self, kind: ObjKind, data: &[u8]) -> usize {
        let align = std::mem::align_of::<Obj<[u8; 0]>>();
        let offset = self.data.len().next_multiple_of(align);
        self.data
            .reserve((offset - self.data.len()) + data.len() + 8);
        unsafe {
            let ptr = self.data.as_ptr().add(offset) as *mut Obj<[u8; 0]>;
            std::ptr::write(&raw mut (*ptr).kind, kind);
            std::ptr::write(&raw mut (*ptr).len, data.len() as u32);
            let src = data.as_ptr();
            let dst = &raw mut (*ptr).data as *mut u8;
            std::ptr::copy_nonoverlapping(src, dst, data.len());
            self.data
                .set_len(dst as usize - self.data.as_ptr() as usize + data.len());
        }
        offset
    }

    pub fn data(&self, index: usize) -> &[u8] {
        &self.get(index).data
    }

    pub fn data_mut(&mut self, index: usize) -> &mut [u8] {
        &mut self.get_mut(index).data
    }

    pub fn get_field(&self, obj: usize, index: usize) -> u64 {
        let data = self.data(obj);
        let bytes = unsafe { *(data[index * 8..].as_ptr() as *const [u8; 8]) };
        u64::from_ne_bytes(bytes)
    }

    pub fn set_field(&mut self, obj: usize, index: usize, value: u64) {
        let data = self.data_mut(obj as usize);
        let bytes = data[index * 8..].as_mut_ptr();
        unsafe {
            std::ptr::copy_nonoverlapping(value.to_ne_bytes().as_ptr(), bytes, 8);
        }
    }

    fn get(&self, index: usize) -> &Obj<[u8]> {
        let thin = self.data[index..].as_ptr() as *const Obj<[u8; 0]>;
        let len = unsafe { (*thin).len as usize };
        let ptr = std::ptr::slice_from_raw_parts(thin as *const u8, len);
        unsafe { &*(ptr as *const Obj<[u8]>) }
    }

    fn get_mut(&mut self, index: usize) -> &mut Obj<[u8]> {
        let thin = self.data[index..].as_mut_ptr() as *mut Obj<[u8; 0]>;
        let len = unsafe { (*thin).len as usize };
        let ptr = std::ptr::slice_from_raw_parts(thin as *mut u8, len);
        unsafe { &mut *(ptr as *mut Obj<[u8]>) }
    }
}

struct Obj<T: ?Sized> {
    kind: ObjKind,
    len: u32,
    data: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjKind {
    String,
    Struct,
}

#[test]
fn test_heap() {
    let mut heap = Heap::new();

    let obj = heap.alloc(ObjKind::String, b"abc");
    assert_eq!(std::str::from_utf8(&heap.get(obj).data), Ok("abc"));

    let obj2 = heap.alloc(ObjKind::String, b"als");
    assert_eq!(std::str::from_utf8(&heap.get(obj2).data), Ok("als"));

    heap.data_mut(obj2)[0] = b'x';
    assert_eq!(std::str::from_utf8(&heap.get(obj2).data), Ok("xls"));
}
