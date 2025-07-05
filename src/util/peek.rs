use std::{cell::RefCell, ops::Deref};

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

    /// Returns a u64 if the current peek context is a u64
    fn u64(&self) -> Option<u64>;

    /// Returns a **filtered** iterator of u64 values
    ///
    /// If the current value is not a vector, returns None
    fn iter_u64(&self) -> Option<impl Iterator<Item = u64>>;

    /// Returns an iterator of peek's from the current item
    ///
    /// If the current value is not a vector, returns None
    fn iter(&self) -> Option<impl Iterator<Item = Peek<'peek>>>;
}

/// Extension methods for working with a Peek container
pub trait PeekExtensions<'peek> {
    /// Converts this `Peek` into a shared reference-like wrapper that enables `Index`-based traversal.
    ///
    /// This method returns a value that implements [`std::ops::Index`] with reference semantics,
    /// allowing chained syntax like:
    ///
    /// ```
    /// let val = &rec.peek().in_ref()["some"]["path"]["to"]["value"];
    /// ```
    ///
    /// Under the hood, this uses interior mutability (`RefCell`) to update the internal cursor
    /// as you access fields. There are no pointer tricks — just a mutable Peek advancing through the structure.
    ///
    /// ⚠️ **Caveat**: This has stateful behavior. Given:
    ///
    /// ```
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
    /// ```
    /// let base = rec.peek().in_ref()["some"]["path"];
    /// let a = base.clone()["hello"];
    /// let b = base.["world"]; // Works as expected
    /// ```
    ///
    /// This design favors ergonomics over strict immutability — and lookups always return `Option`, never panic.
    /// 
    /// If you want strict immutability, and a bit more of a safety net, use `at()` and `at_path()` instead.
    fn in_ref(self) -> impl std::ops::Index<&'peek str, Output = PeekRef<'peek>> + PeekRefExtensions<'peek>;

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

    /// Returns a **filtered** iterator of u64 values
    ///
    /// If the current value is not a vector, returns None
    fn iter_u64(self) -> Option<impl Iterator<Item = u64>>;

    /// Returns an iterator of peek's from the current item
    ///
    /// If the current value is not a vector, returns None
    fn iter(self) -> Option<impl Iterator<Item = Peek<'peek>>>;
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
    fn in_ref(self) -> impl std::ops::Index<&'peek str, Output = PeekRef<'peek>> + PeekRefExtensions<'peek> {
        PeekRef(RefCell::new(Some(self.clone())))
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
    fn iter_u64(self) -> Option<impl Iterator<Item = u64>> {
        self.iter().map(|i| i.filter_map(|r| r.u64()))
    }

    #[inline]
    fn iter(self) -> Option<impl Iterator<Item = Peek<'peek>>> {
        self.get_vector().ok().map(|v| v.iter().map(|r| Peek(r)))
    }

    #[inline]
    fn in_ref(self) -> impl std::ops::Index<&'peek str, Output = PeekRef<'peek>> + PeekRefExtensions<'peek> {
        PeekRef(RefCell::new(Some(self)))
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
    fn in_ref(self) -> impl std::ops::Index<&'peek str, Output = PeekRef<'peek>> + PeekRefExtensions<'peek> {
        PeekRef(RefCell::new(self))
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
    fn in_ref(self) -> impl std::ops::Index<&'peek str, Output = PeekRef<'peek>> + PeekRefExtensions<'peek> {
        PeekRef(RefCell::new(self.cloned()))
    }
}

impl<'p> PeekRefExtensions<'p> for PeekRef<'p> {
    fn bool(&self) -> Option<bool> {
        self.0.borrow().deref().clone().bool()
    }

    fn str(&self) -> Option<&'p str> {
        self.0.borrow().deref().clone().str()
    }

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
}
