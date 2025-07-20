use std::{
    cell::RefCell,
    io::{Cursor, Read},
    marker::PhantomData,
};

use anyhow::anyhow;
use bytes::{BufMut, Bytes, BytesMut};
use generic_array::{GenericArray, typenum::U32};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    IRecord, Record,
    virt::ObjectEncoder,
    vol::{MemoryMappedTarget, Volume, new_mmap_anon_target},
};

use super::{Peek, PeekExtensions};

/// Type-alias for the default multi-root record construct
///
/// - Any content over 128 bytes will be compressed
/// - The default compression level will be used
pub type Container = MultiRoot<GenericPacker<128, 0>>;

const EMPTY_LABELS: Option<()> = None::<()>;

#[inline]
const fn empty_labels() -> Option<impl Serialize> {
    EMPTY_LABELS
}

/// A multi-root record evolves a record w/ a single root, and provides
/// layers of extensions under the record to enable additional features.
///
/// Each additional layer is maintained by a system layer that is inserted when the build has completed,
/// and maps all lower layers w/ any identifying information.
///
/// The format is roughly,
///
/// [0] <-- primary root, can be an object or content
/// [1] <-- system root
/// [N] <-- any number of additional roots
///
/// ### "system" layer
///
/// The system layer is a vector where each entry is a descriptor containing information on each layer. (Including the system layer)
pub struct MultiRoot<P> {
    state: State,
    _p: PhantomData<P>,
}

enum State {
    /// Build state allows new layers to be pushed
    Build {
        /// Root record of the container
        root: Record,
        /// Internal serializer
        ser: flexbuffers::FlexbufferSerializer,
        /// Internal working buffer
        buffer: BytesMut,
        /// Layers being built into this container
        layers: Vec<(BuildDescriptor, BytesMut)>,
    },
    /// Read-only state collapses all layers into a single record for read-only
    Read {
        /// Record that contains all layers
        record: Record,
    },
    /// Run state mounts a read-only record w/ a runtime volume to handle unpacking of any packed layers
    Run {
        /// Record that contains all layers
        record: Record,
        /// Runtime volume that is able to store any layers that require additional space for unpacking
        vol: RefCell<RuntimeVolume>,
    },
}

impl<P: Packer> MultiRoot<P> {
    /// Creates a read-only multi-root record
    ///
    /// Returns an error if the record is not a Multi Record
    #[inline]
    pub fn read(record: &Record) -> crate::Result<Self> {
        if !record.opts().is_multi() {
            return Err(anyhow!("Cannot read a non-multi root record as a Container").into());
        }

        Ok(Self {
            state: State::Read {
                record: record.clone(),
            },
            _p: PhantomData::default(),
        })
    }

    /// Creates a new multi-root record in build mode w/ a root record
    #[inline]
    pub fn build(root: Record) -> Self {
        let container = Self {
            state: State::Build {
                root,
                ser: flexbuffers::FlexbufferSerializer::new(),
                buffer: BytesMut::new(),
                layers: vec![],
            },
            _p: PhantomData::default(),
        };
        container
    }

    /// Pushes a content based layer
    ///
    /// Returns an error if the container is read-only
    #[inline]
    pub fn push_content(&mut self, content: &[u8]) -> crate::Result<()> {
        self._push_content(content, empty_labels())
    }

    /// Pushes a content based layer w/ labels
    ///
    /// Returns an error if the container is read-only
    #[inline]
    pub fn push_content_with(
        &mut self,
        content: &[u8],
        labels: impl Serialize,
    ) -> crate::Result<()> {
        self._push_content(content, Some(labels))
    }

    #[inline]
    fn _push_content(
        &mut self,
        content: &[u8],
        labels: Option<impl Serialize>,
    ) -> crate::Result<()> {
        let mut digest = Sha256::new();
        digest.update(content);
        match &mut self.state {
            State::Build { buffer, .. } => {
                P::pack_bytes(content, buffer)?;
            }
            _ => {}
        }

        self.push(content.len(), digest.finalize().into(), labels, false)
    }

    /// Pushes an object based layer
    ///
    /// Returns an error if the container is read-only
    #[inline]
    pub fn push_object<T: Serialize>(&mut self, obj: &T) -> crate::Result<()> {
        self._push_object(obj, empty_labels())
    }

    /// Pushes an object based layer
    ///
    /// Returns an error if the container is read-only
    #[inline]
    pub fn push_object_with<T: Serialize>(
        &mut self,
        obj: &T,
        labels: impl Serialize,
    ) -> crate::Result<()> {
        self._push_object(obj, Some(labels))
    }

