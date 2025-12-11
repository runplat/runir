use crate::util::{Graph, Node};
use serde::Deserialize;
use std::{cell::RefCell, fmt::Display, ops::Deref};

/// Wrapper over a flexbuffer reader, returned by IRecord::peek(..)
#[derive(Clone, Debug)]
pub struct Peek<'peek>(flexbuffers::Reader<&'peek [u8]>);

impl<'peek> From<flexbuffers::Reader<&'peek [u8]>> for Peek<'peek> {
    fn from(value: flexbuffers::Reader<&'peek [u8]>) -> Self {
        Self(value)
    }
}

impl<'peek> Display for Peek<'peek> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.deref())
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
pub trait PeekExtensions<'peek> : Sized {
    /// Returns the [`Peek<'peek>`] at the current position, if a value exists.
    ///
    /// This method is useful as an "escape hatch" when chaining peek operations,
    /// allowing you to exit the fluent API and inspect or manipulate the raw [`Peek`] directly.
    ///
    /// # Example
    /// ```rs no_run
    /// if let Some(peek) = value.field(&path).val() {
    ///     // Since Peek implements Deref<Target = flexbuffers::Reader>,
    ///     // this allows it to use Reader's Display implementation when dereferenced
    ///     println!("{}", peek.deref());
    ///     
    ///     // Similarly, all `as_*`, `get_*`, etc. methods from Reader are available as well
    ///     println!("{}", peek.as_str());
    /// }
    /// ```
    /// Returns `None` if the current position does not point to a value (e.g., an invalid path).
    fn val(self) -> Option<Peek<'peek>>;

    /// Returns an iterator of peek's from the current item
    ///
    /// If the current value is not a vector, returns None
    fn as_iter(self) -> Option<impl Iterator<Item = Peek<'peek>>>;

    /// Returns an iterator over kv pairs from the peek's current position
    ///
    /// If the current value is not a map, returns None
    fn as_iter_kv(self) -> Option<impl Iterator<Item = (&'peek str, Peek<'peek>)>>;

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
    ///
    /// If you want static path declaration, check out [`PeekPath`]
    fn in_ref(
        self,
    ) -> impl std::ops::Index<&'peek str, Output = PeekRef<'peek>> + PeekRefExtensions<'peek>;

    /// Access a nested reader by key, if the current `Peek` is a map.
    ///
    /// Returns `Some(Peek)` if the key exists in the current map;  
    /// otherwise returns `None`.
    ///
    /// # Example
    /// ```rs no_run
    /// let peek = record.peek();
    /// let value = peek.at("key")?.str();
    /// ```
    #[inline]
    fn at(self, key: &str) -> Option<Peek<'peek>> {
        self.val().at(key)
    }

    /// Traverses a list of keys and returns a `Peek` at the final value, if found.
    ///
    /// This is equivalent to chaining multiple `.at(..)` calls.  
    /// Returns `None` if any intermediate key is missing or not a map.
    ///
    /// # Example
    /// ```rs no_run
    /// let value = peek.at_path(["a", "b", "c"])?.int();
    /// ```
    ///
    /// # Notes
    /// You can use this when working with programmatic or dynamic paths:
    /// ```rs no_run
    /// let keys = vec!["config", "metadata", "version"];
    /// if let Some(p) = peek.at_path(&keys).str() {
    ///     assert_eq!(p, "1.0.0");
    /// }
    /// ```
    ///
    /// For dot-separated strings, see [`PeekExtensions::at_dot`].
    #[inline]
    fn at_path(self, keys: impl AsRef<[&'peek str]>) -> Option<Peek<'peek>> {
        self.val().at_path(keys)
    }

    /// Traverses a dot-separated path and returns a `Peek` at the final key, if it exists.
    ///
    /// Returns `None` if any intermediate key is not a map.
    ///
    /// # Example
    /// ```rs no_run
    /// let peek = record.peek();
    /// assert_eq!(peek.at_dot("a.b.c")?.int(), Some(42));
    /// ```
    ///
    /// # Behavior
    /// - Fails early if any key in the path is missing or not a map
    /// - Returns `None` if the final key does not exist
    ///
    /// Equivalent to chaining multiple `.at("key")` calls:
    /// ```rs no_run
    /// peek.at("a").at("b").at("c")
    /// ```
    #[inline]
    fn at_dot(self, path: &'peek str) -> Option<Peek<'peek>> {
        self.val().at_dot(path)
    }

    /// Access current position as a vector and return the element at idx
    #[inline]
    fn at_idx(self, idx: usize) -> Option<Peek<'peek>> {
        self.val().at_idx(idx)
    }

    /// Peeks at the current position
    #[inline]
    fn peek(self) -> Option<Peek<'peek>>
    {
        self.blob()
            .and_then(|b| flexbuffers::Reader::get_root(b).ok())
            .map(Peek)
    }

    /// Returns a bool if the current peek context is a bool
    #[inline]
    fn bool(self) -> Option<bool> {
        self.val().bool()
    }

    /// Returns a str if the current peek context is a str
    #[inline]
    fn str(self) -> Option<&'peek str> {
        self.val().str()
    }

    /// Returns a u64 if the current peek context is a u64
    #[inline]
    fn u64(self) -> Option<u64> {
        self.val().u64()
    }

    /// Returns an i64 if the current peek context is a i64 or u64
    #[inline]
    fn int(self) -> Option<i64> {
        self.val().int()
    }

    /// Returns a blob slice if the current peek context is a blob slice
    #[inline]
    fn blob(self) -> Option<&'peek [u8]> {
        self.val().blob()
    }

    /// Peeks at many keys at once
    #[inline]
    fn at_many(self, keys: &[&str]) -> Vec<Option<Peek<'peek>>>
    where
        Self: Sized + Clone,
    {
        keys.iter().fold(vec![], |mut vec, k| {
            let current = self.clone(); // This will be a shallow copy since it only needs to clone a pointer
            vec.push(current.at(k));
            vec
        })
    }

    /// Peeks at many "dot" paths at once
    #[inline]
    fn at_dot_many(self, paths: &[&'peek str]) -> Vec<Option<Peek<'peek>>>
    where
        Self: Sized + Clone,
    {
        paths.iter().fold(vec![], |mut vec, p| {
            let current = self.clone(); // This will be a shallow copy since it only needs to clone a pointer
            vec.push(current.at_dot(p));
            vec
        })
    }

    /// Peeks at many paths at once
    #[inline]
    fn at_path_many(self, paths: &[&[&'peek str]]) -> Vec<Option<Peek<'peek>>>
    where
        Self: Sized + Clone,
    {
        paths.iter().fold(vec![], |mut vec, p| {
            let current = self.clone(); // This will be a shallow copy since it only needs to clone a pointer
            vec.push(current.at_path(p));
            vec
        })
    }

    /// Deserializes the current buffer as some object
    ///
    /// Note: This will deserialize the entire object, not just the bytes at the current portion of the buffer
    ///
    /// TODO: Need to deprecate or require format_ext be enabled
    #[inline]
    fn to_obj<T: Deserialize<'peek>>(self) -> Option<T>
    where
        Self: Sized,
    {
        self.val()
            .and_then(|v| flexbuffers::from_slice(v.buffer()).ok())
    }

    /// Converts the current Peek into a Graph
    ///
    /// Returns None if the the current position is empty
    #[inline]
    fn to_graph(self) -> Option<Graph<'peek>>
    where
        Self: Sized,
    {
        self.val().map(|p| p.into())
    }

    /// Converts the current Peek into a Graph
    ///
    /// Returns None if the the current position is empty
    #[inline]
    fn to_node(self) -> Option<Node<'peek>>
    where
        Self: Sized,
    {
        self.val().map(|p| p.into())
    }

    /// Returns the peek reader at the wire object
    #[inline]
    fn to_wire_object(self) -> Option<Peek<'peek>>
    where
        Self: Sized,
    {
        self.at_path([".runir", "object"]).and_then(|v| {
            flexbuffers::Reader::get_root(v.as_blob().0)
                .ok()
                .map(Peek::from)
        })
    }
}

impl<'peek> PeekExtensions<'peek> for &'peek Peek<'peek> {
    #[inline]
    fn at(self, key: &str) -> Option<Peek<'peek>> {
        self.get_map().ok().map(|p| Peek(p.idx(key)))
    }

    #[inline]
    fn at_idx(self, idx: usize) -> Option<Peek<'peek>> {
        self.get_vector().ok().map(|p| Peek(p.idx(idx)))
    }

    #[inline]
    fn at_path(self, keys: impl AsRef<[&'peek str]>) -> Option<Peek<'peek>> {
        keys.as_ref()
            .iter()
            .fold(Some(self.clone()), |p, s| p.at(s))
    }

    #[inline]
    fn bool(self) -> Option<bool> {
        self.get_bool().ok()
    }

    #[inline]
    fn str(self) -> Option<&'peek str> {
        self.get_str().ok()
    }

    #[inline]
    fn u64(self) -> Option<u64> {
        self.get_u64().ok()
    }

    #[inline]
    fn as_iter(self) -> Option<impl Iterator<Item = Peek<'peek>>> {
        self.get_vector().ok().map(|v| v.iter().map(|r| Peek(r)))
    }

    #[inline]
    fn in_ref(
        self,
    ) -> impl std::ops::Index<&'peek str, Output = PeekRef<'peek>> + PeekRefExtensions<'peek> {
        PeekRef(RefCell::new(Some(self.clone())))
    }

    #[inline]
    fn as_iter_kv(self) -> Option<impl Iterator<Item = (&'peek str, Peek<'peek>)>> {
        self.get_map().ok().map(|m| {
            let _m2 = m.clone();
            m.iter_keys().map(move |l| (l, Peek(_m2.idx(l))))
        })
    }

    #[inline]
    fn int(self) -> Option<i64> {
        self.clone()
            .0
            .get_u64()
            .ok()
            .map(|u| u as i64)
            .or(self.0.get_i64().ok())
    }

    #[inline]
    fn val(self) -> Option<Peek<'peek>> {
        Some(self.clone())
    }

    #[inline]
    fn blob(self) -> Option<&'peek [u8]> {
        self.get_blob().ok().map(|b| b.0)
    }
}

