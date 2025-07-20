use std::sync::Arc;

/// Trait enabling interner implementation
pub trait Interner: private::Sealed {
    /// Associated type stored by interner
    type Intern: Intern + ?Sized;

    /// Interns a value w/ this interner
    fn intern(self: Arc<Self>, val: &Self::Intern) -> &'static Self::Intern;

    /// Returns an arc reference to the interner singleton
    fn singleton() -> Arc<Self>;
}

/// Trait enables value interning on a type
pub trait Intern: private::Sealed {
    /// Interner defined by impl_interner macro
    type Interner: Interner<Intern = Self> + Default;

    /// Converts a reference to a static reference
    fn to_static(val: &Self) -> &'static Self;

    /// Returns a static reference to this value
    fn intern(&self) -> &'static Self {
        Interner::intern(Self::Interner::singleton(), self)
    }

    fn interner() -> Arc<Self::Interner> {
        Self::Interner::singleton()
    }
}

impl Intern for str {
    type Interner = str_interner::Interner;

    fn to_static(val: &Self) -> &'static Self {
        Box::leak(val.to_string().into_boxed_str())
    }
}

/// Implements an interner for a type
macro_rules! impl_interner {
    ($name:ident, $ty:ty) => {
        mod $name {
            #[allow(unused)]
            use super::*;

            static INTERNER: std::sync::OnceLock<std::sync::Arc<Interner>> = std::sync::OnceLock::new();

            #[derive(Default)]
            pub struct Interner {
                pub interned: dashmap::DashSet<&'static $ty>,
                pub check_list: dashmap::DashMap<u64, &'static $ty>,
            }

            impl crate::util::interner::private::Sealed for $ty {}
            impl crate::util::interner::private::Sealed for Interner {}

            impl crate::util::interner::Interner for Interner {
                type Intern = $ty;

                fn intern(self: std::sync::Arc<Self>, val: &$ty) -> &'static $ty {
                    use std::hash::Hash;
                    use std::hash::Hasher;
                    if let Some(v) = self.interned.get(val) {
                        &(*v)
                    } else {
                        let mut hasher = ahash::AHasher::default();
                        val.hash(&mut hasher);

                        let s = self.check_list.entry(hasher.finish()).or_insert_with(|| {
                            let interned: &'static $ty = <$ty as crate::util::interner::Intern>::to_static(val);
                            interned
                        });

                        self.interned.insert(&s);
                        &s
                    }
                }

                fn singleton() -> std::sync::Arc<Self> {
                    INTERNER.get_or_init(|| std::sync::Arc::new(Self::default())).clone()
                }
            }
        }
    };
}

pub(crate) use impl_interner;

impl_interner!(str_interner, str);

pub(crate) mod private {
    pub trait Sealed {}
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_str_interner() {
        assert_eq!("hello world", "hello world".intern());
    }
}