    /// Pushes an object based layer optionally w/ labels
    ///
    /// Returns an error if the container is read-only
    #[inline]
    fn _push_object<T: Serialize>(
        &mut self,
        obj: &T,
        labels: Option<impl Serialize>,
    ) -> crate::Result<()> {
        let mut content = Sha256::new();
        let content_len;
        match &mut self.state {
            State::Build { ser, buffer, .. } => {
                ser.reset();

                P::pack_object(obj, ser, buffer)?;

                content.update(ser.view());

                content_len = ser.view().len();
            }
            State::Read { .. } | State::Run { .. } => {
                return Err(anyhow!("Cannot push content to a read-only container").into());
            }
        }

        self.push(content_len, content.finalize().into(), labels, true)
    }

    /// Fetches layer content and puts it into buf
    ///
    /// If the content was packed, will unpack the content before putting it into buf
    #[inline]
    pub fn fetch_content(&self, layer: usize, buf: &mut BytesMut) -> crate::Result<()> {
        match &self.state {
            State::Build { root, layers, .. } => {
                if layer == 0 {
                    buf.put(root.bytes());
                    Ok(())
                } else if layer == 1 {
                    Err(anyhow!("Cannot return bytes for system layer in Build state").into())
                } else {
                    let (desc, packed) = layers
                        .get(layer - 2)
                        .map(Ok::<_, crate::Error>)
                        .unwrap_or_else(|| Err(anyhow!("Layer does not exist").into()))?;

                    if desc.is_packed() {
                        buf.reserve(desc.runtime_size() as usize);

                        let split_off = buf.len();
                        buf.put_bytes(0, desc.runtime_size() as usize);
                        let mut dest = buf.split_off(split_off);

                        P::unpack(&packed, &mut dest)?;
                        buf.unsplit(dest);
                    } else {
                        buf.put(packed.as_ref());
                    }
                    Ok(())
                }
            }
            State::Read { record } | State::Run { record, .. } => {
                let desc = record.layer_desc(layer)?;
                if let Some(packed) = record.layer(layer) {
                    if desc.is_packed() {
                        buf.reserve(desc.runtime_size() as usize);

                        let split_off = buf.len();
                        buf.put_bytes(0, desc.runtime_size() as usize);
                        let mut dest = buf.split_off(split_off);

                        P::unpack(packed, &mut dest)?;
                        buf.unsplit(dest);
                    } else {
                        buf.put(packed);
                    }
                }

                Ok(())
            }
        }
    }

    /// Returns bytes for a layer
    ///
    /// Returns an error if the layer could not be found
    ///
    /// Note: If the layer is packed, this function does not unpack the layer
    #[inline]
    pub fn bytes(&self, layer: usize) -> crate::Result<&[u8]> {
        match &self.state {
            State::Build { root, layers, .. } => {
                if layer == 0 {
                    Ok(root.bytes())
                } else if layer == 1 {
                    Err(anyhow!("Cannot return bytes for system layer in Build state").into())
                } else {
                    layers
                        .get(layer - 2)
                        .map(|l| l.1.as_ref())
                        .map(Ok)
                        .unwrap_or_else(|| Err(anyhow!("Layer does not exist").into()))
                }
            }
            State::Read { record } | State::Run { record, .. } => match record.layer(layer) {
                Some(layer) => Ok(layer),
                None => Err(anyhow!("Layer {layer} does not exist").into()),
            },
        }
    }

    /// Returns a reader for a stored layer
    #[inline]
    pub fn reader(&self, layer: usize) -> crate::Result<impl Read> {
        let bytes = self.bytes(layer)?;
        let desc = self
            .layer_desc(layer)
            .map(Ok)
            .unwrap_or_else(|| Err(anyhow!("Could not find layer")))?;
        if desc.is_packed() {
            P::decoder(bytes).map(either::Either::Left)
        } else {
            Ok(either::Either::Right(Cursor::new(bytes)))
        }
    }

    /// Returns the object from a layer
    ///
    /// Note: layer idx is not transformed, so it must be the actual idx of the desired layer
    ///
    /// Ex: Self::object(0) is the same as Self::peek()
    ///
    /// Note: This only supports inline, non-packed objects
    #[inline]
    pub fn object(&self, layer: usize) -> impl PeekExtensions {
        self.try_object(layer).ok()
    }

