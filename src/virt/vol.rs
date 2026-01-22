use crate::Data;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use futures::{AsyncRead, AsyncSeek, AsyncWrite};
use memmap2::{Mmap, MmapMut, MmapOptions};
use parking_lot::Mutex;
use pin_project_lite::pin_project;
use std::path::{Path, PathBuf};
use std::{
    io::{Read, Write},
    ops::{Deref, DerefMut},
    sync::OnceLock,
};
use tracing::{debug, trace};
use zeroize::Zeroize;

pub const KIB: usize = 2usize.pow(10);
pub const MIB: usize = 2usize.pow(20);
pub const MIB16: usize = MIB * 16;

/// Super trait for types that [`Volume<T>`] can use as it's binary store
pub trait VolumeTarget: AsyncWrite + Unpin + Sync + 'static {
    /// Provides the flush semantics for the volume target
    ///
    /// This enables precise control over flushing behavior, instead of relying
    /// on the default bytes::BufMut::writer adapter, which is always a no-op.
    fn flush(&self) -> std::io::Result<()>;

    /// Returns the "path" for this volume target
    fn path(&self) -> impl AsRef<Path>;

    /// Freezes the volume target to ensure it becomes cloneable and read-only
    fn freeze(self) -> std::io::Result<Data>;

    /// Creates a snapshot of the target from the beginning to end the current pos
    fn snapshot(&self) -> std::io::Result<Bytes>;

    /// Returns a readonly view of the volume target
    fn view<'view>(&'view self) -> &'view [u8];

    /// Returns the current position of the target
    fn pos(&self) -> usize;

    /// Returns the filled section of the volume
    #[inline]
    fn filled<'view>(&'view self) -> &'view [u8] {
        &self.view()[..self.pos()]
    }

    /// Returns the remaining number of available bytes
    fn remaining(&self) -> u64;

    /// Advances the cursor by count
    fn advance(&mut self, count: usize) -> std::io::Result<()>;

    /// Commits the target to it's path setting
    fn commit(&self) -> std::io::Result<()> {
        if self.path().as_ref().as_os_str().is_empty() || self.filled().is_empty() {
            return Ok(()); // TODO: Silent failure okay?
        }

        let commit = self.filled();
        let path: PathBuf = self.path().as_ref().to_path_buf();
        let mut temp_path = path.clone();
        temp_path.set_extension("temp");
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&temp_path)?;
        let result = {
            file.set_len(commit.len() as u64)?;
            let mut dest = unsafe { MmapMut::map_mut(&file)? };
            dest.copy_from_slice(commit);
            dest.flush()
        };

        match result {
            Ok(_) => {}
            Err(err) => {
                std::fs::remove_file(temp_path)?;
                return Err(err);
            }
        }

        let mut old_path = path.clone();
        old_path.set_extension("old");

        /*
            # File Handling
            1) Create TEMP
            err) Delete TEMP
            2) CURRENT -> OLD
            3) TEMP -> CURRENT
            ok) Detele OLD
            error) OLD -> CURRENT
        */
        if path.exists() {
            std::fs::rename(&path, &old_path)?;
        }

        match std::fs::rename(&temp_path, &path) {
            Ok(_) => {
                if old_path.exists() {
                    std::fs::remove_file(&old_path)?;
                }
                debug!("Commiting volume target to `{:?}`", path);
            }
            Err(err) => {
                if old_path.exists() {
                    std::fs::rename(&old_path, &path)?;
                    std::fs::remove_file(&temp_path)?;
                }
                return Err(err);
            }
        }

        Ok(())
    }

    /// Resets the cursor to 0
    /// 
    /// Returns the previous pos
    fn reset_cursor(&mut self) -> usize;
}

/// Type-alias for a memory-mapped volume target
pub type MemoryMappedTarget = CursorTarget<MmapMut>;

/// Type-alias for a readonly memory-mapped volume target
pub type ReadMemoryMappedTarget = CursorTarget<Mmap>;

