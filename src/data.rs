use bytes::Bytes;

/// Enumeration of different data implementations
#[derive(Default, Debug, Clone)]
pub enum Data {
    /// Data has not been set
    #[default]
    Empty,
    /// Data is loaded into memory
    Bytes(Bytes)
}

impl Data {
    /// Returns true if data is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bytes().is_empty()
    }

    /// Returns the len in bytes of data
    #[inline]
    pub fn len(&self) -> usize {
       self.bytes().len()
    }

    /// Returns a slice of the bytes in data
    #[inline]
    pub fn bytes(&self) -> &[u8] {
        match self {
            Data::Empty => &[],
            Data::Bytes(bytes) => {
                &bytes
            },
        }
    }
}