    /// Returns an object at a layer
    ///
    /// Returns an error if unable to access object at layer
    ///
    /// Note: This only supports inline, non-packed objects
    #[inline]
    pub fn try_object(&self, layer: usize) -> crate::Result<Peek> {
        match &self.state {
            State::Build { root, layers, .. } => {
                if layer == 0 {
                    return if let Some(peek) = root.peek().val() {
                        Ok(peek)
                    } else {
                        Err(anyhow!("Root is not an object").into())
                    };
                }

                if layer == 1 {
                    return Err(anyhow!("Cannot access system layer in build state").into());
                }

                layers
                    .get(layer - 2)
                    .map(|l| Ok(Peek::from(flexbuffers::Reader::get_root(l.1.as_ref())?)))
                    .unwrap_or(Err(anyhow!("Layer not found").into()))
            }
            State::Read { record } => Ok(flexbuffers::Reader::get_root(record.bytes())
                .and_then(|root| root.as_vector().idx(layer).get_blob())
                .and_then(|obj| Ok(Peek::from(flexbuffers::Reader::get_root(obj.0)?)))?),
            State::Run { record, vol } => {
                if let Some(desc) = self.layer_desc(layer) {
                    if !desc.is_object() {
                        return Err(anyhow!("Layer is not an object").into());
                    }

                    if desc.is_packed() {
                        if !vol.borrow().is_obj_unpacked(layer) {
                            self.unpack(layer, desc)?;
                        }

                        // SAFETY:
                        // - `vol` is backed by a stable memory region (mmap), and `.as_ptr()`
                        //   returns a valid pointer to the underlying RuntimeVolume.
                        // - We ensure that any mutable borrow (e.g., `encode_packed_obj`) occurs
                        //   strictly *before* this call, and is fully dropped prior to dereferencing.
                        // - This block is only reached if the layer is already unpacked (or has just
                        //   been unpacked), so the pointer is valid and point
                        let obj =
                            unsafe { vol.as_ptr().as_ref().expect("should be a runtime volume") }
                                .view_obj(layer)?;

                        Ok(Peek::from(flexbuffers::Reader::get_root(obj)?))
                    } else {
                        if let Some(inline) = record.layer(layer) {
                            Ok(Peek::from(flexbuffers::Reader::get_root(inline)?))
                        } else {
                            Err(anyhow!("Record does not have packed layer data").into())
                        }
                    }
                } else {
                    return Err(anyhow!("Layer is unknown to record").into());
                }
            }
        }
    }

    /// Transitions the internal multi-root state to read-only
    ///
    /// Returns an error if the current state is already in read-only mode
    #[inline]
    pub fn to_read_only(self) -> crate::Result<Self> {
        match self.state {
            State::Build { root, layers, .. } => {
                // 1) Build system layer
                let mut builder = flexbuffers::Builder::default();
                let mut sys = builder.start_vector();

                sys.start_map().end_map(); // Account for root layer
                sys.start_map().end_map(); // Account for sys layer

                for (desc, ..) in layers.iter() {
                    let mut layer = sys.start_map();
                    layer.push("is_object", desc.is_object());
                    layer.push("is_packed", desc.is_packed());
                    layer.push("labels", flexbuffers::Blob(desc.labels.as_slice()));
                    layer.push("content", flexbuffers::Blob(desc.content().as_slice()));
                    layer.push(
                        "distribution",
                        flexbuffers::Blob(desc.distribution().as_slice()),
                    );
                    layer.push("runtime_size", desc.runtime_size());
                }
                sys.end_vector();

                let system_layer = builder.take_buffer();

                let mut multi_root = builder.start_vector();
                multi_root.push(flexbuffers::Blob(root.bytes()));
                multi_root.push(flexbuffers::Blob(system_layer.as_slice()));
                for (.., bytes) in layers.iter() {
                    multi_root.push(flexbuffers::Blob(bytes.as_ref()));
                }
                multi_root.end_vector();

                let mut record = root.stage(Bytes::copy_from_slice(builder.view()))?;
                record
                    .opts_mut()
                    .expect("should always be able to mutate options from a full record")
                    .set_multi_root_storage(true);

                Ok(Self {
                    state: State::Read { record },
                    _p: PhantomData,
                })
            }
            State::Read { .. } => Ok(self),
            State::Run { record, .. } => Ok(Self {
                state: State::Read { record },
                _p: PhantomData,
            }),
        }
    }