/// Type-alias for an in-memory volume target
pub type InMemoryTarget = CursorTarget<BytesMut>;

/// (Readonly) Opens an existing volume target backed by a memory-mapped file
///
/// Returns an error if the file could not be opened
#[inline]
pub fn read_mmap_target(path: impl Into<PathBuf>) -> std::io::Result<ReadMemoryMappedTarget> {
    let path = path.into();
    let file = std::fs::OpenOptions::new().read(true).open(&path)?;
    let mmap = unsafe { Mmap::map(&file)? };
    Ok(CursorTarget::new(path, mmap))
}

/// Opens an existing volume target backed by a memory-mapped file
///
/// Returns an error if the file could not be opened
#[inline]
pub fn open_mmap_target(
    path: impl Into<PathBuf>,
    copy_on_write: bool,
) -> std::io::Result<MemoryMappedTarget> {
    let path = path.into();
    let file = std::fs::OpenOptions::new()
        .write(true)
        .read(true)
        .open(&path)?;

    let mmap = unsafe {
        if copy_on_write {
            MmapOptions::new().map_copy(&file)?
        } else {
            MmapMut::map_mut(&file)?
        }
    };
    Ok(CursorTarget::new(path, mmap))
}

/// Creates a new volume target backed by a memory-mapped file
///
/// Returns an error if the file could not be opened or created (if it didn't previously exist)
#[inline]
pub fn new_mmap_target(
    path: impl Into<PathBuf>,
    capacity: u64,
) -> std::io::Result<MemoryMappedTarget> {
    let path = path.into();
    let file = std::fs::OpenOptions::new()
        .write(true)
        .read(true)
        .create(true)
        .open(&path)?;

    file.set_len(capacity)?;

    let mmap = unsafe { MmapMut::map_mut(&file)? };
    Ok(CursorTarget::new(path, mmap))
}

/// Creates a new volume target backed by an anonymous memory-mapped file
///
/// Returns an error if the anonymous memory-mapped file could not be allocated
///
/// Note: The path provided here is purely for identifying the target and will not actually interact with any resource at that path
#[inline]
pub fn new_mmap_anon_target(
    path: impl Into<PathBuf>,
    capacity: usize,
) -> std::io::Result<MemoryMappedTarget> {
    let path = path.into();

    let mmap = MmapMut::map_anon(capacity)?;

    Ok(CursorTarget::new(path, mmap))
}

static BYTES_POOL: OnceLock<Mutex<BytesMut>> = OnceLock::new();

/// Returns a bytes-mut from a shared pool
/// 
/// This pool will reclaim bytes from previous borrows from the pool, so it is
/// well suited for temporary/short-lived small buffers
/// 
/// Pool starts at 8 MiB but will attempt to reserve capacity if the requested buffer
/// length could not be allocated
#[inline]
pub fn pool_bytes_mut(size: usize) -> BytesMut {
    let pool = BYTES_POOL.get_or_init(|| Mutex::new(BytesMut::with_capacity(8 * MIB)));

    let mut pool = pool.lock();
    if pool.capacity() < size {
        if pool.try_reclaim(size) {
            trace!(size, reclaim = true, "pool_bytes_alloc");
        } else {
            trace!(size, reclaim = false, "pool_bytes_alloc");
            pool.reserve(size);
        }
    }

    if pool.len() < size {
        let grow = size * 2;
        trace!(grow, "pool_bytes_grow");
        pool.put_bytes(0, grow);
    }
    pool.split_to(size)
}

/// Creates a new (zeroed) in memory volume target
#[inline]
pub fn new_memory_target(path: impl Into<PathBuf>, capacity: usize) -> InMemoryTarget {
    let path = path.into();

    let bytes = pool_bytes_mut(capacity);

    CursorTarget::new(path, bytes)
}

/// Volume stores a "target" which can read/write bytes
///
/// And an "encoder" which manages state/mapping/decoding
#[derive(Debug)]
pub struct Volume<T, Enc> {
    /// Inner target
    target: T,
    /// Encoder
    encoder: Enc,
}

