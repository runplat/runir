use tracing::warn;

use crate::{define_intern_table, prelude::*};

/// Wraps an inner interner and assigns an entity-id to the intern result,
///
#[derive(Default)]
pub struct EntityInterner<Inner: InternerFactory> {
    inner: Inner,
    counter: u64,
}

define_intern_table!(ENTITY: u64);

impl<Inner: InternerFactory> InternerFactory for EntityInterner<Inner> {
    fn push_tag<T: std::hash::Hash + Send + Sync + 'static>(
        &mut self,
        value: T,
        tag: impl Fn(crate::prelude::InternHandle) -> anyhow::Result<()> + Send + Sync + 'static,
    ) {
        self.inner.push_tag(value, tag);
    }

    fn set_level_flags(&mut self, flags: crate::prelude::LevelFlags) {
        self.inner.set_level_flags(flags);
    }

    fn set_data(&mut self, _: u64) {
        warn!("Data is managed by entity interner");
    }

    fn interner(&mut self) -> crate::prelude::InternResult {
        // **Note** Entity 0 is reserved
        self.counter += 1;
        self.inner.set_data(self.counter);
        let result = self.inner.interner()?;

        ENTITY.assign_intern(result, self.counter)?;

        Ok(result)
    }
}