impl<'peek> PeekExtensions<'peek> for Peek<'peek> {
    #[inline]
    fn in_ref(
        self,
    ) -> impl std::ops::Index<&'peek str, Output = PeekRef<'peek>> + PeekRefExtensions<'peek> {
        PeekRef(RefCell::new(Some(self)))
    }

    #[inline]
    fn at(self, key: &str) -> Option<Peek<'peek>> {
        self.get_map().ok().map(|p| Peek(p.idx(key)))
    }

    #[inline]
    fn at_idx(self, idx: usize) -> Option<Peek<'peek>> {
        self.get_vector().ok().map(|p| Peek(p.idx(idx)))
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
        self.clone()
            .0
            .get_u64()
            .ok()
            .map(|u| u as i64)
            .or(self.0.get_i64().ok())
    }

    #[inline]
    fn as_iter(self) -> Option<impl Iterator<Item = Peek<'peek>>> {
        self.get_vector().ok().map(|v| v.iter().map(|r| Peek(r)))
    }

    #[inline]
    fn as_iter_kv(self) -> Option<impl Iterator<Item = (&'peek str, Peek<'peek>)>> {
        self.get_map().ok().map(|m| {
            let _m = m.clone();
            let _m2 = m.clone();
            m.iter_keys().map(move |l| (l, Peek(_m2.idx(l))))
        })
    }

    #[inline]
    fn at_dot(self, path: &'peek str) -> Option<Peek<'peek>> {
        path.trim_matches(['.'])
            .split_terminator(".")
            .fold(Some(self), |acc, p| acc.at(p))
    }

    #[inline]
    fn val(self) -> Option<Peek<'peek>> {
        Some(self)
    }

    #[inline]
    fn blob(self) -> Option<&'peek [u8]> {
        self.0.get_blob().ok().map(|b| b.0)
    }
}

