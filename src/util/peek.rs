use std::{
    cell::RefCell,
    ops::Deref,
};

/// Wrapper over a flexbuffer reader, returned by IRecord::peek(..)
#[derive(Clone)]
pub struct Peek<'peek>(flexbuffers::Reader<&'peek [u8]>);

impl<'peek> From<flexbuffers::Reader<&'peek [u8]>> for Peek<'peek> {
    fn from(value: flexbuffers::Reader<&'peek [u8]>) -> Self {
        Self(value)
    }
}

impl<'peek> Deref for Peek<'peek> {
    type Target = flexbuffers::Reader<&'peek [u8]>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'peek> AsRef<Peek<'peek>> for Peek<'peek> {
    fn as_ref(&self) -> &Peek<'peek> {
        self
    }
}

/// Enables std::op::Index
#[derive(Clone)]
pub struct PeekRef<'peek>(RefCell<Option<Peek<'peek>>>);

impl<'p> std::ops::Index<&str> for PeekRef<'p> {
    type Output = PeekRef<'p>;

    fn index(&self, index: &str) -> &Self::Output {
        self.0.replace(self.0.take().at(index));
        self
    }
}

/// Extensions methods for working with a PeekRef container
pub trait PeekRefExtensions<'peek> {
    /// Returns a bool if the current peek context is a bool
    fn bool(&self) -> Option<bool>;

    /// Returns a str if the current peek context is a str
    fn str(&self) -> Option<&'peek str>;

    /// Returns a u64 if the current peek context is an unsigned integer
    fn u64(&self) -> Option<u64>;

    /// Returns an i64 if the current item is a signed or unsigned integer
    fn int(&self) -> Option<i64>;

    /// Returns a **filtered** iterator of u64 values
    ///
    /// If the current value is not a vector, returns None
    fn iter_u64(&self) -> Option<impl Iterator<Item = u64>>;

    /// Returns an iterator of peek's from the current item
    ///
    /// If the current value is not a vector, returns None
    fn iter(&self) -> Option<impl Iterator<Item = Peek<'peek>>>;

    /// Returns an iterator of the current item of key/value pairs
    /// 
    /// If the current item is not a map, returns None
    fn iter_kv(&self) -> Option<impl Iterator<Item = (&'peek str, Peek<'peek>)>>;
}

