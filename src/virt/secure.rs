use std::ops::{Deref, DerefMut};
use zeroize::Zeroizing;
use crate::vol::{SharedVolume, Volume};

/// Wrapper for a volume which zeroizes the inner target on drop
pub struct Secure<T, Enc>
where
    T: AsMut<[u8]>,
{
    target: Zeroizing<Volume<T, Enc>>,
}

/// Wrapper for a shared volume which zeroizes the inner target on drop
pub struct SharedSecure<T, Enc>
where
    T: AsMut<[u8]>
{
    target: Zeroizing<SharedVolume<T, Enc>>
}


impl<T: AsMut<[u8]>, Enc> SharedVolume<T, Enc> {
    /// Converts the shared volume to a secure shared volume
    #[inline]
    pub fn to_secure(self) -> SharedSecure<T, Enc> {
        self.into()
    }
}

impl<T: AsMut<[u8]>, Enc> Volume<T, Enc> {
    /// Converts the shared volume to a secure shared volume
    #[inline]
    pub fn to_secure(self) -> Secure<T, Enc> {
        self.into()
    }
}

impl<T, Enc> Deref for SharedSecure<T, Enc> 
where
    T: AsMut<[u8]>
{
    type Target = SharedVolume<T, Enc>;

    fn deref(&self) -> &Self::Target {
        &self.target
    }
}

impl<T, Enc> DerefMut for SharedSecure<T, Enc> 
where
    T: AsMut<[u8]>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.target
    }
}

impl<T, Enc> Deref for Secure<T, Enc> 
where
    T: AsMut<[u8]>
{
    type Target = Volume<T, Enc>;

    fn deref(&self) -> &Self::Target {
        &self.target
    }
}

impl<T, Enc> DerefMut for Secure<T, Enc> 
where
    T: AsMut<[u8]>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.target
    }
}

impl<T, Enc> From<SharedVolume<T, Enc>> for SharedSecure<T, Enc> 
where
    T: AsMut<[u8]>
{
    fn from(value: SharedVolume<T, Enc>) -> Self {
        SharedSecure { target: Zeroizing::new(value) }
    }
}

impl<T, Enc> From<Volume<T, Enc>> for Secure<T, Enc>
where
    T: AsMut<[u8]>
{
    fn from(value: Volume<T, Enc>) -> Self {
        Secure { target: Zeroizing::new(value) }
    }
}