impl<'peek> PeekExtensions<'peek> for Option<Peek<'peek>> {
    #[inline]
    fn as_iter(self) -> Option<impl Iterator<Item = Peek<'peek>>> {
        self.and_then(|r| r.as_iter())
    }

    #[inline]
    fn in_ref(
        self,
    ) -> impl std::ops::Index<&'peek str, Output = PeekRef<'peek>> + PeekRefExtensions<'peek> {
        PeekRef(RefCell::new(self))
    }

    #[inline]
    fn as_iter_kv(self) -> Option<impl Iterator<Item = (&'peek str, Peek<'peek>)>> {
        self.and_then(|r| r.as_iter_kv())
    }

    #[inline]
    fn val(self) -> Option<Peek<'peek>> {
        self
    }
}

impl<'peek> PeekExtensions<'peek> for &Option<Peek<'peek>> {
    #[inline]
    fn as_iter(self) -> Option<impl Iterator<Item = Peek<'peek>>> {
        self.clone().and_then(|r| r.as_iter())
    }

    #[inline]
    fn in_ref(
        self,
    ) -> impl std::ops::Index<&'peek str, Output = PeekRef<'peek>> + PeekRefExtensions<'peek> {
        PeekRef(RefCell::new(self.clone()))
    }

    #[inline]
    fn as_iter_kv(self) -> Option<impl Iterator<Item = (&'peek str, Peek<'peek>)>> {
        self.clone().and_then(|r| r.as_iter_kv())
    }

    #[inline]
    fn val(self) -> Option<Peek<'peek>> {
        self.clone()
    }
}