impl<T, Enc> Volume<T, Enc> {
    /// Returns a mutable reference to the inner target
    #[inline]
    pub fn target_mut(&mut self) -> &mut T {
        &mut self.target
    }

    /// Returns a reference to the inner target
    #[inline]
    pub fn target(&self) -> &T {
        &self.target
    }

    /// Returns a mutable reference to the inner encoder
    #[inline]
    pub fn encoder_mut(&mut self) -> &mut Enc {
        &mut self.encoder
    }

    /// Returns a reference to the inner encoder
    #[inline]
    pub fn encoder(&self) -> &Enc {
        &self.encoder
    }

    /// Returns a mutable reference to the inner parts
    #[inline]
    pub fn parts_mut(&mut self) -> (&mut T, &mut Enc) {
        (&mut self.target, &mut self.encoder)
    }

    /// Returns a reference to the inner parts
    #[inline]
    pub fn parts(&self) -> (&T, &Enc) {
        (&self.target, &self.encoder)
    }

    /// Consumes the volume and returns the parts
    #[inline]
    pub fn into_parts(self) -> (T, Enc) {
        (self.target, self.encoder)
    }

    /// Restores a volumes from parts
    #[inline]
    pub fn from_parts(parts: (T, Enc)) -> Self {
        parts.into()
    }
}

impl<T: AsRef<[u8]> + Sync + Send + 'static, Enc> Deref for Volume<T, Enc> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.target.as_ref()
    }
}

impl<T, Enc> From<(T, Enc)> for Volume<T, Enc> {
    fn from((target, encoder): (T, Enc)) -> Self {
        Self { target, encoder }
    }
}

pin_project! {
    /// Cursor Target **actually** enables [`futures::AsyncWrite`]/[`futures::AsyncRead`]/[`futures::AsyncSeek`]
    /// for types that implement [`AsRef<[u8]>`] and [`AsMut<[u8]>`].
    ///
    /// Used as the backing store for both in-memory and memory-mapped volume targets.
    /// Maintains a manual cursor for read/write/seek coordination across async traits.
    #[derive(Debug)]
    pub struct CursorTarget<T> {
        path: PathBuf,
        pos: usize,
        inner: T,
    }
}

impl<T> CursorTarget<T> {
    /// Returns a new cursor target
    #[inline]
    pub fn new(path: impl Into<PathBuf>, inner: T) -> Self {
        Self {
            path: path.into(),
            pos: 0,
            inner,
        }
    }

    /// Returns the inner parts of the cursor_target
    #[inline]
    pub fn into_parts(self) -> (PathBuf, T, usize) {
        (self.path, self.inner, self.pos)
    }

    /// Returns the remaining capacity of the current target
    #[inline]
    pub fn remaining_capacity(&self) -> usize
    where
        T: AsRef<[u8]>,
    {
        self.inner.as_ref().len().saturating_sub(self.pos)
    }
}

impl VolumeTarget for MemoryMappedTarget {
    #[inline]
    fn flush(&self) -> std::io::Result<()> {
        self.inner.flush()
    }

    #[inline]
    fn path(&self) -> impl AsRef<Path> {
        self.path.as_path()
    }

    #[inline]
    fn freeze(self) -> std::io::Result<Data> {
        let bytes = Bytes::from_owner(self.inner.make_read_only()?);

        Ok(bytes.slice(..self.pos).into())
    }