    /// Returns a descriptor for a layer
    #[inline]
    pub fn layer_desc(&self, layer: usize) -> Option<impl ILayerDescriptor> {
        match &self.state {
            State::Build { layers, .. } => {
                if layer == 0 {
                    return None;
                }

                if layer == 1 {
                    return None;
                }

                layers.get(layer - 2).map(|v| LayerDesc::Build(&v.0))
            }
            State::Read { record } | State::Run { record, .. } => record.layer_desc(layer).ok(),
        }
    }

    /// Finds all layers that match a predicate based on layer labels
    #[inline]
    pub fn find_all_layer_by_labels(
        &self,
        find: impl Fn(&Peek) -> bool,
    ) -> crate::Result<Vec<usize>> {
        Ok(match &self.state {
            State::Build { layers, .. } => layers
                .iter()
                .enumerate()
                .filter_map(|(idx, l)| l.0.labels().map(|l| (idx, l)))
                .filter(|(_, l)| find(l))
                .map(|p| p.0 + 2)
                .collect(),
            State::Read { record } | State::Run { record, .. } => record
                .iter_layer_desc()?
                .enumerate()
                .skip(2) // System/Root layers never have labels
                .filter(|(.., l)| l.labels().map(|l| find(&l)).unwrap_or_default())
                .map(|p| p.0)
                .collect(),
        })
    }

    /// Prepares the multi-root for runtime
    ///
    /// Note: This state-transition is only required when a multi-root record has packed layers
    #[inline]
    pub fn to_run(mut self) -> crate::Result<Self> {
        if matches!(self.state, State::Build { .. }) {
            self = self.to_read_only()?;
        }

        if matches!(self.state, State::Run { .. }) {
            return Ok(self);
        }

        match self.state {
            State::Read { record } => {
                // 1) Figure out how much runtime space we'll need for any packed layers
                let required_size: u64 = record
                    .iter_layer_desc()?
                    .filter(|l| l.is_packed())
                    .map(|l| l.runtime_size())
                    .sum();

                if required_size == 0 {
                    return Err(anyhow!("Multi-root record is completely inline").into());
                }

                // Not specifically required, but good hygiene
                let alignment = required_size % 512;
                let required_size = required_size + (512 - alignment);

                let target = new_mmap_anon_target(
                    format!("objects/{}", record.uuid().simple()),
                    required_size as usize,
                )?;

                let mut objects = RuntimeVolume::objects(target);

                for (idx, desc) in record.iter_layer_desc()?.enumerate() {
                    if idx == 0 || idx == 1 {
                        objects.pre_encode_object_inline();
                        continue;
                    }

                    if let Some(layer) = record.layer(idx) {
                        let actual = Sha256::digest(layer);
                        if actual.as_slice() != desc.distribution().as_slice() {
                            return Err(anyhow!(
                                "Layer {idx} digest did not match, expected: {}, actual: {}",
                                hex::encode(desc.distribution()),
                                hex::encode(actual)
                            )
                            .into());
                        }
                    } else {
                        return Err(anyhow!("Record data is incomplete, cannot").into());
                    }

                    if desc.is_packed() && desc.is_object() {
                        let _idx = objects
                            .pre_encode_object(*desc.content(), desc.runtime_size() as u32)?;
                        debug_assert_eq!(idx, _idx);
                    } else {
                        let _idx = objects.pre_encode_object_inline();
                        debug_assert_eq!(idx, _idx);
                    }
                }

                Ok(Self {
                    state: State::Run {
                        record,
                        vol: RefCell::new(objects),
                    },
                    _p: PhantomData,
                })
            }
            State::Run { .. } => unreachable!("Must have returned early"),
            State::Build { .. } => {
                unreachable!("Must convert to this state, or return an error before this arm")
            }
        }
    }

    /// Unpacks all packed object layers
    ///
    /// If any layer was already unpacked, it will be skipped (however it's packed digest will still be checked)
    ///
    /// Returns an error if to_run was not called first before calling this function
    ///
    /// Note: Since unpacking is intended to be lazily done, this function maintains the use of interior-mutability
    #[inline]
    pub fn unpack_all(&self) -> crate::Result<()> {
        match &self.state {
            State::Run { record, .. } => {
                for (idx, desc) in record.iter_layer_desc()?.enumerate() {
                    if idx == 0 || idx == 1 {
                        continue;
                    }

                    if desc.is_packed() && desc.is_object() {
                        self.unpack(idx, desc)?;
                    }
                }
                Ok(())
            }
            _ => Err(anyhow!("Can only unpack all layers in a run state").into()),
        }
    }

