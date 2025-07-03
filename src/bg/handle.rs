use super::Address;


/// Handle wraps an address and it's functions for a scheme
pub struct Handle {
    pub(crate) address: Address,
}

impl Handle {
    /// Returns the address this handle points to
    #[inline]
    pub fn address(&self) -> &Address {
        &self.address
    }
}

