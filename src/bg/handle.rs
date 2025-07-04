use super::Address;
use super::Result;

/// Handle wraps an address and it's functions for a scheme
pub struct Handle {
    /// Address handle has been opened for
    pub(crate) address: Address,
    /// Inner handle
    pub(super) inner: InnerHandle,
}

impl Handle {
    /// Opens a read-only handle to an object
    #[inline]
    pub fn open(address: impl Into<Address>) -> Result<Self> {
        let address = address.into();
        super::open(address)
    }

    /// Opens a read-write handle to an object
    #[inline]
    pub fn open_rw(address: impl Into<Address>) -> Result<Self> {
        let address = address.into();
        super::open_rw(address)
    }

    /// Creates a new location for an object at address
    #[inline]
    pub fn create_new(address: impl Into<Address>) -> Result<Self> {
        let address = address.into();
        super::create_new(address)
    }

    /// Returns the address this handle points to
    #[inline]
    pub fn address(&self) -> &Address {
        &self.address
    }
}

type InnerHandle = Box<dyn ObjectHandle>;

pub(super) trait ObjectHandle:
    std::io::Read + std::io::Write + std::io::Seek + Send + 'static
{
}

impl std::io::Read for Handle {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl std::io::Write for Handle {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl std::io::Seek for Handle {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}