/// Extension methods for working with a Peek container
pub trait PeekExtensions<'peek> {
    /// Converts this `Peek` into a shared reference-like wrapper that enables `Index`-based traversal.
    ///
    /// This method returns a value that implements [`std::ops::Index`] with reference semantics,
    /// allowing chained syntax like:
    ///
    /// ```rs no_run
    /// let val = &rec.peek().in_ref()["some"]["path"]["to"]["value"];
    /// ```
    ///
    /// Under the hood, this uses interior mutability (`RefCell`) to update the internal cursor
    /// as you access fields. There are no pointer tricks — just a mutable Peek advancing through the structure.
    ///
    /// ⚠️ **Caveat**: This has stateful behavior. Given:
    ///
    /// ```rs no_run
    /// let some_path = &rec.peek().in_ref()["some"]["path"];
    /// let val1 = some_path["hello"];
    /// let val2 = some_path["world"];
    /// ```
    ///
    /// The second lookup (`["world"]`) is **not** relative to `"some.path"`, but to
    /// `"some.path.hello"` — because the internal reader advanced during the first call.
    ///
    /// If you need to reuse a subpath, call `clone` before you use `[]`
    ///
    /// ```rs no_run
    /// let base = rec.peek().in_ref()["some"]["path"];
    /// let a = base.clone()["hello"];
    /// let b = base.["world"]; // Works as expected
    /// ```
    ///
    /// This design favors ergonomics over strict immutability — and lookups always return `Option`, never panic.
    ///
    /// If you want strict immutability, and a bit more of a safety net, use `at()` and `at_path()` instead.
    fn in_ref(
        self,
    ) -> impl std::ops::Index<&'peek str, Output = PeekRef<'peek>> + PeekRefExtensions<'peek>;

    /// Peek at the reader under a specific key, if the current peek context is a map
    fn at(self, key: &str) -> Option<Peek<'peek>>;

    /// Peek at the reader, that exists at the end of a list of keys
    ///
    /// Returns None if each key before the last is not a map
    fn at_path(self, keys: impl AsRef<[&'peek str]>) -> Option<Peek<'peek>>;

    /// Returns a bool if the current peek context is a bool
    fn bool(self) -> Option<bool>;

    /// Returns a str if the current peek context is a str
    fn str(self) -> Option<&'peek str>;

    /// Returns a u64 if the current peek context is a u64
    fn u64(self) -> Option<u64>;

    /// Returns an i64 if the current peek context is a i64 or u64
    fn int(self) -> Option<i64>;

    /// Returns a **filtered** iterator of u64 values
    ///
    /// If the current value is not a vector, returns None
    fn iter_u64(self) -> Option<impl Iterator<Item = u64>>;

    /// Returns an iterator of peek's from the current item
    ///
    /// If the current value is not a vector, returns None
    fn iter(self) -> Option<impl Iterator<Item = Peek<'peek>>>;

    /// Returns an iterator over kv pairs from the peek's current position
    /// 
    /// If the current value is not a map, returns None
    fn iter_kv(self) -> Option<impl Iterator<Item = (&'peek str, Peek<'peek>)>>;
}
impl<'peek> PeekExtensions<'peek> for &'peek Peek<'peek> {
    #[inline]
    fn at(self, key: &str) -> Option<Peek<'peek>> {
        self.get_map().ok().map(|p| Peek(p.idx(key)))
    }

    #[inline]
    fn at_path(self, keys: impl AsRef<[&'peek str]>) -> Option<Peek<'peek>> {
        keys.as_ref()
            .iter()
            .fold(Some(self.clone()), |p, s| p.at(s))
    }

    #[inline]
    fn bool(self) -> Option<bool> {
        self.0.get_bool().ok()
    }

    #[inline]
    fn str(self) -> Option<&'peek str> {
        self.0.get_str().ok()
    }

    #[inline]
    fn u64(self) -> Option<u64> {
        self.0.get_u64().ok()
    }

    #[inline]
    fn iter_u64(self) -> Option<impl Iterator<Item = u64>> {
        self.iter().map(|i| i.filter_map(|r| r.u64()))
    }

    #[inline]
    fn iter(self) -> Option<impl Iterator<Item = Peek<'peek>>> {
        self.get_vector().ok().map(|v| v.iter().map(|r| Peek(r)))
    }

    #[inline]
    fn in_ref(
        self,
    ) -> impl std::ops::Index<&'peek str, Output = PeekRef<'peek>> + PeekRefExtensions<'peek> {
        PeekRef(RefCell::new(Some(self.clone())))
    }

    #[inline]
    fn iter_kv(self) -> Option<impl Iterator<Item = (&'peek str, Peek<'peek>)>> {
        self.get_map().ok().map(|m| {
            let _m2 = m.clone();
            m.iter_keys()
                .map(move |l| (l, Peek(_m2.idx(l))))
        })
    }
    
    #[inline]
    fn int(self) -> Option<i64> {
        self.clone().0.get_u64().ok().map(|u| u as i64).or(self.0.get_i64().ok())
    }
}

impl<'peek> PeekExtensions<'peek> for Peek<'peek> {
    #[inline]
    fn at(self, key: &str) -> Option<Peek<'peek>> {
        self.get_map().ok().map(|p| Peek(p.idx(key)))
    }

    #[inline]
    fn at_path(self, keys: impl AsRef<[&'peek str]>) -> Option<Peek<'peek>> {
        keys.as_ref().iter().fold(Some(self), |p, s| p.at(s))
    }

    #[inline]
    fn bool(self) -> Option<bool> {
        self.0.get_bool().ok()
    }

    #[inline]
    fn str(self) -> Option<&'peek str> {
        self.0.get_str().ok()
    }

    #[inline]
    fn u64(self) -> Option<u64> {
        self.0.get_u64().ok()
    }

    #[inline]
    fn int(self) -> Option<i64> {
        self.clone().0.get_u64().ok().map(|u| u as i64).or(self.0.get_i64().ok())
    }

    #[inline]
    fn iter_u64(self) -> Option<impl Iterator<Item = u64>> {
        self.iter().map(|i| i.filter_map(|r| r.u64()))
    }

    #[inline]
    fn iter(self) -> Option<impl Iterator<Item = Peek<'peek>>> {
        self.get_vector().ok().map(|v| v.iter().map(|r| Peek(r)))
    }

    #[inline]
    fn in_ref(
        self,
    ) -> impl std::ops::Index<&'peek str, Output = PeekRef<'peek>> + PeekRefExtensions<'peek> {
        PeekRef(RefCell::new(Some(self)))
    }
    
    #[inline]
    fn iter_kv(self) -> Option<impl Iterator<Item = (&'peek str, Peek<'peek>)>> {
        self.get_map().ok().map(|m| {
            let _m = m.clone();
            let _m2 = m.clone();
            m.iter_keys()
                .map(move |l| (l, Peek(_m2.idx(l))))
        })
    }
}

