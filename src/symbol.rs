use crate::util::Intern;
use serde::Serialize;
use std::hash::Hash;

/// Super-trait for "symbols" which are used for deriving the keyform of namespaces and record labels
pub trait Symbol: Sized {
    /// Returns a the canonical hash reference of the symbol
    fn symbol(&self) -> &impl Hash;
}

impl Symbol for &str {
    #[inline]
    fn symbol(&self) -> &impl Hash {
        self
    }
}

/// Provides functions for computing a symbol
#[derive(Hash)]
pub struct ComputedSymbol {
    computed: &'static str,
}

impl ComputedSymbol {
    /// Computes a symbol from a mustache based template and serializable data
    ///
    /// Returns an error if the template could not be compiled or if the symbol could not be rendered
    #[inline]
    pub fn mustache(template: &str, data: &impl Serialize) -> crate::Result<Self> {
        let template = mustache::compile_str(template)?;

        Self::with_mustache(&template, data)
    }

    /// Computes a symbol w/ a mustache template
    ///
    /// Returns an error if the symbol could not be rendered
    #[inline]
    pub fn with_mustache(
        template: &mustache::Template,
        data: &impl Serialize,
    ) -> crate::Result<Self> {
        Self::try_compute(|| {
            Ok(template.render_to_string(data)?)
        })
    }

    /// Computes a symbol w/ closure
    #[inline]
    pub fn compute(c: impl FnOnce() -> String + 'static) -> Self {
        Self {
            computed: c().as_str().intern(),
        }
    }

    /// Computes a symbole w/ closure
    /// 
    /// Returns an error if the closure returns an error
    #[inline]
    pub fn try_compute(c: impl FnOnce() -> crate::Result<String>) -> crate::Result<Self> {
        Ok(Self {
            computed: c()?.as_str().intern(),
        })
    }

    /// Returns the computed symbol
    #[inline]
    pub fn computed(&self) -> &'static str {
        self.computed
    }
}

impl Symbol for ComputedSymbol {
    #[inline]
    fn symbol(&self) -> &impl Hash {
        &self.computed
    }
}

#[cfg(test)]
mod test {
    use super::ComputedSymbol;
    use crate::util::Intern;
    use toml::toml;

    #[test]
    fn test_computed_symbol() {
        let s = ComputedSymbol::mustache("{{name}}-test", &toml! {name = "hello world"}).unwrap();
        assert_eq!("hello world-test", s.computed());
        assert_eq!(
            "hello world-test".addr_of_interned(),
            s.computed().addr_of_interned()
        );

        let t = mustache::compile_str("{{name}}-test").unwrap();
        let s = ComputedSymbol::with_mustache(&t, &toml! {name = "hello world 2"}).unwrap();
        assert_eq!("hello world 2-test", s.computed());
        assert_eq!(
            "hello world 2-test".addr_of_interned(),
            s.computed().addr_of_interned()
        );

        let s = ComputedSymbol::compute(|| String::from("hello world"));
        assert_eq!("hello world", s.computed());
        assert_eq!(
            "hello world".addr_of_interned(),
            s.computed().addr_of_interned()
        );
    }
}