    #[inline]
    fn unpack(&self, layer: usize, desc: impl ILayerDescriptor) -> crate::Result<()> {
        match &self.state {
            State::Run { record, vol } => {
                if let Some(packed) = record.layer(layer) {
                    let packed_digest = Sha256::digest(packed);
                    if packed_digest.as_slice() != desc.distribution().as_slice() {
                        return Err(anyhow!(
                            "Packed data does not match recorded distribution digest"
                        )
                        .into());
                    }

                    vol.borrow_mut().encode_packed_obj::<P>(layer, packed)?;
                    Ok(())
                } else {
                    return Err(anyhow!("Record does not have packed layer data").into());
                }
            }
            _ => Err(anyhow!("Can only unpack layers in Run mode").into()),
        }
    }

    fn push(
        &mut self,
        len: usize,
        digest: [u8; 32],
        labels: Option<impl Serialize>,
        is_object: bool,
    ) -> crate::Result<()> {
        match &mut self.state {
            State::Build {
                buffer,
                layers,
                ser,
                ..
            } => {
                let next = buffer.split();

                ser.reset();
                if let Some(labels) = labels {
                    labels.serialize(&mut *ser)?;
                }

                if next.len() != len {
                    let mut packed = Sha256::new();
                    packed.update(next.as_ref());

                    layers.push((
                        BuildDescriptor {
                            content: digest,
                            packed: Some(Packed {
                                digest: packed.finalize().into(),
                                unpacked: len as u64,
                            }),
                            len: next.len() as u64,
                            labels: ser.take_buffer(),
                            is_object,
                        },
                        next,
                    ));
                } else {
                    layers.push((
                        BuildDescriptor {
                            content: digest,
                            packed: None,
                            len: next.len() as u64,
                            labels: ser.take_buffer(),
                            is_object,
                        },
                        next,
                    ));
                }
            }
            State::Read { .. } | State::Run { .. } => {
                return Err(anyhow!("Cannot push content to a read-only container").into());
            }
        }

        Ok(())
    }
}

/// Generic Packer trait implementation
pub struct GenericPacker<const SIZE_THRESHOLD: usize, const COMPRESSION_LEVEL: i32>;

impl<const SIZE_THRESHOLD: usize, const COMPRESSION_LEVEL: i32> Packer
    for GenericPacker<SIZE_THRESHOLD, COMPRESSION_LEVEL>
{
    fn pack_bytes(bytes: &[u8], buffer: &mut BytesMut) -> crate::Result<()> {
        if bytes.len() < SIZE_THRESHOLD {
            buffer.reserve(bytes.len());
            buffer.put(bytes);
            Ok(())
        } else {
            let bytes = zstd::encode_all(bytes, COMPRESSION_LEVEL)?;
            buffer.reserve(bytes.len());
            buffer.put(bytes.as_slice());
            Ok(())
        }
    }

    fn unpack<'u>(from: &[u8], to: &'u mut [u8]) -> crate::Result<()> {
        if from.len() == to.len() {
            to.copy_from_slice(from);
            Ok(())
        } else {
            let decoded = zstd::decode_all(from)?;
            to.copy_from_slice(&decoded);
            Ok(())
        }
    }

    fn decoder(from: &[u8]) -> crate::Result<impl Read> {
        Ok(zstd::Decoder::new(std::io::Cursor::new(from))?)
    }
}

/// Packer handles packing and unpacking bytes
pub trait Packer: private::Sealed {
    /// Pack bytes into a layer
    fn pack_bytes(bytes: &[u8], buffer: &mut BytesMut) -> crate::Result<()>;

    /// Unpack bytes from a layer
    fn unpack<'u>(from: &[u8], to: &'u mut [u8]) -> crate::Result<()>;

    /// Returns a decoder for bytes
    fn decoder(from: &[u8]) -> crate::Result<impl Read>;

    /// Pack an object into a layer
    fn pack_object<T: Serialize>(
        obj: &T,
        ser: &mut flexbuffers::FlexbufferSerializer,
        buffer: &mut BytesMut,
    ) -> crate::Result<()> {
        obj.serialize(&mut *ser)?;
        Self::pack_bytes(ser.view(), buffer)
    }
}

mod private {
    use super::GenericPacker;

    pub trait Sealed {}

    impl<const COMPRESSION_THRESHOLD: usize, const COMPRESSION_LEVEL: i32> Sealed
        for GenericPacker<COMPRESSION_THRESHOLD, COMPRESSION_LEVEL>
    {
    }
}