impl<'peek> PeekExtensions<'peek> for Option<Peek<'peek>> {
    #[inline]
    fn at(self, key: &str) -> Option<Peek<'peek>> {
        self.and_then(|r| r.at(key))
    }

    #[inline]
    fn at_path(self, keys: impl AsRef<[&'peek str]>) -> Option<Peek<'peek>> {
        self.and_then(|p| p.at_path(keys))
    }

    #[inline]
    fn bool(self) -> Option<bool> {
        self.and_then(|r| r.bool())
    }

    #[inline]
    fn str(self) -> Option<&'peek str> {
        self.and_then(|r| r.str())
    }

    #[inline]
    fn u64(self) -> Option<u64> {
        self.and_then(|r| r.u64())
    }

    #[inline]
    fn iter_u64(self) -> Option<impl Iterator<Item = u64>> {
        self.and_then(|r| r.iter_u64())
    }

    #[inline]
    fn iter(self) -> Option<impl Iterator<Item = Peek<'peek>>> {
        self.and_then(|r| r.iter())
    }

    #[inline]
    fn in_ref(
        self,
    ) -> impl std::ops::Index<&'peek str, Output = PeekRef<'peek>> + PeekRefExtensions<'peek> {
        PeekRef(RefCell::new(self))
    }
    
    #[inline]
    fn iter_kv(self) -> Option<impl Iterator<Item = (&'peek str, Peek<'peek>)>> {
        self.and_then(|r| r.iter_kv())
    }
    
    #[inline]
    fn int(self) -> Option<i64> {
        self.and_then(|r| r.int())
    }
}

impl<'peek> PeekExtensions<'peek> for Option<&'peek Peek<'peek>> {
    #[inline]
    fn bool(self) -> Option<bool> {
        self.cloned().and_then(|r| r.bool())
    }

    #[inline]
    fn str(self) -> Option<&'peek str> {
        self.cloned().and_then(|r| r.str())
    }

    #[inline]
    fn u64(self) -> Option<u64> {
        self.cloned().and_then(|r| r.u64())
    }

    #[inline]
    fn at(self, key: &str) -> Option<Peek<'peek>> {
        self.cloned().and_then(|r| r.at(key))
    }

    #[inline]
    fn iter_u64(self) -> Option<impl Iterator<Item = u64>> {
        self.cloned().and_then(|r| r.iter_u64())
    }

    #[inline]
    fn iter(self) -> Option<impl Iterator<Item = Peek<'peek>>> {
        self.cloned().and_then(|r| r.iter())
    }

    #[inline]
    fn at_path(self, keys: impl AsRef<[&'peek str]>) -> Option<Peek<'peek>> {
        self.cloned().and_then(|r| r.at_path(keys))
    }

    #[inline]
    fn in_ref(
        self,
    ) -> impl std::ops::Index<&'peek str, Output = PeekRef<'peek>> + PeekRefExtensions<'peek> {
        PeekRef(RefCell::new(self.cloned()))
    }
    
    #[inline]
    fn iter_kv(self) -> Option<impl Iterator<Item = (&'peek str, Peek<'peek>)>> {
        self.cloned().and_then(|r| r.iter_kv())
    }
    
    #[inline]
    fn int(self) -> Option<i64> {
        self.and_then(|r| r.int())
    }
}

impl<'p> PeekRefExtensions<'p> for PeekRef<'p> {
    #[inline]
    fn bool(&self) -> Option<bool> {
        self.0.borrow().deref().clone().bool()
    }

    #[inline]
    fn str(&self) -> Option<&'p str> {
        self.0.borrow().deref().clone().str()
    }

    #[inline]
    fn u64(&self) -> Option<u64> {
        self.0.borrow().deref().clone().u64()
    }

    #[inline]
    fn iter_u64(&self) -> Option<impl Iterator<Item = u64>> {
        self.0.borrow().clone().iter_u64()
    }

    #[inline]
    fn iter(&self) -> Option<impl Iterator<Item = Peek<'p>>> {
        self.0.borrow().clone().iter()
    }
    
    #[inline]
    fn iter_kv(&self) -> Option<impl Iterator<Item = (&'p str, Peek<'p>)>> {
        self.0.borrow().clone().iter_kv()
    }
    
    #[inline]
    fn int(&self) -> Option<i64> {
        self.0.borrow().clone().int()
    }
}

#[derive(Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct PathNode {
    current: &'static str,
    link: usize
}

