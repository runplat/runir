use std::ops::Deref;
use std::sync::Arc;

use crate::prelude::*;
use crate::repr::HANDLES;

/// Each level of runtime representation is defined by a set of tags,
///
#[derive(Clone, Copy)]
pub struct Tag<T: Send + Sync + 'static, F: Sync = fn() -> T> {
    /// Table that contains the tag value,
    ///
    pub(crate) intern_table: &'static InternTable<T>,
    /// Create value method,
    ///
    pub(crate) create_value: F,
}

impl<T: Send + Sync + 'static, F: Sync> Tag<T, F> {
    /// Returns a new tag,
    ///
    #[inline]
    pub const fn new(intern_table: &'static InternTable<T>, create_value: F) -> Self {
        Self {
            intern_table,
            create_value,
        }
    }
}

impl<T: Send + Sync + 'static> Tag<T> {
    /// Assigns a value to an intern handle,
    ///
    #[inline]
    pub fn assign(&self, handle: InternHandle) -> anyhow::Result<()> {
        self.intern_table
            .assign_intern(handle, (self.create_value)())
    }

    /// Returns the inner value,
    ///
    #[inline]
    pub fn value(&self) -> T {
        (self.create_value)()
    }
}

impl<T: ToOwned<Owned = T> + Send + Sync + 'static> Tag<T, Arc<T>> {
    /// Assign a value to an intern handle,
    ///
    #[inline]
    pub fn assign(&self, handle: InternHandle) -> anyhow::Result<()> {
        self.intern_table
            .assign_intern(handle, self.create_value.deref().to_owned())
    }

    /// Returns the inner value,
    ///
    #[inline]
    pub fn value(&self) -> T {
        self.create_value.deref().to_owned()
    }
}

impl Tag<InternHandle, Arc<InternHandle>> {
    /// Creates and assigns an intern handle representing the link between the current intern handle and the
    /// next intern handle.
    ///
    pub fn link(
        &self,
        next: &Tag<InternHandle, Arc<InternHandle>>,
    ) -> anyhow::Result<InternHandle> {
        let from = self.create_value.clone();
        let to = next.create_value.clone();

        if !from.level_flags().is_empty()
            && from.level_flags().bits() << 1 != to.level_flags().bits()
        {
            Err(anyhow::anyhow!(
                "Trying to link an intern handle out of order"
            ))?;
        }

        let link = from.register() ^ to.register();

        let mut out = *to.clone();
        out.link = link;

        Tag::new(&HANDLES, Arc::new(out)).assign(*to)?;

        Ok(out)
    }
}
