use std::{ops::Deref, sync::Arc};

use bytes::Bytes;
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use zeroize::Zeroize;

use crate::Data;

use super::vol::Volume;


/// Wrapper over a shared volume
///
/// Allows conversion into Data
pub struct SharedVolume<T, Enc>(Arc<RwLock<Volume<T, Enc>>>);

impl<T, Enc> SharedVolume<T, Enc> {
    /// Returns a read lock guard for the vol
    #[inline]
    pub fn vol(&self) -> RwLockReadGuard<'_, Volume<T, Enc>> {
        self.0.read()
    }

    /// Returns a write lock guard for the vol
    #[inline]
    pub fn vol_mut(&self) -> RwLockWriteGuard<'_, Volume<T, Enc>> {
        self.0.write()
    }

    /// Returns a reference to the inner volume that bypasses the lock
    #[inline]
    pub fn view(&self) -> &Volume<T, Enc> {
        // SAFETY:
        // - The inner volume is backed by a stable memory region, and `.as_ptr()`
        //   returns a valid pointer to the underlying Volume.
        unsafe {
            self.0
                .data_ptr()
                .as_ref()
                .expect("should be a runtime volume")
        }
    }
}

impl<T, Enc> Volume<T, Enc> {
    /// Converts the volume to a shared volume
    #[inline]
    pub fn to_shared(self) -> SharedVolume<T, Enc> {
        self.into()
    }
}

impl<T, Enc> Clone for SharedVolume<T, Enc> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T, Enc> From<Volume<T, Enc>> for SharedVolume<T, Enc> {
    fn from(value: Volume<T, Enc>) -> Self {
        Self(Arc::new(RwLock::new(value)))
    }
}

impl<T: AsRef<[u8]> + Sync + Send + 'static, Enc: Send + Sync + 'static> Deref
    for SharedVolume<T, Enc>
{
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        // SAFETY:
        // - The inner volume is backed by a stable memory region (mmap), and `.as_ptr()`
        //   returns a valid pointer to the underlying Volume.
        match unsafe { self.0.data_ptr().as_ref() } {
            Some(data) => data.deref(),
            None => &[],
        }
    }
}

impl<T: AsRef<[u8]> + Sync + Send + 'static, Enc: Send + Sync + 'static> AsRef<[u8]>
    for SharedVolume<T, Enc>
{
    fn as_ref(&self) -> &[u8] {
        &self
    }
}

impl<T: AsRef<[u8]> + Sync + Send + 'static, Enc: Send + Sync + 'static> From<SharedVolume<T, Enc>>
    for Data
{
    fn from(value: SharedVolume<T, Enc>) -> Self {
        Data::from(Bytes::from_owner(value))
    }
}

impl<T: AsMut<[u8]>, Enc> Zeroize for SharedVolume<T, Enc> {
    fn zeroize(&mut self) {
        self.0.write().target_mut().as_mut().zeroize();
    }
}