#[derive(Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct XorPath {
    head: Option<std::ptr::NonNull<PathNode>>
}

impl XorPath {
    fn append(&self, seg: &str) -> XorPath {
        let seg = intern_str(seg);
        let prev = self.head;
        let new: &'static PathNode = intern_path_node(PathNode {
            current: seg,
            link: prev.map_or(0, |p| p.as_ptr() as usize),
        });
    
        if let Some(prev_ptr) = prev {
            let prev_node = unsafe { prev_ptr.as_ref() };
            let back_link = prev_node.link ^ (new as *const _ as usize);
            let prev_mut = unsafe { (prev_ptr.as_ptr() as *mut PathNode).as_mut().unwrap() };
            prev_mut.link = back_link;
        }

        let new: *mut PathNode = new as *const _ as *mut _;
    
        XorPath {
            head: std::ptr::NonNull::new(new),
        }
    }

    fn resolve(&self) -> Vec<&'static str> {
        let mut path = Vec::new();
        let mut curr = self.head;
        let mut prev = None;
    
        while let Some(node_ptr) = curr {
            let node = unsafe { node_ptr.as_ref() };
            path.push(node.current);
    
            let next_addr = node.link ^ prev.map_or(0, |p: std::ptr::NonNull<PathNode>| p.as_ptr() as usize);
            prev = curr;
            curr = std::ptr::NonNull::new(next_addr as *mut _);
        }
    
        path.reverse(); // because we walked backward
        path
    }
}

static PATH_NODE_INTERNER: std::sync::OnceLock<NodeInterner> = std::sync::OnceLock::new();

fn intern_path_node(str: PathNode) -> &'static PathNode {
    let interner = PATH_NODE_INTERNER.get_or_init(NodeInterner::default);
    interner.intern(str)
}

#[derive(Default)]
struct XorPathInterner {
    strings: dashmap::DashSet<&'static XorPath>,
    check_list: dashmap::DashMap<u64, &'static XorPath>,
}

impl XorPathInterner {
    pub fn intern(&self, str: XorPath) -> &'static XorPath {
        use std::hash::Hash;
        use std::hash::Hasher;
        if let Some(v) = self.strings.get(&str) {
            &(*v)
        } else {
            let mut hasher = ahash::AHasher::default();
            str.hash(&mut hasher);

            let s = self.check_list.entry(hasher.finish()).or_insert_with(|| {
                let interned: &'static XorPath = Box::leak(Box::new(str));
                interned
            });

            self.strings.insert(&s);

            &s
        }
    }
}


#[derive(Default)]
struct NodeInterner {
    strings: dashmap::DashSet<&'static PathNode>,
    check_list: dashmap::DashMap<u64, &'static PathNode>,
}

impl NodeInterner {
    pub fn intern(&self, str: PathNode) -> &'static PathNode {
        use std::hash::Hash;
        use std::hash::Hasher;
        if let Some(v) = self.strings.get(&str) {
            &(*v)
        } else {
            let mut hasher = ahash::AHasher::default();
            str.hash(&mut hasher);

            let s = self.check_list.entry(hasher.finish()).or_insert_with(|| {
                let interned: &'static PathNode = Box::leak(Box::new(str));
                interned
            });

            self.strings.insert(&s);

            &s
        }
    }
}

#[test]
fn test_xor_path() {
    let xor_path = XorPath::default();

    let path = xor_path.append("other").append("values");
    eprintln!("{:?}", path.resolve());
}


static INTERNER: std::sync::OnceLock<Interner> = std::sync::OnceLock::new();

pub(crate) fn intern_str(str: &str) -> &'static str {
    let interner = INTERNER.get_or_init(Interner::default);
    interner.intern(str)
}

#[derive(Default)]
struct Interner {
    strings: dashmap::DashSet<&'static str>,
    check_list: dashmap::DashMap<u64, &'static str>,
}

impl Interner {
    pub fn intern(&self, str: &str) -> &'static str {
        use std::hash::Hash;
        use std::hash::Hasher;
        if let Some(v) = self.strings.get(str) {
            &(*v)
        } else {
            let mut hasher = ahash::AHasher::default();
            str.hash(&mut hasher);

            let s = self.check_list.entry(hasher.finish()).or_insert_with(|| {
                let interned: &'static str = Box::leak(str.to_string().into_boxed_str());
                &interned
            });

            let s = &(*s);
            self.strings.insert(s);
            s
        }
    }
}

#[test]
fn test_interner() {
    let interner = Interner::default();
    let hello_world = interner.intern("hello world");
    assert_eq!("hello world", hello_world);
}