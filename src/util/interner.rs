use std::hash::Hash;

/// Trait to enable interning values of a 'static type
pub trait Intern: private::ToStatic {
    /// Returns a static reference to this value
    fn intern(&self) -> &'static Self;

    /// Returns the current interner state
    /// 
    /// Note: Each member of the hashset is guranteed to have a stable memory address
    fn interner_state() -> HashSet<&'static Self>;

    /// Returns the address of the interned value as a raw, opaque pointer
    ///
    /// If the value has not yet been interned, it will be interned first
    fn addr_of_interned(&self) -> *const ()
    {
        self.intern() as *const _ as *const ()
    }
}

/// Implements an Interner for a Type
///
/// ## Usage
/// ```rs no_run
/// impl_interner(str_interner, str, str::to_string)
/// impl_interner(my_interner, MyType, Clone::clone)
/// ```
macro_rules! impl_interner {
    ($name:ident, $ty:ty, $to_owned:expr) => {
        mod $name {
            #[allow(unused)]
            use super::*;

            /// Returns a static reference to a value
            pub fn intern(val: &$ty) -> &'static $ty {
                use crate::util::interner::private::Interner;
                __Interner::intern(__Interner::singleton(), val)
            }

            /// Returns a hash set of the current interner state
            pub fn interner_state() -> ahash::HashSet<&'static $ty> {
                use crate::util::interner::private::Interner;
                ahash::HashSet::from_iter(interner().interned())
            }

            /// Returns a clone of the interner singleton
            pub fn interner()
            -> std::sync::Arc<impl crate::util::interner::private::Interner<Intern = $ty>> {
                use crate::util::interner::private::Interner;
                __Interner::singleton()
            }

            static INTERNER: std::sync::OnceLock<std::sync::Arc<__Interner>> =
                std::sync::OnceLock::new();

            #[derive(Default)]
            struct __Interner(crate::util::interner::InternStore<$ty>);

            impl crate::util::interner::private::Interner for __Interner {
                type Intern = $ty;

                fn intern(self: std::sync::Arc<Self>, val: &$ty) -> &'static $ty {
                    self.0.__intern(val)
                }

                fn singleton() -> std::sync::Arc<Self> {
                    INTERNER
                        .get_or_init(|| std::sync::Arc::new(Self::default()))
                        .clone()
                }

                fn interned(&self) -> impl Iterator<Item = &'static Self::Intern>
                where
                    Self::Intern: 'static,
                {
                    self.0.__interned()
                }
            }
        }

        impl crate::util::Intern for $ty {
            fn intern(&self) -> &'static Self {
                $name::intern(self)
            }

            fn interner_state() -> ahash::HashSet<&'static Self>
            where
                Self: 'static,
            {
                $name::interner_state()
            }
        }

        impl crate::util::interner::private::ToStatic for $ty {
            fn to_static(val: &Self) -> &'static Self {
                Box::leak(Box::new($to_owned(val)))
            }
        }
    };
}

use ahash::HashSet;
pub(crate) use impl_interner;

impl_interner!(str_interner, str, str::to_string);

pub(crate) mod private {
    use std::sync::Arc;

    /// Trait enabling interner implementation
    pub trait Interner {
        /// Associated type stored by interner
        type Intern: super::Intern + ?Sized;

        /// Interns a value w/ this interner
        fn intern(self: Arc<Self>, val: &Self::Intern) -> &'static Self::Intern;

        /// Returns an arc reference to the interner singleton
        fn singleton() -> Arc<Self>;

        /// Returns an iterator over interned items
        fn interned(&self) -> impl Iterator<Item = &'static Self::Intern>
        where
            Self::Intern: 'static;
    }

    /// Trait for types that can return a static reference of themselves
    pub trait ToStatic : 'static {
        /// Converts a reference to a static reference
        fn to_static(val: &Self) -> &'static Self;
    }
}

/// Internal Interner implementation
pub(crate) struct InternStore<T: ?Sized + 'static> {
    interned: dashmap::DashSet<&'static T>,
    check_list: dashmap::DashMap<u64, &'static T>,
}

impl<T: ?Sized + Eq + Hash + 'static> Default for InternStore<T> {
    fn default() -> Self {
        Self {
            interned: dashmap::DashSet::new(),
            check_list: Default::default(),
        }
    }
}

impl<T: Intern + Hash + Eq + ?Sized + 'static> InternStore<T> {
    #[inline]
    pub fn __intern(&self, val: &T) -> &'static T {
        use std::hash::Hasher;
        if let Some(v) = self.interned.get(val) {
            &(*v)
        } else {
            let mut hasher = ahash::AHasher::default();
            val.hash(&mut hasher);

            let s = self.check_list.entry(hasher.finish()).or_insert_with(|| {
                let interned: &'static T = T::to_static(val);
                interned
            });

            self.interned.insert(&s);
            &s
        }
    }

    #[inline]
    pub fn __interned(&self) -> impl Iterator<Item = &'static T>
    where
        T: 'static,
    {
        self.interned.iter().map(|v| *v)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_str_interner() {
        assert_eq!("hello world", "hello world".intern());
        assert_eq!(
            "hello world".addr_of_interned(),
            "hello world".addr_of_interned()
        );
    }
}