impl<'peek> PeekExtensions<'peek> for Option<&'peek Peek<'peek>> {
    #[inline]
    fn in_ref(
        self,
    ) -> impl std::ops::Index<&'peek str, Output = PeekRef<'peek>> + PeekRefExtensions<'peek> {
        PeekRef(RefCell::new(self.cloned()))
    }

    #[inline]
    fn as_iter(self) -> Option<impl Iterator<Item = Peek<'peek>>> {
        self.cloned().and_then(|r| r.as_iter())
    }

    #[inline]
    fn as_iter_kv(self) -> Option<impl Iterator<Item = (&'peek str, Peek<'peek>)>> {
        self.cloned().and_then(|r| r.as_iter_kv())
    }

    #[inline]
    fn val(self) -> Option<Peek<'peek>> {
        self.cloned()
    }
}

impl<'peek> PeekExtensions<'peek> for &'peek crate::Data {
    #[inline]
    fn in_ref(
        self,
    ) -> impl std::ops::Index<&'peek str, Output = PeekRef<'peek>> + crate::util::PeekRefExtensions<'peek>
    {
        self.val().in_ref()
    }

    #[inline]
    fn as_iter(self) -> Option<impl Iterator<Item = crate::util::Peek<'peek>>> {
        self.val().as_iter()
    }

    #[inline]
    fn as_iter_kv(self) -> Option<impl Iterator<Item = (&'peek str, crate::util::Peek<'peek>)>> {
        self.val().as_iter_kv()
    }

    #[inline]
    fn val(self) -> Option<crate::util::Peek<'peek>> {
        flexbuffers::Reader::get_root(self.as_ref())
            .ok()
            .map(Peek::from)
    }
}

impl<'peek> PeekExtensions<'peek> for &'peek [u8] {
    fn val(self) -> Option<Peek<'peek>> {
        flexbuffers::Reader::get_root(self).ok().map(Peek)
    }

    fn as_iter(self) -> Option<impl Iterator<Item = Peek<'peek>>> {
        self.val().as_iter()
    }

    fn as_iter_kv(self) -> Option<impl Iterator<Item = (&'peek str, Peek<'peek>)>> {
        self.val().as_iter_kv()
    }

    fn in_ref(
        self,
    ) -> impl std::ops::Index<&'peek str, Output = PeekRef<'peek>> + PeekRefExtensions<'peek> {
        self.val().in_ref()
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
    fn int(&self) -> Option<i64> {
        self.0.borrow().clone().int()
    }

    #[inline]
    fn iter(&self) -> Option<impl Iterator<Item = Peek<'p>>> {
        self.0.borrow().clone().as_iter()
    }

    #[inline]
    fn iter_kv(&self) -> Option<impl Iterator<Item = (&'p str, Peek<'p>)>> {
        self.0.borrow().clone().as_iter_kv()
    }
}

pub use peek_path::PeekPath;