    #[inline]
    fn view<'view>(&'view self) -> &'view [u8] {
        &self.inner
    }

    #[inline]
    fn remaining(&self) -> u64 {
        self.remaining_capacity() as u64
    }

    #[inline]
    fn advance(&mut self, count: usize) -> std::io::Result<()> {
        self.pos += count;
        Ok(())
    }

    #[inline]
    fn snapshot(&self) -> std::io::Result<Bytes> {
        if self.pos == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Cannot snapshot an empty target",
            ));
        }
        self.inner.flush()?;
        let ptr = self.inner.as_ptr();
        let view = unsafe { std::slice::from_raw_parts(ptr, self.pos) };
        let view = Bytes::from_owner(view);
        Ok(view.into())
    }

    // #[inline]
    // fn copy_from_slice(&mut self, slice: &[u8]) -> std::io::Result<()> {
    //     if self.pos + slice.len() > self.inner.len() {
    //         return Err(std::io::Error::new(
    //             std::io::ErrorKind::InvalidInput,
    //             "Slice exceeds available capacity",
    //         ));
    //     }
    //     let pos = self.pos;
    //     if let Some(copy_to) = self.get_mut(pos..pos + slice.len()) {
    //         copy_to.copy_from_slice(slice);
    //         self.advance(slice.len())?;
    //     }
    //     Ok(())
    // }

    #[inline]
    fn pos(&self) -> usize {
        self.pos
    }
    
    #[inline]
    fn reset_cursor(&mut self) -> usize {
        std::mem::replace(&mut self.pos, 0)
    }
}

unsafe impl BufMut for MemoryMappedTarget {
    fn remaining_mut(&self) -> usize {
        self.len() - self.pos
    }

    unsafe fn advance_mut(&mut self, cnt: usize) {
        self.advance(cnt).ok();
    }

    fn chunk_mut(&mut self) -> &mut bytes::buf::UninitSlice {
        let unfilled = &mut self.inner[self.pos..];
        let len = unfilled.len();
        let ptr = unfilled.as_mut_ptr() as *mut u8;

        // SAFETY: The pointer is valid for `len` bytes because it comes from a
        // slice of that length.
        unsafe { bytes::buf::UninitSlice::from_raw_parts_mut(ptr, len) }
    }
}

unsafe impl BufMut for InMemoryTarget {
    fn remaining_mut(&self) -> usize {
        self.remaining_capacity()
    }

    unsafe fn advance_mut(&mut self, cnt: usize) {
        self.advance(cnt).ok();
    }

    fn chunk_mut(&mut self) -> &mut bytes::buf::UninitSlice {
        let unfilled = &mut self.inner[self.pos..];
        let len = unfilled.len();
        let ptr = unfilled.as_mut_ptr() as *mut u8;

        // SAFETY: The pointer is valid for `len` bytes because it comes from a
        // slice of that length.
        unsafe { bytes::buf::UninitSlice::from_raw_parts_mut(ptr, len) }
    }
}

impl VolumeTarget for InMemoryTarget {
    #[inline]
    fn flush(&self) -> std::io::Result<()> {
        Ok(())
    }

    #[inline]
    fn path(&self) -> impl AsRef<Path> {
        self.path.as_path()
    }

    #[inline]
    fn freeze(self) -> std::io::Result<Data> {
        Ok(self.inner.freeze().slice(..self.pos).into())
    }

    #[inline]
    fn view<'view>(&'view self) -> &'view [u8] {
        &self.inner[..self.pos]
    }

    #[inline]
    fn remaining(&self) -> u64 {
        self.remaining_capacity() as u64
    }

    #[inline]
    fn advance(&mut self, count: usize) -> std::io::Result<()> {
        self.pos += count;
        Ok(())
    }

    #[inline]
    fn snapshot(&self) -> std::io::Result<Bytes> {
        if self.pos == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Cannot snapshot an empty target",
            ));
        }
        let ptr = self.inner.as_ptr();
        let view = unsafe { std::slice::from_raw_parts(ptr, self.pos) };
        let view = Bytes::from_owner(view);
        Ok(view)
    }

    // #[inline]
    // fn copy_from_slice(&mut self, slice: &[u8]) -> std::io::Result<()> {
    //     self.inner.reserve(slice.len());
    //     let pos = self.pos;
    //     if let Some(copy_to) = self.get_mut(pos..pos + slice.len()) {
    //         copy_to.copy_from_slice(slice);
    //         self.advance(slice.len())?;
    //         Ok(())
    //     } else {
    //         unreachable!("")
    //     }
    // }

    #[inline]
    fn pos(&self) -> usize {
        self.pos
    }
    
    #[inline]
    fn reset_cursor(&mut self) -> usize {
        std::mem::replace(&mut self.pos, 0)
    }
}

