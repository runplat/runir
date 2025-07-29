use std::{
    ops::{Deref, DerefMut},
    sync::OnceLock,
};

use super::PeekExtensions;
use crate::IRecord;
use serde::de::DeserializeOwned;

/// Wraps an IRecord to provide an IRecord interface,
/// while also providing an "instance" field to allow a mutable view
pub struct RunCell<T, R> {
    record: R,
    instance: OnceLock<T>,
}

impl<T, R: IRecord> RunCell<T, R> {
    /// Creates a new run cell
    #[inline]
    pub fn new(record: R) -> Self {
        Self {
            record,
            instance: OnceLock::new(),
        }
    }

    /// Resets the inner state
    /// 
    /// Returns the existing value of T if a value was set
    #[inline]
    pub fn reset(&mut self) -> Option<T> {
        self.instance.take()
    }
}

impl<T, R: IRecord> IRecord for RunCell<T, R> {
    fn ns_chk(&self) -> u64 {
        self.record.ns_chk()
    }

    fn uuid(&self) -> uuid::Uuid {
        self.record.uuid()
    }

    fn opts(&self) -> &crate::Opts {
        self.record.opts()
    }

    fn opts_mut(&mut self) -> Option<&mut crate::Opts> {
        self.record.opts_mut()
    }

    fn bytes(&self) -> &[u8] {
        self.record.bytes()
    }

    fn to_record(&self) -> crate::Record {
        self.record.to_record()
    }
}

impl<T: DeserializeOwned + Default, R: IRecord> AsMut<T> for RunCell<T, R> {
    fn as_mut(&mut self) -> &mut T {
        self.as_ref();

        self.instance
            .get_mut()
            .expect("should exist just initialized")
    }
}

impl<T: DeserializeOwned + Default, R: IRecord> AsRef<T> for RunCell<T, R> {
    fn as_ref(&self) -> &T {
        self.instance
            .get_or_init(|| self.record.peek().to_obj::<T>().unwrap_or_default())
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
