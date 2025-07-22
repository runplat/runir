use crate::{
    archive::{Entry, TapeEncoder},
    store::ArchiveMember,
    util::Packer,
};
use anyhow::anyhow;
use asynchronous_codec::{FramedWrite, FramedWriteParts};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use futures::{AsyncRead, AsyncSeek, AsyncWrite, SinkExt};
use generic_array::{GenericArray, typenum::U32};
use memmap2::{Mmap, MmapMut};
use parking_lot::{Mutex, RwLock};
use pin_project_lite::pin_project;
use std::{
    io::{Read, Write},
    ops::{Deref, DerefMut},
    sync::OnceLock,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tracing::trace;

use super::{ObjectEncoder, VirtualData, data::BackingData};

pub const KIB: usize = 2usize.pow(10);
pub const MIB: usize = 2usize.pow(20);
pub const MIB16: usize = MIB * 16;

/// Super trait for types that [`Volume<T>`] can use as it's binary store
pub trait VolumeTarget: AsyncWrite + Unpin + Sync + 'static {
    /// Portable "Bytes" type that is safe and cheap to clone and share
    type Bytes: Into<BackingData> + Clone;

    /// Provides the flush semantics for the volume target
    ///
    /// This enables precise control over flushing behavior, instead of relying
    /// on the default bytes::BufMut::writer adapter, which is always a no-op.
    fn flush(&self) -> std::io::Result<()>;

    /// Returns the "path" for this volume target
    fn path(&self) -> impl AsRef<Path>;

    /// Freezes the volume target to ensure it becomes cloneable and read-only
    fn freeze(self) -> std::io::Result<Self::Bytes>;

    /// Returns a readonly view of the volume target
    fn view<'view>(&'view self) -> &'view [u8];

    /// Returns the remaining number of available bytes
    fn remaining(&self) -> u64;

    /// Advances the cursor by count
    fn advance(&mut self, count: usize) -> std::io::Result<()>;
}

/// Type-alias for a memory-mapped volume target
///
/// A memory mapped target is backed by a memory mapped file
pub type MemoryMappedTarget = CursorTarget<MmapMut>;

/// Type-alias for an in-memory volume target
pub type InMemoryTarget = CursorTarget<BytesMut>;

/// Creates a new volume target backed by a memory-mapped file
///
/// Returns an error if the file could not be opened
///
/// Note: If the file previously existed, this will end up truncating that file due to MmapMut semantics
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

fn get_bytes_slice(size: usize) -> BytesMut {
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

    let bytes = get_bytes_slice(capacity);

    CursorTarget::new(path, bytes)
}

/// Volume enables archiving batches of records to a single storage target
pub struct Volume<T, Enc> {
    /// Inner target
    target: T,
    /// Encoder
    encoder: Enc,
}

impl<T: AsRef<[u8]> + Sync + Send + 'static, Enc> Deref for Volume<T, Enc> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.target.as_ref()
    }
}

/// Wrapper over a shared volume
/// 
/// Allows conversion into BackingData
pub struct SharedVolume<T, Enc>(pub(crate) Arc<RwLock<Volume<T, Enc>>>);

impl<T: AsRef<[u8]> + Sync + Send + 'static, Enc: Send + Sync + 'static> Deref
    for SharedVolume<T, Enc>
{
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        match unsafe { self.0.data_ptr().as_ref() } {
            Some(data) => data.deref(),
            None => &[],
        }
    }
}

impl<T: AsRef<[u8]> + Sync + Send + 'static, Enc: Send + Sync + 'static> From<SharedVolume<T, Enc>>
    for BackingData
{
    fn from(value: SharedVolume<T, Enc>) -> Self {
        BackingData(Arc::new(value))
    }
}

impl<T: VolumeTarget + AsMut<[u8]>> Volume<T, ObjectEncoder> {
    /// Returns a volume for storing object bytes
    #[inline]
    pub fn objects(target: T) -> Self {
        Self {
            target,
            encoder: ObjectEncoder::default(),
        }
    }

    /// "Pre-" encode an inline-object
    ///
    /// Returns the index for the inline-object
    ///
    /// Note: This ensures that both sides are in sync and can return useful errors
    #[inline]
    pub fn pre_encode_object_inline(&mut self) -> usize {
        self.encoder.pre_encode_next_inline()
    }

    /// "Pre-" encodes an object
    ///
    /// Returns an error if the target does not have enough capacity for this object;
    ///
    /// Otherwise, returns the index of the object
    #[inline]
    pub fn pre_encode_object(
        &mut self,
        expected: GenericArray<u8, U32>,
        expected_len: u32,
    ) -> crate::Result<usize> {
        let _size_check = self.encoder.total_runtime_size() as u64 + expected_len as u64;
        if _size_check > self.target.remaining() {
            Err(anyhow!("Not enough space to pre-encode object").into())
        } else {
            self.target.advance(expected_len as usize)?;
            Ok(self.encoder.pre_encode_object(expected, expected_len))
        }
    }

    /// Encodes a packed object
    ///
    /// Returns an error if the unpacked object did not match the expected pre-encoded digest, or if the idx returned an inline object
    #[inline]
    pub fn encode_packed_obj<P: Packer>(&mut self, idx: usize, packed: &[u8]) -> crate::Result<()> {
        self.encoder
            .encode_packed_object::<P>(idx, packed, self.target.as_mut())
    }