impl<T: AsMut<[u8]>> Zeroize for CursorTarget<T> {
    fn zeroize(&mut self) {
        self.as_mut().zeroize();
    }
}

impl<T: AsMut<[u8]>, Enc> Zeroize for Volume<T, Enc> {
    fn zeroize(&mut self) {
        self.target_mut().as_mut().zeroize()
    }
}

impl<T: AsMut<[u8]>> AsyncWrite for CursorTarget<T>
where
    Self: VolumeTarget,
{
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let project = self.project();

        if buf.len() + *project.pos > project.inner.as_mut().len() {
            return std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::QuotaExceeded,
                format!(
                    "Did not attempt to write buf, buf len is {}, which would exceed target size, {} > {}",
                    buf.len(),
                    buf.len() + *project.pos,
                    project.inner.as_mut().len()
                ),
            )));
        }

        let mut inner = project.inner.as_mut()[*project.pos as usize..].writer();
        std::task::Poll::Ready(inner.write(buf).inspect(|w| *project.pos += w))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(<Self as VolumeTarget>::flush(&self))
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.poll_flush(cx)
    }
}

impl<T: AsRef<[u8]>> AsyncRead for CursorTarget<T> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let project = self.project();

        let mut inner = project.inner.as_ref()[*project.pos as usize..].reader();
        std::task::Poll::Ready(inner.read(buf).inspect(|r| *project.pos += r))
    }
}

impl<T: AsRef<[u8]>> AsyncSeek for CursorTarget<T> {
    fn poll_seek(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        pos: std::io::SeekFrom,
    ) -> std::task::Poll<std::io::Result<u64>> {
        let project = self.project();

        let mut cursor = std::io::Cursor::new(project.inner.as_ref());
        cursor.set_position(*project.pos as u64);

        let r = std::io::Seek::seek(&mut cursor, pos);

        std::task::Poll::Ready(r.inspect(|p| *project.pos = *p as usize))
    }
}

impl<T: Deref<Target = [u8]>> Deref for CursorTarget<T> {
    type Target = T::Target;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T: DerefMut<Target = [u8]>> DerefMut for CursorTarget<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<T: AsRef<[u8]>> AsRef<[u8]> for CursorTarget<T> {
    fn as_ref(&self) -> &[u8] {
        self.inner.as_ref()
    }
}

impl<T: AsMut<[u8]>> AsMut<[u8]> for CursorTarget<T> {
    fn as_mut(&mut self) -> &mut [u8] {
        self.inner.as_mut()
    }
}

#[cfg(test)]
mod test {
    use crate::{
        IRecord, ToNamespace, VecIndex,
        archive::Entry,
        util::{PeekExtensions, PeekRefExtensions},
    };

    use super::*;
    use futures::AsyncWriteExt;
    use zstd::zstd_safe::WriteBuf;

    #[tokio::test]
    async fn test_writer() {
        let mut writer = new_mmap_target("test_writer.bin", 11).unwrap();

        writer.write_all(b"hello").await.unwrap();
        writer.write_all(b" ").await.unwrap();
        writer.write_all(b"world").await.unwrap();

        assert_eq!(*b"hello world", writer[..11]);

        let frozen = writer.freeze().unwrap();
        assert_eq!(*b"hello world", frozen[..11]);
    }

    #[tokio::test]
    async fn test_writer_error_on_out_of_space() {
        let mut writer = new_mmap_target("test_writer_error_on_out_of_space.bin", 10).unwrap();
        writer.write_all(b"hello").await.unwrap();
        writer.write_all(b" ").await.unwrap();
        assert!(writer.write_all(b"world").await.is_err());
    }

