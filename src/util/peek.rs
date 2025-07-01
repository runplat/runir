use std::ops::Deref;

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

/// Extension methods for working with a Peek container
pub trait PeekExtensions<'peek> {
    /// Peek at the reader under a specific key, if the current peek context is a map
    fn at(&'peek self, key: &str) -> Option<Peek<'peek>>;

    /// Returns a bool if the current peek context is a bool
    fn bool(&'peek self) -> Option<bool>;

    /// Returns a str if the current peek context is a str
    fn str(&'peek self) -> Option<&'peek str>;

    /// Returns a u64 if the current peek context is a u64
    fn u64(&'peek self) -> Option<u64>;
}

impl<'peek> PeekExtensions<'peek> for Peek<'peek> {
    #[inline]
    fn bool(&'peek self) -> Option<bool> {
        self.0.get_bool().ok()
    }

    #[inline]
    fn str(&'peek self) -> Option<&'peek str> {
        self.0.get_str().ok()
    }

    #[inline]
    fn u64(&'peek self) -> Option<u64> {
        self.0.get_u64().ok()
    }

    #[inline]
    fn at(&'peek self, key: &str) -> Option<Peek<'peek>> {
        self.get_map().ok().map(|p| Peek(p.idx(key)))
    }
}

impl<'peek> PeekExtensions<'peek> for Option<Peek<'peek>> {
    #[inline]
    fn bool(&'peek self) -> Option<bool> {
        self.as_ref().and_then(|r| r.bool())
    }

    #[inline]
    fn str(&'peek self) -> Option<&'peek str> {
        self.as_ref().and_then(|r| r.str())
    }

    #[inline]
    fn u64(&'peek self) -> Option<u64> {
        self.as_ref().and_then(|r| r.u64())
    }

    #[inline]
    fn at(&'peek self, key: &str) -> Option<Peek<'peek>> {
        self.as_ref().and_then(|r| r.at(key))
    }
}