trait IContainer<'p>
where
    Self: 'p,
{
    fn layer(&'p self, idx: usize) -> Option<&'p [u8]>;

    fn system(&'p self) -> crate::Result<Peek<'p>>;

    fn iter_layer_desc(&self) -> crate::Result<impl Iterator<Item = impl ILayerDescriptor>>;

    fn layer_desc(&'p self, idx: usize) -> crate::Result<LayerDesc<'p>> {
        Ok(LayerDesc::Read(self.system()?.as_vector().idx(idx).into()))
    }
}

impl<'p> IContainer<'p> for Record {
    fn layer(&'p self, idx: usize) -> Option<&'p [u8]> {
        if !self.opts().is_multi() {
            return None;
        }

        let root = flexbuffers::Reader::get_root(self.bytes()).ok()?;

        let blob = root.as_vector().idx(idx).get_blob().ok()?;

        Some(blob.0)
    }

    fn iter_layer_desc(&self) -> crate::Result<impl Iterator<Item = impl ILayerDescriptor>> {
        if !self.opts().is_multi() {
            return Err(anyhow!("Record is not a multi-root record").into());
        }

        let system = self.system()?;

        if let Some(iter) = system.iter() {
            Ok(iter.map(|p| LayerDesc::Read(p)))
        } else {
            Err(anyhow!("System layer was in an unexpected format").into())
        }
    }

    fn system(&'p self) -> crate::Result<Peek<'p>> {
        Ok(flexbuffers::Reader::get_root(self.bytes())
            .and_then(|root| root.as_vector().idx(1).get_blob())
            .and_then(|obj| Ok(Peek::from(flexbuffers::Reader::get_root(obj.0)?)))?)
    }
}

/// Trait for types that are able to describe a layer
pub trait ILayerDescriptor {
    /// Total required-size at runtime for this layer
    fn runtime_size(&self) -> u64;

    /// Returns the "distribution" digest of this layer
    fn distribution(&self) -> &GenericArray<u8, U32>;

    /// Returns the "content" digest of this layer
    fn content(&self) -> &GenericArray<u8, U32>;

    /// Returns true if the layer has been packed
    fn is_packed(&self) -> bool;

    /// Returns true if the layer is an object
    fn is_object(&self) -> bool;

    /// Returns labels stored w/ this layer
    fn labels<'a: 'b, 'b>(&'a self) -> Option<Peek<'b>>;
}

enum LayerDesc<'p> {
    Build(&'p BuildDescriptor),
    Read(Peek<'p>),
}

impl<'p> ILayerDescriptor for LayerDesc<'p> {
    fn runtime_size(&self) -> u64 {
        match self {
            LayerDesc::Build(build_descriptor) => build_descriptor.runtime_size(),
            LayerDesc::Read(peek) => peek.runtime_size(),
        }
    }

    fn distribution(&self) -> &GenericArray<u8, U32> {
        match self {
            LayerDesc::Build(build_descriptor) => build_descriptor.distribution(),
            LayerDesc::Read(peek) => peek.distribution(),
        }
    }

    fn content(&self) -> &GenericArray<u8, U32> {
        match self {
            LayerDesc::Build(build_descriptor) => build_descriptor.content(),
            LayerDesc::Read(peek) => peek.content(),
        }
    }

    fn is_packed(&self) -> bool {
        match self {
            LayerDesc::Build(build_descriptor) => build_descriptor.is_packed(),
            LayerDesc::Read(peek) => peek.is_packed(),
        }
    }

    fn is_object(&self) -> bool {
        match self {
            LayerDesc::Build(build_descriptor) => build_descriptor.is_object(),
            LayerDesc::Read(peek) => peek.is_object(),
        }
    }

    fn labels<'a: 'b, 'b>(&'a self) -> Option<Peek<'b>> {
        match self {
            LayerDesc::Build(build_descriptor) => build_descriptor.labels(),
            LayerDesc::Read(peek) => peek.labels(),
        }
    }
}

#[derive(Clone)]
struct BuildDescriptor {
    /// Content digest of the layer
    ///
    /// The content digest is the digest of the bytes of the provided data
    content: [u8; 32],
    /// Packed digest of the layer
    ///
    /// If set, indicates that the stored data is "packed", and the digest should be the digest
    /// of the "packed" bytes, while the content digest should be the digest of the "unpacked"
    /// bytes
    packed: Option<Packed>,
    /// Length of the stored layer
    len: u64,
    /// List of labels that describe the layer
    labels: Vec<u8>,
    /// True if the layer being build is an object
    is_object: bool,
}

const EMPTY_DIGEST: GenericArray<u8, U32> = GenericArray::from_array([0; 32]);

impl<'p> ILayerDescriptor for Peek<'p> {
    fn runtime_size(&self) -> u64 {
        self.at("runtime_size").u64().unwrap_or_default()
    }

    fn distribution(&self) -> &GenericArray<u8, U32> {
        self.at("distribution")
            .blob()
            .map(|b| GenericArray::<u8, U32>::from_slice(b))
            .unwrap_or(&EMPTY_DIGEST)
    }

    fn content(&self) -> &GenericArray<u8, U32> {
        self.at("content")
            .blob()
            .map(|b| GenericArray::<u8, U32>::from_slice(b))
            .unwrap_or(&EMPTY_DIGEST)
    }

    fn is_packed(&self) -> bool {
        self.at("is_packed").bool().unwrap_or_default()
    }

    fn is_object(&self) -> bool {
        self.at("is_object").bool().unwrap_or_default()
    }

    fn labels<'a: 'b, 'b>(&'a self) -> Option<Peek<'b>> {
        let labels = self.at("labels").blob()?;
        Some(Peek::from(flexbuffers::Reader::get_root(labels).ok()?))
    }
}

impl ILayerDescriptor for BuildDescriptor {
    /// Total required-size at runtime for this layer
    #[inline]
    fn runtime_size(&self) -> u64 {
        self.packed.as_ref().map(|p| p.unpacked).unwrap_or(self.len)
    }

    /// Returns the "distribution" digest of this layer
    #[inline]
    fn distribution(&self) -> &GenericArray<u8, U32> {
        self.packed
            .as_ref()
            .map(|p| &p.digest)
            .unwrap_or(&self.content)
            .into()
    }

    /// Returns the "content" digest of this layer
    #[inline]
    fn content(&self) -> &GenericArray<u8, U32> {
        (&self.content).into()
    }

    /// Returns true if the layer has been packed
    #[inline]
    fn is_packed(&self) -> bool {
        self.packed.is_some()
    }

    /// Returns true if the layer is an object
    #[inline]
    fn is_object(&self) -> bool {
        self.is_object
    }

    /// Returns labels for the this layer
    #[inline]
    fn labels<'a: 'b, 'b>(&'a self) -> Option<Peek<'b>> {
        flexbuffers::Reader::get_root(self.labels.as_slice())
            .ok()
            .map(Peek::from)
    }
}

