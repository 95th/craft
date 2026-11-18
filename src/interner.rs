use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StrId(u32);

pub struct StrInterner {
    map: HashMap<&'static str, StrId>,
    vec: Vec<&'static str>,
    full: Vec<String>,
    buf: String,
}

impl StrInterner {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            vec: Vec::new(),
            full: Vec::new(),
            buf: String::with_capacity(1024),
        }
    }

    pub fn intern(&mut self, s: &str) -> StrId {
        if let Some(name) = self.map.get(s) {
            return *name;
        }
        let s = unsafe { self.alloc(s) };
        let id = StrId(self.vec.len() as u32);
        self.map.insert(s, id);
        self.vec.push(s);
        id
    }

    pub fn lookup(&self, s: StrId) -> &str {
        self.vec[s.0 as usize]
    }

    unsafe fn alloc(&mut self, name: &str) -> &'static str {
        let cap = self.buf.capacity();
        if cap < self.buf.len() + name.len() {
            let new_cap = (cap.max(name.len()) + 1).next_power_of_two();
            let new_buf = String::with_capacity(new_cap);
            let old_buf = std::mem::replace(&mut self.buf, new_buf);
            self.full.push(old_buf);
        }

        let i = self.buf.len();
        self.buf.push_str(name);
        unsafe { std::mem::transmute(&self.buf[i..]) }
    }
}
