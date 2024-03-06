use crate::prelude::*;
use crate::repr::HANDLES;
use std::sync::Arc;

/// Struct for linking together levels into a single representation,
///
#[derive(Default)]
pub struct Linker<I = CrcInterner>
where
    I: InternerFactory,
{
    /// Interner,
    ///
    interner: I,
    /// Vector of intern handles tags for each level of the current representation,
    ///
    levels: Vec<Tag<InternHandle, Arc<InternHandle>>>,
}

impl Linker<CrcInterner> {
    /// Returns a new linker w/ a crc-interner,
    ///
    pub fn new_crc<T: Send + Sync + 'static>() -> Self {
        Self::describe_resource::<T>()
    }
}

impl Linker<EntityInterner<CrcInterner>> {
    /// Returns a new linker w/ an entity crc-interner,
    ///
    /// **Note** This allows single level intern handles because
    /// each new handle is assigned a counter value in addition to
    /// the checksum hash of the resource type,
    ///
    pub fn new_entity_crc<T: Send + Sync + 'static>() -> Self {
        Self::describe_resource::<T>()
    }
}

impl<I: InternerFactory + Default> Linker<I> {
    /// Constructs and returns a new representation,
    ///
    pub fn link(&mut self) -> anyhow::Result<Repr> {
        let tail = self.levels.iter().try_fold(
            Tag::new(&HANDLES, Arc::new(InternHandle::default())),
            |from, to| {
                let _ = from.link(to)?;

                Ok::<_, anyhow::Error>(to.clone())
            },
        )?;

        let tail = tail.value();

        if let Some(tail) = HANDLES.copy(&tail) {
            Ok(Repr { tail })
        } else {
            Err(anyhow::anyhow!("Could not create representation"))
        }
    }

    /// Pushes a level to the current stack of levels,
    ///
    pub fn push_level(&mut self, level: impl Level) -> anyhow::Result<()> {
        // Configure a new handle
        let handle = level.configure(&mut self.interner)?;

        // Handle errors
        if let Some(last) = self.levels.last() {
            let flag = last.create_value.level_flags();

            if flag != LevelFlags::from_bits_truncate(handle.level_flags().bits() >> 1) {
                Err(anyhow::anyhow!("Expected next level"))?;
            }
        } else if handle.level_flags() != LevelFlags::ROOT {
            Err(anyhow::anyhow!("Expected root level"))?;
        }

        // Push the level to the stack
        self.levels.push(Tag::new(&HANDLES, Arc::new(handle)));

        Ok(())
    }

    /// Creates a new repr w/ the root as the ResourceLevel,
    ///
    #[inline]
    pub(crate) fn describe_resource<T: Send + Sync + 'static>() -> Self {
        let mut repr = Linker::default();

        repr.push_level(ResourceLevel::new::<T>())
            .expect("should be able to push since the repr is empty");

        repr
    }

    /// Returns the current representation level,
    ///
    #[inline]
    pub fn level(&self) -> usize {
        self.levels.len() - 1
    }
}

#[allow(unused)]
mod tests {
    use super::Linker;

    #[test]
    fn test_entity_crc() {
        struct Test {
            name: String,
        }

        let mut linker = Linker::new_entity_crc::<Test>();
        let a = linker.link().unwrap();
        let b = linker.link().unwrap();

        eprintln!("{:x?}", a);
        eprintln!("{:x?}", b);

        ()
    }
}