    #[tokio::test]
    async fn test_writer_memory() {
        let mut writer = new_memory_target("test_writer_memory.bin", 11);
        writer.write_all(b"hello").await.unwrap();
        writer.write_all(b" ").await.unwrap();
        writer.write_all(b"world").await.unwrap();
        assert_eq!(*b"hello world", writer[..11]);

        let frozen = writer.freeze().unwrap();
        assert_eq!(*b"hello world", frozen[..11]);
    }

    #[tokio::test]
    async fn test_writer_memor_error_on_out_of_space() {
        let mut writer = new_memory_target("test_writer_memory.bin", 10);
        writer.write_all(b"hello").await.unwrap();
        writer.write_all(b" ").await.unwrap();
        assert!(writer.write_all(b"world").await.is_err());
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_volume_swap_and_snapshot_in_memory() {
        let volume = Volume::new_archive(new_memory_target("<inline>", MIB));

        let ns = ().to_namespace();

        let mut records = vec![];

        records.push(ns.store(
            "test_1",
            &toml::toml! {
                value = "test_1"
            },
        ));

        records.push(ns.store(
            "test_2",
            &toml::toml! {
                value = "test_2"
            },
        ));

        records.push(ns.store(
            "test_3",
            &toml::toml! {
                value = "test_3"
            },
        ));

        records.push(ns.store(
            "test_4",
            &toml::toml! {
                value = "test_4"
            },
        ));

        records.push(ns.store(
            "test_5",
            &toml::toml! {
                value = "test_5"
            },
        ));

        let mut volume = volume
            .archive(records.drain(..).map(|r| Entry::Record(r)).collect())
            .await
            .unwrap();

        let member = volume
            .swap_and_archive(new_memory_target("<inline>", MIB))
            .unwrap();
        let index: VecIndex<_> = member.get_records().unwrap().into();
        assert_eq!(
            "test_1",
            index.lookup((), "test_1").unwrap().peek().in_ref()["value"]
                .str()
                .unwrap()
        );
        assert_eq!(
            "test_2",
            index.lookup((), "test_2").unwrap().peek().in_ref()["value"]
                .str()
                .unwrap()
        );
        assert_eq!(
            "test_3",
            index.lookup((), "test_3").unwrap().peek().in_ref()["value"]
                .str()
                .unwrap()
        );
        assert_eq!(
            "test_4",
            index.lookup((), "test_4").unwrap().peek().in_ref()["value"]
                .str()
                .unwrap()
        );
        assert_eq!(
            "test_5",
            index.lookup((), "test_5").unwrap().peek().in_ref()["value"]
                .str()
                .unwrap()
        );
        assert!(volume.encoder.is_empty());
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_volume_swap_and_snapshot_mmap() {
        let volume = Volume::new_archive(
            new_mmap_target("test_volume_swap_and_snapshot_mmap.0.bin", MIB as u64).unwrap(),
        );

        let ns = ().to_namespace();

        let mut records = vec![];

        records.push(ns.store(
            "test_1",
            &toml::toml! {
                value = "test_1"
            },
        ));

        records.push(ns.store(
            "test_2",
            &toml::toml! {
                value = "test_2"
            },
        ));

        records.push(ns.store(
            "test_3",
            &toml::toml! {
                value = "test_3"
            },
        ));

        records.push(ns.store(
            "test_4",
            &toml::toml! {
                value = "test_4"
            },
        ));

        records.push(ns.store(
            "test_5",
            &toml::toml! {
                value = "test_5"
            },
        ));

        let mut volume = volume
            .archive(records.drain(..).map(|r| Entry::Record(r)).collect())
            .await
            .unwrap();

        let member = volume
            .swap_and_archive(
                new_mmap_target("test_volume_swap_and_snapshot_mmap.1.bin", MIB as u64).unwrap(),
            )
            .unwrap();
        let index: VecIndex<_> = member.get_records().unwrap().into();
        assert_eq!(
            "test_1",
            index.lookup((), "test_1").unwrap().peek().in_ref()["value"]
                .str()
                .unwrap()
        );
        assert_eq!(
            "test_2",
            index.lookup((), "test_2").unwrap().peek().in_ref()["value"]
                .str()
                .unwrap()
        );
        assert_eq!(
            "test_3",
            index.lookup((), "test_3").unwrap().peek().in_ref()["value"]
                .str()
                .unwrap()
        );
        assert_eq!(
            "test_4",
            index.lookup((), "test_4").unwrap().peek().in_ref()["value"]
                .str()
                .unwrap()
        );
        assert_eq!(
            "test_5",
            index.lookup((), "test_5").unwrap().peek().in_ref()["value"]
                .str()
                .unwrap()
        );
        assert!(volume.encoder.is_empty());
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_volume_swap_and_snapshot_mmap_anon() {
        let volume = Volume::new_archive(new_mmap_anon_target("<inline>", MIB).unwrap());

        let ns = ().to_namespace();

        let mut records = vec![];

        records.push(ns.store(
            "test_1",
            &toml::toml! {
                value = "test_1"
            },
        ));

        records.push(ns.store(
            "test_2",
            &toml::toml! {
                value = "test_2"
            },
        ));

        records.push(ns.store(
            "test_3",
            &toml::toml! {
                value = "test_3"
            },
        ));

        records.push(ns.store(
            "test_4",
            &toml::toml! {
                value = "test_4"
            },
        ));

        records.push(ns.store(
            "test_5",
            &toml::toml! {
                value = "test_5"
            },
        ));

        let mut volume = volume
            .archive(records.drain(..).map(|r| Entry::Record(r)).collect())
            .await
            .unwrap();

        let member = volume
            .swap_and_archive(new_mmap_anon_target("<inline>", MIB).unwrap())
            .unwrap();
        let index: VecIndex<_> = member.get_records().unwrap().into();
        assert_eq!(
            "test_1",
            index.lookup((), "test_1").unwrap().peek().in_ref()["value"]
                .str()
                .unwrap()
        );
        assert_eq!(
            "test_2",
            index.lookup((), "test_2").unwrap().peek().in_ref()["value"]
                .str()
                .unwrap()
        );
        assert_eq!(
            "test_3",
            index.lookup((), "test_3").unwrap().peek().in_ref()["value"]
                .str()
                .unwrap()
        );
        assert_eq!(
            "test_4",
            index.lookup((), "test_4").unwrap().peek().in_ref()["value"]
                .str()
                .unwrap()
        );
        assert_eq!(
            "test_5",
            index.lookup((), "test_5").unwrap().peek().in_ref()["value"]
                .str()
                .unwrap()
        );
        assert!(volume.encoder.is_empty());
    }

    #[tokio::test]
    async fn test_snapshot() {
        let mut target = new_mmap_anon_target("<inline>", MIB).unwrap();
        target.write_all(b"hello world").await.unwrap();
        let snapshot = target.snapshot().unwrap();
        assert_eq!(snapshot.as_slice(), b"hello world");

        let mut target = new_memory_target("<inline>", MIB);
        target.write_all(b"hello world").await.unwrap();
        let snapshot = target.snapshot().unwrap();
        assert_eq!(snapshot.as_slice(), b"hello world");
    }

    #[test]
    fn test_commit_skip_on_empty() {
        let vol = new_memory_target("", 0);
        vol.commit().unwrap();
    }

    #[test]
    fn test_commit_from_mmap_anon() {
        let mut vol = new_mmap_anon_target(".test/test_commit_from_mmap_anon.bin", 4096).unwrap();
        vol.put_bytes(b'h', 4096);
        vol.flush().unwrap();
        vol.commit().unwrap();
        let check = open_mmap_target(".test/test_commit_from_mmap_anon.bin", false).unwrap();
        assert!(check.iter().all(|b| *b == b'h'));
        let check = read_mmap_target(".test/test_commit_from_mmap_anon.bin").unwrap();
        assert!(check.iter().all(|b| *b == b'h'))
    }
}