impl From<&str> for PeekPath {
    fn from(value: &str) -> Self {
        value
            .split_terminator('.')
            .fold(PeekPath::default(), |p, s| p[s])
    }
}

mod peek_path {
    use super::PeekExtensions;
    use crate::util::{Intern, impl_interner};

    /// ⚠️ **Experimental API** – uses pointer arithmetic and interning.
    ///
    /// Provides a reusable, append-only path object for ergonomic field access.
    ///
    /// Unlike `.in_ref()`, this does **not** use interior mutability.
    /// Instead, it builds a `PeekPath` as an immutable XOR-linked list,
    /// relying on interning and pointer identity for path resolution.
    ///
    /// # Example
    /// ```rs no_run
    /// let path = crate::util::PeekPath::default();
    /// let other_values = &path["other"]["values"];
    ///
    /// assert_eq!(
    ///     Some("another hello"),
    ///     other_values["also_important"]
    ///         .lookup(kv.serde().peek("hello2"))
    ///         .str()
    /// );
    ///
    /// assert_eq!(
    ///     None,
    ///     other_values["doesn't exist"]
    ///         .lookup(kv.serde().peek("hello2"))
    ///         .str()
    /// );
    ///
    /// // Paths are reusable
    /// assert_eq!(
    ///     Some("another hello"),
    ///     other_values["also_important"]
    ///         .lookup(kv.serde().peek("hello2"))
    ///         .str()
    /// );
    /// ```
    ///
    /// ## Tradeoffs
    /// - ✅ Cheap to clone and reuse
    /// - ✅ No allocations or mutation **during** traversal
    /// - ⚠️ Requires stable memory addresses (i.e. `'static`
    ///   interning of path segments)
    /// - ⚠️ Pointer math under the hood — use with care
    ///
    /// If you need strict correctness or dynamic safety,  
    /// prefer `.at(..)` or `.at_path(..)` from the `Peek` API.
    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct PeekPath {
        head: Option<*const PathNode>,
        depth: usize,
    }
    unsafe impl Sync for PeekPath {}

    impl std::ops::Index<&str> for PeekPath {
        type Output = PeekPath;

        fn index(&self, index: &str) -> &Self::Output {
            self.append(index)
        }
    }