    /// Returns true if the obj at idx has been marked as unpacked
    #[inline]
    pub fn is_obj_unpacked(&self, idx: usize) -> bool {
        self.encoder.is_unpacked(idx)
    }

    /// View bytes for an object
    #[inline]
    pub fn view_obj(&self, idx: usize) -> crate::Result<&[u8]> {
        match self.encoder.object(idx) {
            Some((offset, len)) => {
                Ok(&self.target.view()[offset as usize..offset as usize + len as usize])
            }
            None => Err(anyhow!("Object {idx} not found").into()),
        }
    }
}

impl<T: VolumeTarget> Volume<T, TapeEncoder> {
    /// Creates a new volume w/ target for archiving
    #[inline]
    pub fn archiver(target: T) -> Self {
        Self {
            target,
            encoder: TapeEncoder::default(),
        }
    }

    /// Encodes a series of Records using [TapeEncoder] and writes them sequentially into the underlying target
    pub async fn archive_batch(self, batch: Vec<Entry>) -> std::io::Result<Self> {
        let mut writer = FramedWrite::new(self.target, self.encoder);

        for r in batch {
            writer.feed(r).await?;
        }

        writer.close().await?;

        let FramedWriteParts {
            io,
            encoder,
            buffer,
            ..
        } = writer.into_parts();

        if !buffer.is_empty() {
            unreachable!("This means that close() did not finish flushing")
        }
        Ok(Self {
            target: io,
            encoder,
        })
    }

    /// Returns the number of entries that have been encoded into this volume
    #[inline]
    pub fn count(&self) -> usize {
        self.encoder.len()
    }

    /// Swaps the inner target and snapshots the state into an index
    #[inline]
    pub fn swap_and_archive(&mut self, next: T) -> std::io::Result<ArchiveMember> {
        let target = std::mem::replace(&mut self.target, next);
        let encoder = std::mem::replace(&mut self.encoder, TapeEncoder::default());

        let to_archive = Self { target, encoder };
        to_archive.to_archive()
    }

    /// Consumes the volume and returns an archive member
    #[inline]
    pub fn to_archive(mut self) -> std::io::Result<ArchiveMember> {
        let target = self.target;
        let path = target.path().as_ref().to_path_buf();

        let target = target.freeze()?;
        let manifest = self.encoder.stamp_manifest();

        let mut records = vec![];
        let entries = manifest.journal_entries()?;
        let count = entries.len();
        for e in entries {
            let data = VirtualData::new(e, target.clone())?;
            if let Some(record) = data.materialize() {
                records.push(record);
            }
        }

        if count != records.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Would have returned partial data",
            ));
        }
        Ok(ArchiveMember::Volume {
            path,
            manifest,
            records,
        })
    }
}

pin_project! {
    /// Cursor Target **actually** enables [`futures::AsyncWrite`]/[`futures::AsyncRead`]/[`futures::AsyncSeek`]
    /// for types that implement [`AsRef<[u8]>`] and [`AsMut<[u8]>`].
    ///
    /// Used as the backing store for both in-memory and memory-mapped volume targets.
    /// Maintains a manual cursor for read/write/seek coordination across async traits.
    pub struct CursorTarget<T> {
        path: PathBuf,
        pos: usize,
        inner: T
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

/// Arc<Mmap> is already frozen, except if the mmap is an anonymous mmap,
/// we aren't able to re_map/re_size the underlying mmap, which means if
/// we didn't use all of the capacity, we need to know where the cutoff should be.
#[derive(Clone)]
pub struct FrozenMmap {
    len: usize,
    inner: Arc<Mmap>,
}

impl Deref for FrozenMmap {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.inner[..self.len]
    }
}

impl VolumeTarget for MemoryMappedTarget {
    type Bytes = FrozenMmap;

    #[inline]
    fn flush(&self) -> std::io::Result<()> {
        self.inner.flush()
    }

    #[inline]
    fn path(&self) -> impl AsRef<Path> {
        self.path.as_path()
    }

    #[inline]
    fn freeze(self) -> std::io::Result<Self::Bytes> {
        Ok(FrozenMmap {
            len: self.pos,
            inner: Arc::new(self.inner.make_read_only()?),
        })
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
}

impl VolumeTarget for InMemoryTarget {
    type Bytes = Bytes;

    #[inline]
    fn flush(&self) -> std::io::Result<()> {
        Ok(())
    }

    #[inline]
    fn path(&self) -> impl AsRef<Path> {
        self.path.as_path()
    }

    #[inline]
    fn freeze(self) -> std::io::Result<Self::Bytes> {
        Ok(self.inner.freeze().slice(..self.pos))
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
        util::{PeekExtensions, PeekRefExtensions},
    };

    use super::*;
    use futures::AsyncWriteExt;

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
        let volume = Volume::archiver(new_memory_target("<inline>", MIB));

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
            .archive_batch(records.drain(..).map(|r| Entry::Record(r)).collect())
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
        let volume = Volume::archiver(
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
            .archive_batch(records.drain(..).map(|r| Entry::Record(r)).collect())
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
        let volume = Volume::archiver(new_mmap_anon_target("<inline>", MIB).unwrap());

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
            .archive_batch(records.drain(..).map(|r| Entry::Record(r)).collect())
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
}
