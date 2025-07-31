use std::{
    ops::{Deref, DerefMut},
    sync::OnceLock,
};

use super::PeekExtensions;
use crate::{IRecord, vol::Volume};
use serde::de::DeserializeOwned;

/// Wraps an IRecord to provide an IRecord interface w/ a mutable interface w/ the stored object
pub struct RunCell<T, R>(Volume<OnceLock<T>, R>);

impl<T, R: IRecord> RunCell<T, R> {
    /// Creates a new run cell w/ source
    #[inline]
    pub fn new(record: R) -> Self {
        Self((OnceLock::new(), record).into())
    }

    /// Resets the inner state
    ///
    /// Returns the existing value of T if a value was set
    #[inline]
    pub fn reset(&mut self) -> Option<T> {
        self.0.target_mut().take()
    }

    /// Returns a reference to the inner source
    #[inline]
    pub fn as_source(&self) -> &R {
        &self.0.encoder()
    }

    /// Returns a mutable reference to the inner source
    #[inline]
    pub fn as_source_mut(&mut self) -> &mut R {
        self.0.encoder_mut()
    }
}

impl<T, R: IRecord> IRecord for RunCell<T, R> {
    fn ns_chk(&self) -> u64 {
        self.0.encoder().ns_chk()
    }

    fn uuid(&self) -> uuid::Uuid {
        self.0.encoder().uuid()
    }

    fn opts(&self) -> &crate::Opts {
        self.0.encoder().opts()
    }

    fn opts_mut(&mut self) -> Option<&mut crate::Opts> {
        self.0.encoder_mut().opts_mut()
    }

    fn bytes(&self) -> &[u8] {
        self.0.encoder().bytes()
    }

    fn to_record(&self) -> crate::Record {
        self.0.encoder().to_record()
    }
}

impl<'b, T, R: IRecord> IRecord for &'b RunCell<T, R> {
    fn ns_chk(&self) -> u64 {
        <RunCell<T, R> as IRecord>::ns_chk(self)
    }

    fn uuid(&self) -> uuid::Uuid {
        <RunCell<T, R> as IRecord>::uuid(self)
    }

    fn opts(&self) -> &crate::Opts {
        <RunCell<T, R> as IRecord>::opts(self)
    }

    fn opts_mut(&mut self) -> Option<&mut crate::Opts> {
        None
    }

    fn bytes(&self) -> &[u8] {
        <RunCell<T, R> as IRecord>::bytes(self)
    }

    fn to_record(&self) -> crate::Record {
        <RunCell<T, R> as IRecord>::to_record(self)
    }
}

impl<T: DeserializeOwned + Default, R: IRecord> AsMut<T> for RunCell<T, R> {
    fn as_mut(&mut self) -> &mut T {
        self.as_ref();

        self.0.target_mut()
            .get_mut()
            .expect("should exist just initialized")
    }
}

impl<T: DeserializeOwned + Default, R: IRecord> AsRef<T> for RunCell<T, R> {
    fn as_ref(&self) -> &T {
        self.0.target()
            .get_or_init(|| self.0.encoder().peek().to_obj::<T>().unwrap_or_default())
    }
}

impl<T: DeserializeOwned + Default, R: IRecord> Deref for RunCell<T, R> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<T: DeserializeOwned + Default, R: IRecord> DerefMut for RunCell<T, R> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.as_mut()
    }
}

impl<T, R: Clone> Clone for RunCell<T, R> {
    fn clone(&self) -> Self {
        Self((OnceLock::new(), self.0.encoder().clone()).into())
    }
}

#[cfg(test)]
mod test {
    use super::RunCell;
    use crate::Namespace;
    use serde::{Deserialize, Serialize};

    #[derive(Default, Serialize, Deserialize)]
    struct Test {
        value: usize,
    }

    impl Test {
        fn increment(&mut self) {
            self.value += 1;
        }
    }

    #[test]
    fn test_run_cell() {
        let ns = Namespace::ephemeral();

        let rec = ns.store("test", &Test { value: 42 });

        let mut cell = RunCell::<Test, _>::new(rec);

        assert_eq!(42, cell.value);
        cell.increment();

        assert_eq!(43, cell.as_mut().value);
        assert_eq!(43, cell.reset().unwrap().value);
    }
}