#[derive(Clone)]
struct Packed {
    /// Digest of the packed content
    digest: [u8; 32],
    /// Length of the unpacked layer
    unpacked: u64,
}

impl<P> IRecord for MultiRoot<P> {
    fn ns_chk(&self) -> u64 {
        match &self.state {
            State::Build { root, .. } => root.ns_chk(),
            State::Read { record } | State::Run { record, .. } => record.ns_chk(),
        }
    }

    fn uuid(&self) -> uuid::Uuid {
        match &self.state {
            State::Build { root, .. } => root.uuid(),
            State::Read { record } | State::Run { record, .. } => record.uuid(),
        }
    }

    fn opts(&self) -> &crate::Opts {
        match &self.state {
            State::Build { root, .. } => root.opts(),
            State::Read { record } | State::Run { record, .. } => record.opts(),
        }
    }

    fn bytes(&self) -> &[u8] {
        match &self.state {
            State::Build { root, .. } => root.bytes(),
            State::Read { record } | State::Run { record, .. } => {
                record.layer(0).expect("should always have a layer 0")
            }
        }
    }

    fn to_record(&self) -> Record {
        match &self.state {
            State::Build { root, .. } => root.to_record(),
            State::Read { record } | State::Run { record, .. } => record.to_record(),
        }
    }

    fn opts_mut(&mut self) -> Option<&mut crate::Opts> {
        match &mut self.state {
            State::Build { root, .. } => root.opts_mut(),
            State::Read { record } | State::Run { record, .. } => record.opts_mut(),
        }
    }
}

/// Type-alias for a "Runtime" volume
type RuntimeVolume = Volume<MemoryMappedTarget, ObjectEncoder>;

#[cfg(test)]
mod test {
    use std::io::Read;

    use super::{Container, GenericPacker};
    use crate::{
        Namespace,
        util::{
            PeekExtensions,
            container::{ILayerDescriptor, MultiRoot},
        },
    };
    use bytes::{BufMut, BytesMut};
    use toml::toml;