    #[derive(Debug, Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct PathNode {
        current: &'static str,
        link: usize,
    }
    unsafe impl Sync for PathNode {}

    impl PeekPath {
        /// ⚠️ **Experimental API** – uses pointer arithmetic and interning.
        ///
        /// Resolves this `PeekPath` against a `Peek` root and returns the final nested reader, if found.
        ///
        /// This is a zero-copy, non-allocating **lookup** traversal using an XOR-linked path structure,
        /// designed for fast, repeated lookups without re-parsing or string comparisons.
        ///
        /// # Example
        /// ```rs no_run
        /// let path = &PeekPath::default()["outer"]["inner"];
        /// let value = path.lookup(root).str();
        /// ```
        ///
        /// # Returns
        /// - `Some(Peek)` if all segments in the path exist and lead to a value
        /// - `None` if any segment is missing or not a map
        ///
        /// # Notes
        /// - `PeekPath` is built using interned `&'static str` segments
        /// - The lookup is performed against a `Peek` tree (usually from `kv.serde().peek(..)`)
        /// - Internally relies on unsafe pointer operations; correctness requires that the PeekPath was built via valid interning.
        ///
        /// It's basically a DAG of interned, deduplicated segments, reconstructed via symmetric keying.
        ///
        /// For dynamic or one-off lookups, prefer using [`Peek::at_path`] or [`Peek::at_dot`].
        ///
        /// See [`PeekPath`] for more details on how these reusable paths are constructed.
        #[inline]
        pub fn lookup<'a>(&'a self, peek: impl PeekExtensions<'a>) -> impl PeekExtensions<'a> {
            peek.at_path(self.resolve().as_slice())
        }

        // fn append(&self, segment: &str) -> &'static PeekPath {
        //     let _seg = intern_str(segment);
        //     let segment: *const _ = _seg;
        //     let link = self.link(segment as *const () as usize);

        //     let new = intern_path_node(PathNode {
        //         current: _seg,
        //         link,
        //     });

        //     intern_path(PeekPath { head: Some(new) })
        // }

        // fn link(&self, next: usize) -> usize {
        //     self.resolve()
        //         .iter()
        //         .last()
        //         .map(|s| *s as *const _ as *const () as usize)
        //         .map_or(0, |l| l ^ next)
        // }

        // fn resolve(&self) -> Vec<&'static str> {
        //     let mut path = Vec::new();
        //     let mut current = self.head;

        //     while let Some(node) = current {
        //         let node = unsafe {
        //             node.as_ref().expect("should not be null as these values are always from 'static interned values")
        //         };
        //         path.push(node.current);

        //         let next_ptr = node.link ^ (node.current as *const _ as *const () as usize);

        //         current = PATH_NODE_INTERNER
        //             .get()
        //             .and_then(|set| {
        //                 set.nodes.iter().find(|n| {
        //                     (n.current as *const _ as *const () as usize) == next_ptr
        //                         && n.link != node.link
        //                 })
        //             })
        //             .map(|n| *n as *const _)
        //     }

        //     path.reverse();
        //     path
        // }

        pub fn append(&self, seg: &str) -> &'static PeekPath {
            let chain_hash = self.structural_hash(seg.addr_of_interned() as usize); // XOR of all previous segment pointers

            // Create a new immutable node
            let new = PathNode {
                current: seg.intern(),
                link: chain_hash,
            }
            .intern();

            PeekPath {
                head: Some(new),
                depth: self.depth + 1,
            }
            .intern()
        }

        fn structural_hash(&self, next: usize) -> usize {
            self.resolve()
                .iter()
                .last()
                .map(|s| s.addr_of_interned() as usize)
                .map_or(0, |l| l ^ next)
        }

        pub fn resolve(&self) -> Vec<&'static str> {
            let mut path = Vec::new();
            let mut current = self.head;

            // let mut last_n_link = 0;

            let intern_state = PathNode::interner_state();
            while let Some(node) = current {
                let node = unsafe { node.as_ref().unwrap() };
                path.push(node.current);

                if self.depth == path.len() {
                    break;
                }

                let next_ptr = node.link ^ (node.current.addr_of_interned() as usize);

                current = if self.depth.saturating_sub(path.len()) == 1 {
                    // The below search algo would never find the root, so we leave early to let the last step complete
                    None
                } else {
                    intern_state
                        .iter()
                        .copied()
                        .find(|n| {
                            // tracing::debug!(
                            //     depth = self.depth,
                            //     path_len = path.len(),
                            //     "Searching: n.link: {}, node.link: {}",
                            //     n.link,
                            //     node.link,
                            //     // last_n_link ^ node.link == 0,
                            // );
                            (n.current.addr_of_interned() as usize) == next_ptr
                                && n.link != node.link
                                && if node.link == 0 {
                                    // last_n_link ^ node.link == 0 {
                                    // we're inside the chain
                                    path.len() < self.depth && n.link != 0 // while we're inside the chain, and still not at the root, lhs can't be 0
                                } else {
                                    n.link != 0 // && last_n_link == 0 // we're not yet inside the chain, so lhs_link must be 0 and n.link can't be the root
                                }
                        })
                        // .inspect(|n| {
                        //     tracing::debug!(
                        //         depth = self.depth,
                        //         path_len = path.len(),
                        //         "\tFound --> n.link: {}, node.link: {}",
                        //         n.link,
                        //         node.link,
                        //         // last_n_link ^ node.link == 0,
                        //     );
                        //     // last_n_link = n.link;
                        // })
                        .map(|n| n as *const _)
                };
                if current.is_none() {
                    if let Some(last) = intern_state.iter().find(|n| {
                        // tracing::debug!(
                        //     depth = self.depth,
                        //     path_len = path.len(),
                        //     "Searching for root: n.current: {}, next_ptr: {}",
                        //     n.current,
                        //     next_ptr,
                        //     // last_n_link ^ node.link == 0,
                        // );
                        (n.current.addr_of_interned() as usize) == next_ptr
                    })
                    // .inspect(|n| {
                    //     tracing::debug!(
                    //         depth = self.depth,
                    //         path_len = path.len(),
                    //         "\tFound root --> n.current: {}, next_ptr: {}",
                    //         n.current,
                    //         next_ptr,
                    //         // last_n_link ^ node.link == 0,
                    //     );
                    // })
                    {
                        // If our depth is only 1, which means we are at the root;
                        // Then, the root will be the first segment added to path.
                        // The below is only required when path len is greater than 1, which means we did begin at the root
                        if path.len() > 1 {
                            path.push(last.current);
                        }
                    }
                }
            }

            path.reverse();
            path
        }
    }

    impl_interner!(path_node_interner, PathNode, Clone::clone);
    impl_interner!(path_interner, PeekPath, Clone::clone);

    #[test]
    #[tracing_test::traced_test]
    fn test_peek_path() {
        let peek_path = PeekPath::default();
        let other_values = &peek_path["other"]["values"]["also_important"];

        // eprintln!("{:?}", &peek_path["other"].resolve());
        // eprintln!("{:?}", other_values["also_important"].resolve());
        // eprintln!("{:?}", other_values["test1"]["test2"].resolve());

        assert_eq!(vec!["other"], peek_path["other"].resolve());
        assert_eq!(
            vec!["other", "values", "also_important", "also_important"],
            other_values["also_important"].resolve()
        );
        assert_eq!(
            vec!["other", "values", "also_important", "test1", "test2"],
            other_values["test1"]["test2"].resolve()
        );
        assert_eq!(
            vec!["other", "values", "also_important", "also_important"],
            other_values["also_important"].resolve()
        );
    }
}

