use std::sync::Arc;

use serde::Deserialize;
use serde::Serialize;

use crate::define_intern_table;
use crate::prelude::*;
use crate::push_tag;

// Intern table for address values
define_intern_table!(ADDRESS: String);

// Intern table for extension values
define_intern_table!(EXTENSIONS: Vec<Repr>);

/// Host level is the upper most level of representation,
///
/// Host level assigns an address that represents the current representation.
///
pub struct HostLevel {
    /// The address is derived by the documentation hierarchy from runmd and
    /// is some human-readable string associated to some resource.
    ///
    address: Tag<String, Arc<String>>,
    /// Extensions that have been added under this host,
    ///
    extensions: Option<Tag<Vec<Repr>, Arc<Vec<Repr>>>>,
}

impl HostLevel {
    /// Creates a new host level representation,
    ///
    #[inline]
    pub fn new(address: impl Into<String>) -> Self {
        Self {
            address: Tag::new(&ADDRESS, Arc::new(address.into())),
            extensions: None,
        }
    }

    /// Sets extensions on the host,
    ///
    #[inline]
    pub fn set_extensions(&mut self, extensions: Vec<Repr>) {
        self.extensions = Some(Tag::new(&EXTENSIONS, Arc::new(extensions)));
    }
}

impl Level for HostLevel {
    fn configure(&self, interner: &mut impl InternerFactory) -> InternResult {
        push_tag!(dyn interner, &self.address);

        if let Some(extensions) = self.extensions.as_ref() {
            push_tag!(dyn interner, extensions);
        }

        interner.set_level_flags(LevelFlags::LEVEL_3);

        interner.interner()
    }

    type Mount = (Arc<String>, Option<Arc<Vec<Repr>>>);

    #[inline]
    fn mount(&self) -> Self::Mount {
        (
            self.address.create_value.clone(),
            self.extensions.as_ref().map(|e| e.create_value.clone()),
        )
    }
}

/// Wrapper struct with access to host tags,
///
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash, Serialize, Deserialize)]
pub struct HostRepr(pub(crate) InternHandle);

impl HostRepr {
    /// Returns the address provided by the host,
    ///
    #[inline]
    pub fn address(&self) -> Option<Arc<String>> {
        self.0.host_address()
    }

    /// Returns the address provided by the host,
    ///
    #[inline]
    pub fn extensions(&self) -> Option<Arc<Vec<Repr>>> {
        self.0.host_extensions()
    }

    /// Finds the repr of a field owned by receiver,
    ///
    pub fn find_extension(&self, name: &str) -> Option<Repr> {
        if let Some(extensions) = self.extensions() {
            extensions
                .iter()
                .find(|f| {
                    f.as_recv()
                        .and_then(|f| {
                            if f.name().map(|n| n.to_string()) == Some(name.to_string()) {
                                Some(f)
                            } else {
                                None
                            }
                        })
                        .is_some()
                })
                .copied()
        } else {
            None
        }
    }
}