    #[test]
    fn test_container() {
        let ns = Namespace::ephemeral();

        let mut container = Container::build(ns.commit("test", b"hello world".as_slice()));

        container
            .push_content_with(
                b"good bye world".as_slice(),
                toml! {
                    name = "test"
                },
            )
            .unwrap();

        container
            .push_object_with(
                &toml! {
                    message = "hello world"
                },
                toml! {
                    name = "test2"
                },
            )
            .unwrap();

        let obj = container.try_object(3).unwrap();

        assert_eq!(
            "hello world",
            obj.to_obj::<toml::Value>().unwrap()["message"]
                .as_str()
                .unwrap()
        );

        assert_eq!(
            "test2",
            container
                .layer_desc(3)
                .unwrap()
                .labels()
                .at("name")
                .str()
                .unwrap()
        );

        assert_eq!(
            "test",
            container
                .layer_desc(2)
                .unwrap()
                .labels()
                .at("name")
                .str()
                .unwrap()
        );

        let container = container.to_read_only().unwrap();
        let obj = container.try_object(3).unwrap();

        assert_eq!(
            "hello world",
            obj.to_obj::<toml::Value>().unwrap()["message"]
                .as_str()
                .unwrap()
        );

        assert_eq!(
            "test2",
            container
                .layer_desc(3)
                .unwrap()
                .labels()
                .at("name")
                .str()
                .unwrap()
        );

        assert_eq!(
            "test",
            container
                .layer_desc(2)
                .unwrap()
                .labels()
                .at("name")
                .str()
                .unwrap()
        );

        assert!(
            container.to_run().is_err(),
            "container should not have any packed layers"
        );
    }

    #[test]
    fn test_container_run_state() {
        type TestPacker = GenericPacker<0, 0>;

        let ns = Namespace::ephemeral();
        let mut container =
            MultiRoot::<TestPacker>::build(ns.commit("test", b"hello world".as_slice()));

        container
            .push_object(&toml! {
                name = "hello"
            })
            .unwrap();

        let run = container.to_run().unwrap();
        assert_eq!("hello", run.object(2).at("name").str().unwrap());
    }

    #[test]
    fn test_container_run_state_unpack_all() {
        type TestPacker = GenericPacker<0, 0>;

        let ns = Namespace::ephemeral();
        let mut container =
            MultiRoot::<TestPacker>::build(ns.commit("test", b"hello world".as_slice()));

        container
            .push_object(&toml! {
                name = "hello"
            })
            .unwrap();

        container
            .push_object(&toml! {
                name = "hello2"
            })
            .unwrap();

        container
            .push_object(&toml! {
                name = "hello3"
            })
            .unwrap();

        assert!(
            container.unpack_all().is_err(),
            "to_run must be called first"
        );

        let run = container.to_run().unwrap();
        run.unpack_all().unwrap();

        assert_eq!("hello", run.object(2).at("name").str().unwrap());
        assert_eq!("hello2", run.object(3).at("name").str().unwrap());
        assert_eq!("hello3", run.object(4).at("name").str().unwrap());
    }

    #[test]
    fn test_container_fetch_content() {
        type TestPacker = GenericPacker<0, 0>;

        let ns = Namespace::ephemeral();
        let mut container =
            MultiRoot::<TestPacker>::build(ns.commit("test", b"hello world".as_slice()));

        container.push_content(b"hello world").unwrap();

        let read = container.to_read_only().unwrap();

        let mut dest = BytesMut::new();
        dest.put(b"existing data".as_slice());

        read.fetch_content(2, &mut dest).unwrap();

        assert_eq!(b"existing datahello world", dest.as_ref());
    }

    #[test]
    fn test_container_reader() {
        type TestPacker = GenericPacker<0, 0>;

        let ns = Namespace::ephemeral();
        let mut container =
            MultiRoot::<TestPacker>::build(ns.commit("test", b"hello world".as_slice()));

        container.push_content(b"hello world").unwrap();

        let read = container.to_read_only().unwrap();

        let mut reader = read.reader(2).unwrap();
        let mut string = String::new();
        reader.read_to_string(&mut string).unwrap();

        assert_eq!("hello world", string);
    }

    #[test]
    fn test_container_bytes() {
        type TestPacker = GenericPacker<0, 0>;

        let ns = Namespace::ephemeral();
        let mut container =
            MultiRoot::<TestPacker>::build(ns.commit("test", b"hello world".as_slice()));

        container.push_content(b"hello world").unwrap();

        let read = container.to_read_only().unwrap();

        let bytes = read.bytes(2).unwrap();
        assert_ne!(b"hello world", bytes, "should still be packed");
    }
}