pub mod peek_ser {
    use serde::Serialize;

    use super::{Peek, PeekExtensions};

    impl<'peek> Serialize for Peek<'peek> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            if let Some(map) = self.as_iter_kv() {
                serializer.collect_map(map)
            } else if let Some(arr) = self.as_iter() {
                serializer.collect_seq(arr)
            } else {
                match self.flexbuffer_type() {
                    flexbuffers::FlexBufferType::Null => serializer.serialize_none(),
                    flexbuffers::FlexBufferType::Int => match self.bitwidth() {
                        flexbuffers::BitWidth::W8 => serializer.serialize_i8(self.as_i8()),
                        flexbuffers::BitWidth::W16 => serializer.serialize_i16(self.as_i16()),
                        flexbuffers::BitWidth::W32 => serializer.serialize_i32(self.as_i32()),
                        flexbuffers::BitWidth::W64 => serializer.serialize_i64(self.as_i64()),
                    },
                    flexbuffers::FlexBufferType::UInt => match self.bitwidth() {
                        flexbuffers::BitWidth::W8 => serializer.serialize_u8(self.as_u8()),
                        flexbuffers::BitWidth::W16 => serializer.serialize_u16(self.as_u16()),
                        flexbuffers::BitWidth::W32 => serializer.serialize_u32(self.as_u32()),
                        flexbuffers::BitWidth::W64 => serializer.serialize_u64(self.as_u64()),
                    },
                    flexbuffers::FlexBufferType::Float => match self.bitwidth() {
                        flexbuffers::BitWidth::W32 => serializer.serialize_f32(self.as_f32()),
                        flexbuffers::BitWidth::W64 => serializer.serialize_f64(self.as_f64()),
                        _ => {
                            unreachable!()
                        }
                    },
                    flexbuffers::FlexBufferType::Bool => serializer.serialize_bool(self.as_bool()),
                    flexbuffers::FlexBufferType::String => serializer.serialize_str(self.as_str()),
                    flexbuffers::FlexBufferType::Blob => {
                        serializer.serialize_bytes(self.as_blob().0)
                    }
                    _ => serializer.serialize_none(),
                }
            }
        }
    }

    #[test]
    fn test_serialize() {
        use crate::{IRecord, Namespace};
        use toml::toml;

        let rec = Namespace::ephemeral().store(
            "test",
            &toml! {
                [test]
                value = "hello world"

                [test2]
                value = 3.14

                [test3]
                value = 100
            },
        );

        let val = rec.peek().val().unwrap();

        let val = toml::to_string(&val).unwrap();
        eprintln!("{val}");
    }
}
