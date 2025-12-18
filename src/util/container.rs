use crate::{
    Data, IRecord, Index, Opts, Record, RecordInfo, Storage,
    record::record_crc,
    util::{Packer, packer::GenericPacker},
    virt::{AtlasEncoder, SharedVolume},
    vol::{MemoryMappedTarget, Volume, new_mmap_anon_target},
};
use ahash::HashSet;
use anyhow::anyhow;
use bytes::{BufMut, Bytes, BytesMut};
use generic_array::{GenericArray, typenum::U32};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Read},
    marker::PhantomData,
    sync::Arc,
};
use tracing::debug;

use super::{Peek, PeekExtensions};

/// Type-alias for a layer index
pub type LayerIndex<S> = Index<Layer, S>;

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

struct Builder {
    /// Internal serializer
    ser: flexbuffers::FlexbufferSerializer,
    /// Internal working buffer
    buffer: BytesMut,
    /// Layers being built into this container
    layers: Vec<(BuildDescriptor, Data)>,
}

enum State {
    /// Build state allows new layers to be pushed
    Build {
        /// Root record of the container
        root: Record,
        /// Layer builder
        builder: Builder,
        /// True if append mode is enabled
        append_mode: bool,
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
        vol: SharedRuntimeVolume,
    },
}

impl Clone for Container {
    fn clone(&self) -> Self {
        match &self.state {
            State::Build {
                root,
                builder,
                append_mode,
            } => {
                debug_assert!(
                    builder.ser.view().is_empty() && builder.buffer.is_empty(),
                    "Builder must not be in a partial state"
                );
                Self {
                    state: State::Build {
                        root: root.clone(),
                        builder: Builder {
                            ser: flexbuffers::FlexbufferSerializer::new(),
                            buffer: BytesMut::new(),
                            layers: builder.layers.clone(),
                        },
                        append_mode: *append_mode,
                    },
                    _p: PhantomData,
                }
            }
            State::Read { record } => Self {
                state: State::Read {
                    record: record.clone(),
                },
                _p: PhantomData,
            },
            State::Run { record, vol } => Self {
                state: State::Run {
                    record: record.clone(),
                    vol: vol.clone(),
                },
                _p: PhantomData,
            },
        }
    }
}

impl<P: Packer> MultiRoot<P> {
    /// Creates a read-only multi-root record
    ///
    /// Returns an error if the record is not a Multi Record
    #[inline]
    pub fn read(record: &impl IRecord) -> crate::Result<Self> {
        if !record.opts().is_multi() {
            return Err(anyhow!("Cannot read a non-multi root record as a Container").into());
        }

        Ok(Self {
            state: State::Read {
                record: record.to_record(),
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
                builder: Builder {
                    ser: flexbuffers::FlexbufferSerializer::new(),
                    buffer: BytesMut::new(),
                    layers: vec![],
                },
                append_mode: false,
            },
            _p: PhantomData::default(),
        };
        container
    }

    /// Creates a multi-root record from an existing multi-root record in "append" mode
    #[inline]
    pub fn append(root: Record) -> crate::Result<Self> {
        let read = Self::read(&root)?;

        let system_layer = read.system_layer()?;
        let mut building = Self::build(read.to_record());

        match &mut building.state {
            State::Build {
                root,
                builder: Builder { layers, .. },
                append_mode,
            } => {
                *append_mode = true;

                for (idx, _) in root.iter_layer_desc()?.enumerate().skip(2) {
                    layers.push((
                        BuildDescriptor::Existing {
                            system: system_layer.clone(),
                            layer: idx,
                        },
                        Data::default(),
                    ));
                }
            }
            _ => unreachable!("Should be in build state"),
        }

        Ok(building)
    }

    /// Tries to clone the container state
    ///
    /// Returns an error if the Container is in an uncloneable state
    #[inline]
    pub fn try_clone(&self) -> crate::Result<Self> {
        match &self.state {
            State::Build {
                root,
                builder,
                append_mode,
            } => {
                if !builder.ser.view().is_empty() || !builder.buffer.is_empty() {
                    return Err(anyhow!("Container build state has pending data").into());
                }

                Ok(Self {
                    state: State::Build {
                        root: root.clone(),
                        builder: Builder {
                            ser: flexbuffers::FlexbufferSerializer::new(),
                            buffer: BytesMut::new(),
                            layers: builder.layers.clone(),
                        },
                        append_mode: *append_mode,
                    },
                    _p: PhantomData,
                })
            }
            State::Read { record } => Ok(Self {
                state: State::Read {
                    record: record.clone(),
                },
                _p: PhantomData,
            }),
            State::Run { record, vol } => Ok(Self {
                state: State::Run {
                    record: record.clone(),
                    vol: vol.clone(),
                },
                _p: PhantomData,
            }),
        }
    }

    /// Swaps the current root record of the container
    ///
    /// Returns an error if the Container is not in build mode
    #[inline]
    pub fn swap_root(&mut self, replacement: Record) -> crate::Result<Record> {
        match &mut self.state {
            State::Build { root, .. } => Ok(std::mem::replace(root, replacement)),
            _ => Err(anyhow!("Cannot swap the root when the container is read only").into()),
        }
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

    /// Fetches layer content and puts it into buf
    ///
    /// If the content was packed, will unpack the content before putting it into buf
    #[inline]
    pub fn fetch_content(&self, layer: usize, buf: &mut BytesMut) -> crate::Result<()> {
        match &self.state {
            State::Build {
                root,
                builder: Builder { layers, .. },
                ..
            } => {
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
    pub fn layer_bytes(&self, layer: usize) -> crate::Result<&[u8]> {
        match &self.state {
            State::Build {
                root,
                builder: Builder { layers, .. },
                ..
            } => {
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
        let bytes = self.layer_bytes(layer)?;
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
    pub fn object(&self, layer: usize) -> impl PeekExtensions<'_> {
        self.try_object(layer).ok()
    }

    /// Returns an object at a layer
    ///
    /// Returns an error if unable to access object at layer
    ///
    /// Note: This only supports inline, non-packed objects
    #[inline]
    pub fn try_object(&self, layer: usize) -> crate::Result<Peek<'_>> {
        self.try_content(layer)
            .and_then(|b| Ok(Peek::from(flexbuffers::Reader::get_root(b)?)))
    }

    /// Returns an content bytes at a layer
    ///
    /// Returns an error if unable to access object at layer
    ///
    /// Note: This only supports inline, non-packed objects
    #[inline]
    pub fn try_content(&self, layer: usize) -> crate::Result<&[u8]> {
        match &self.state {
            State::Build {
                root,
                builder: Builder { layers, .. },
                ..
            } => {
                if layer == 0 {
                    return Ok(root.bytes());
                }

                if layer == 1 {
                    return Err(anyhow!("Cannot access system layer in build state").into());
                }

                layers
                    .get(layer - 2)
                    .map(|l| Ok(l.1.as_ref()))
                    .unwrap_or(Err(anyhow!("Layer not found").into()))
            }
            State::Read { record } => Ok(flexbuffers::Reader::get_root(record.bytes())
                .and_then(|root| root.as_vector().idx(layer).get_blob())
                .and_then(|obj| Ok(obj.0))?),
            State::Run { record, vol } => {
                if let Some(desc) = self.layer_desc(layer) {
                    if desc.is_packed() {
                        if !vol.vol().is_object_unpacked(layer) {
                            self.unpack(layer, desc)?;
                        }

                        let obj = vol.view().view_object(layer)?;
                        Ok(obj)
                    } else {
                        if let Some(inline) = record.layer(layer) {
                            Ok(inline)
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
            State::Build {
                root,
                builder: Builder { layers, .. },
                append_mode,
            } => {
                // 1) Build system layer
                let mut builder = flexbuffers::Builder::default();
                let mut sys = builder.start_vector();

                sys.start_map().end_map(); // Account for root layer
                sys.start_map().end_map(); // Account for sys layer

                for (desc, ..) in layers.iter() {
                    let mut layer = sys.start_map();
                    layer.push("is_object", desc.is_object());
                    layer.push("is_packed", desc.is_packed());
                    layer.push(
                        "labels",
                        flexbuffers::Blob(desc.labels().map(|l| l.buffer()).unwrap_or_default()),
                    );
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
                if root.opts().is_multi() && append_mode {
                    debug!("Detected 'append' mode");
                    multi_root.push(flexbuffers::Blob(root.layer(0).unwrap_or_default()));
                    multi_root.push(flexbuffers::Blob(system_layer.as_slice()));
                    for (idx, (_, bytes)) in layers.iter().enumerate() {
                        if bytes.is_empty() {
                            multi_root
                                .push(flexbuffers::Blob(root.layer(idx + 2).unwrap_or_default()));
                        } else {
                            multi_root.push(flexbuffers::Blob(bytes.as_ref()));
                        }
                    }
                } else {
                    multi_root.push(flexbuffers::Blob(root.bytes()));
                    multi_root.push(flexbuffers::Blob(system_layer.as_slice()));
                    for (.., bytes) in layers.iter() {
                        multi_root.push(flexbuffers::Blob(bytes.as_ref()));
                    }
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
            State::Build {
                builder: Builder { layers, .. },
                ..
            } => {
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
            State::Build {
                builder: Builder { layers, .. },
                ..
            } => layers
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

    /// Returns all layers stored in this vector
    ///
    /// Note: This will not include the root and system layers
    #[inline]
    pub fn layers(&self) -> crate::Result<Vec<Layer>> {
        Ok(self.try_clone()?.to_index(vec![])?.storage().to_vec())
    }

    /// Returns true if this container is a super set of the other container
    #[inline]
    pub fn is_super_set(&self, other: &Container) -> bool {
        if self.content() != other.content() {
            return false;
        }

        let current = self.to_record();
        let next = other.to_record();

        match (current.iter_layer_desc(), next.iter_layer_desc()) {
            (Ok(l), Ok(r)) => {
                let lhs = l.skip(2).fold(HashSet::default(), |mut s, l| {
                    s.insert(l.content().clone());
                    s
                });

                let rhs = r.skip(2).fold(HashSet::default(), |mut s, l| {
                    s.insert(l.content().clone());
                    s
                });

                lhs.is_superset(&rhs)
            }
            _ => false,
        }
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

                let mut objects = RuntimeVolume::new_atlas(target);

                for (idx, desc) in record.iter_layer_desc()?.enumerate() {
                    if idx == 0 || idx == 1 {
                        objects.pre_encode_inline();
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

                    if desc.is_packed() {
                        let _idx =
                            objects.pre_encode(desc.content(), desc.runtime_size() as u32)?;
                        debug_assert_eq!(idx, _idx);
                    } else {
                        let _idx = objects.pre_encode_inline();
                        debug_assert_eq!(idx, _idx);
                    }
                }

                Ok(Self {
                    state: State::Run {
                        record,
                        vol: objects.to_shared(),
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

                    if desc.is_packed() {
                        self.unpack(idx, desc)?;
                    }
                }
                Ok(())
            }
            _ => Err(anyhow!("Can only unpack all layers in a run state").into()),
        }
    }

    /// Converts to the inner record
    ///
    /// Note: If currently in "Build" state, all pending layers will be lost
    #[inline]
    pub fn to_inner(self) -> Record {
        match self.state {
            State::Build { root, .. } => root,
            State::Read { record } => record,
            State::Run { record, .. } => record,
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

                    vol.vol_mut().encode_object::<P>(layer, packed)?;
                    Ok(())
                } else {
                    return Err(anyhow!("Record does not have packed layer data").into());
                }
            }
            _ => Err(anyhow!("Can only unpack layers in Run mode").into()),
        }
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
            State::Build {
                builder: Builder { buffer, .. },
                ..
            } => {
                P::pack_bytes(content, buffer)?;
            }
            State::Read { .. } | State::Run { .. } => {
                return Err(anyhow!("Cannot push content to a read-only container").into());
            }
        }

        self.push(content.len(), digest.finalize().into(), labels, false)
    }

    #[inline]
    fn _push_object<T: Serialize>(
        &mut self,
        obj: &T,
        labels: Option<impl Serialize>,
    ) -> crate::Result<()> {
        let mut content = Sha256::new();
        let content_len;
        match &mut self.state {
            State::Build {
                builder: Builder { buffer, ser, .. },
                ..
            } => {
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

    fn push(
        &mut self,
        len: usize,
        digest: [u8; 32],
        labels: Option<impl Serialize>,
        is_object: bool,
    ) -> crate::Result<()> {
        match &mut self.state {
            State::Build {
                builder:
                    Builder {
                        ser,
                        buffer,
                        layers,
                    },
                ..
            } => {
                let next = buffer.split();

                let mut packed = Sha256::new();
                packed.update(next.as_ref());
                let packed: [u8; 32] = packed.finalize().into();

                ser.reset();
                if let Some(labels) = labels {
                    labels.serialize(&mut *ser)?;
                }

                if packed != digest {
                    layers.push((
                        BuildDescriptor::New {
                            content: digest,
                            packed: Some(Packed {
                                digest: packed,
                                unpacked: len as u64,
                            }),
                            len: next.len() as u64,
                            labels: ser.take_buffer(),
                            is_object,
                        },
                        next.freeze().into(),
                    ));
                } else {
                    layers.push((
                        BuildDescriptor::New {
                            content: digest,
                            packed: None,
                            len: next.len() as u64,
                            labels: ser.take_buffer(),
                            is_object,
                        },
                        next.freeze().into(),
                    ));
                }
            }
            State::Read { .. } | State::Run { .. } => {
                return Err(anyhow!("Cannot push content to a read-only container").into());
            }
        }

        Ok(())
    }

    /// Returns true if the container has any packed layers
    #[inline]
    fn has_packed(&self) -> bool {
        match &self.state {
            State::Build {
                builder: Builder { layers, .. },
                ..
            } => layers.iter().any(|l| l.0.is_packed()),
            State::Read { record } | State::Run { record, .. } => record
                .iter_layer_desc()
                .map(|mut l| l.any(|l| l.is_packed()))
                .unwrap_or_default(),
        }
    }

    /// Converts the multi-root record into an Index of the lower layers
    ///
    /// If the multi-root record has any packed layers, will call to_run(..) automatically before converting
    /// to an index
    #[inline]
    pub fn to_index<S>(mut self, storage: S) -> crate::Result<LayerIndex<S>>
    where
        S: Storage<Record = Layer>,
    {
        if matches!(self.state, State::Build { .. }) {
            debug!(
                "Multi-root record is in a build state, finalizing w/ to_read_only(..) before proceeding"
            );
            self = self.to_read_only()?;
        }

        if self.has_packed() {
            debug!("Multi-root record has packed layers, upgrade w/ to_run(..) before proceeding");
            self = self.to_run()?;
        }

        let mut index = LayerIndex::from(storage);

        match &self.state {
            State::Read { record } => {
                let container: Arc<_> = Container {
                    state: State::Read {
                        record: record.clone(),
                    },
                    _p: PhantomData,
                }
                .into();
                for (idx, desc) in record.iter_layer_desc()?.enumerate().skip(2) {
                    let mut opts = Opts::default();
                    if desc.is_object() {
                        opts.set_object_storage(true);
                    }
                    index
                        .index(Layer {
                            container: container.clone(),
                            opts,
                            layer: idx,
                        })
                        .result()?;
                }
            }
            State::Run { record, vol } => {
                let container: Arc<_> = Container {
                    state: State::Run {
                        record: record.clone(),
                        vol: vol.clone(),
                    },
                    _p: PhantomData,
                }
                .into();

                for (idx, desc) in record.iter_layer_desc()?.enumerate().skip(2) {
                    let mut opts = Opts::default();
                    if desc.is_object() {
                        opts.set_object_storage(true);
                    }
                    index
                        .index(Layer {
                            container: container.clone(),
                            opts,
                            layer: idx,
                        })
                        .result()?;
                }
            }
            _ => {
                unreachable!("Should detect this arm before this match")
            }
        }
        Ok(index)
    }

    /// Returns the system layer
    ///
    /// Return an error if the current state is not in read or run mode
    #[inline]
    pub fn system_layer(&self) -> crate::Result<SystemLayer> {
        match &self.state {
            State::Read { record } => {
                let mut opts = Opts::default();
                opts.set_object_storage(true);
                Ok(SystemLayer(Layer {
                    container: Container {
                        state: State::Read {
                            record: record.clone(),
                        },
                        _p: PhantomData,
                    }
                    .into(),
                    opts,
                    layer: 1,
                }))
            }
            State::Run { record, vol } => {
                let mut opts = Opts::default();
                opts.set_object_storage(true);
                Ok(SystemLayer(Layer {
                    container: Container {
                        state: State::Run {
                            record: record.clone(),
                            vol: vol.clone(),
                        },
                        _p: PhantomData,
                    }
                    .into(),
                    opts,
                    layer: 1,
                }))
            }
            _ => Err(anyhow!("System layer is only available in read or run state").into()),
        }
    }

    /// Returns the root layer as a record
    #[inline]
    pub fn root_layer(&self) -> Record {
        match &self.state {
            State::Build { root, .. } => root.clone(),
            _ => {
                let (info, data) = self.to_record().into_parts();
                let (uuid, ns_chk, ts, opts) = info.to_parts();
                let data = data
                    .find_view(self.bytes())
                    .expect("should be able to find it because self.byytes() is always from data")
                    .1;

                let (hi, _) = uuid.as_u64_pair();
                let uuid = uuid::Uuid::from_u64_pair(hi, record_crc(self.bytes(), ts));
                let info = RecordInfo {
                    key: uuid,
                    ns_chk,
                    opts,
                    ts,
                };
                Record::from_parts((info, data))
            }
        }
    }
}

#[derive(Clone)]
pub struct SystemLayer(Layer);

impl SystemLayer {
    /// Returns a descriptor for layer
    #[inline]
    fn desc<'a: 'b, 'b>(&'a self, layer: usize) -> Option<LayerDesc<'b>> {
        self.0
            .peek()
            .val()?
            .get_vector()
            .ok()
            .map(|v| LayerDesc::Read(v.idx(layer).into()))
    }

    /// Returns any labels associated to this layer
    #[inline]
    fn labels<'p>(&'p self, layer: usize) -> Option<Peek<'p>>
    where
        Self: 'p,
    {
        self.0
            .peek()
            .val()?
            .get_vector()
            .ok()
            .map(|v| v.idx(layer).into())
            .and_then(|v: Peek| {
                let labels = v.at("labels").blob()?;
                Some(Peek::from(flexbuffers::Reader::get_root(labels).ok()?))
            })
    }
}

/// Scopes the view of a container into a single layer stored in the container
#[derive(Clone)]
pub struct Layer {
    container: Arc<Container>,
    opts: Opts,
    layer: usize,
}

impl Layer {
    /// Returns the layer index this layer is accessing
    #[inline]
    pub fn idx(&self) -> usize {
        self.layer
    }

    /// Returns a reference to the container that owns this layer
    #[inline]
    pub fn owner(&self) -> &Container {
        &self.container
    }
}

impl ILayerDescriptor for Layer {
    fn runtime_size(&self) -> u64 {
        self.container
            .system_layer()
            .ok()
            .and_then(|s| s.desc(self.layer).map(|l| l.runtime_size()))
            .unwrap_or_default()
    }

    fn distribution(&self) -> GenericArray<u8, U32> {
        self.container
            .system_layer()
            .ok()
            .and_then(|s| s.desc(self.layer).map(|l| l.distribution()))
            .unwrap_or_default()
    }

    fn content(&self) -> GenericArray<u8, U32> {
        self.container
            .system_layer()
            .ok()
            .and_then(|s| s.desc(self.layer).map(|l| l.content()))
            .unwrap_or_default()
    }

    fn is_packed(&self) -> bool {
        self.container
            .system_layer()
            .ok()
            .and_then(|s| s.desc(self.layer).map(|l| l.is_packed()))
            .unwrap_or_default()
    }

    fn is_object(&self) -> bool {
        self.container
            .system_layer()
            .ok()
            .and_then(|s| s.desc(self.layer).map(|l| l.is_object()))
            .unwrap_or_default()
    }

    fn labels(&self) -> Option<Peek<'_>> {
        // To conform to borrow rules,
        // must access bytes directly
        match &self.container.state {
            State::Build { builder, .. } => builder
                .layers
                .get(self.layer.saturating_sub(2))
                .and_then(|l| l.0.labels()),
            State::Read { record } | State::Run { record, .. } => record
                .system()
                .ok()
                .and_then(|s| {
                    flexbuffers::Reader::get_root(
                        s.get_vector()
                            .ok()?
                            .idx(self.layer)
                            .get_map()
                            .ok()?
                            .idx("labels")
                            .get_blob()
                            .ok()?
                            .0,
                    )
                    .ok()
                })
                .map(Into::into),
        }
    }
}

impl IRecord for Layer {
    #[inline]
    fn ns_chk(&self) -> u64 {
        self.container.ns_chk()
    }

    #[inline]
    fn uuid(&self) -> uuid::Uuid {
        let (hi, _) = self.container.uuid().as_u64_pair();
        uuid::Uuid::from_u64_pair(
            hi ^ self.layer as u64,
            record_crc(self.bytes(), self.container.to_record().ts()),
        )
    }

    #[inline]
    fn opts(&self) -> &crate::Opts {
        &self.opts
    }

    #[inline]
    fn opts_mut(&mut self) -> Option<&mut crate::Opts> {
        None
    }

    #[inline]
    fn bytes(&self) -> &[u8] {
        self.container.try_content(self.layer).unwrap_or_default()
    }

    #[inline]
    fn to_record(&self) -> Record {
        let (k, data, ns_chk, ts, opts) = self.container.to_record().into_parts();

        if let Some((_, data)) =
            data.find_view(self.bytes())
                .or_else(|| match &self.container.state {
                    State::Run { vol, .. } => {
                        let backing: crate::Data = vol.clone().into();

                        backing.find_view(self.bytes())
                    }
                    _ => None,
                })
        {
            Record::from_parts((self.uuid(), data, self.ns_chk(), ts, self.opts))
        } else {
            Record::from_parts((k, data, ns_chk, ts, opts))
        }
    }
}

impl<'a> IRecord for &'a Layer {
    #[inline]
    fn ns_chk(&self) -> u64 {
        <Layer as IRecord>::ns_chk(self)
    }

    #[inline]
    fn uuid(&self) -> uuid::Uuid {
        <Layer as IRecord>::uuid(self)
    }

    #[inline]
    fn opts(&self) -> &crate::Opts {
        <Layer as IRecord>::opts(self)
    }

    #[inline]
    fn opts_mut(&mut self) -> Option<&mut crate::Opts> {
        None
    }

    #[inline]
    fn bytes(&self) -> &[u8] {
        <Layer as IRecord>::bytes(self)
    }

    #[inline]
    fn to_record(&self) -> Record {
        <Layer as IRecord>::to_record(self)
    }
}

trait IContainer<'p>
where
    Self: 'p,
{
    fn layer(&'p self, idx: usize) -> Option<&'p [u8]>;

    fn system(&'p self) -> crate::Result<Peek<'p>>;

    fn iter_layer_desc(&self) -> crate::Result<impl Iterator<Item = impl ILayerDescriptor>>;

    #[inline]
    fn layer_desc(&'p self, idx: usize) -> crate::Result<LayerDesc<'p>> {
        Ok(LayerDesc::Read(self.system()?.as_vector().idx(idx).into()))
    }
}

impl<'p> IContainer<'p> for Record {
    #[inline]
    fn layer(&'p self, idx: usize) -> Option<&'p [u8]> {
        if !self.opts().is_multi() {
            return None;
        }

        let root = flexbuffers::Reader::get_root(self.bytes()).ok()?;

        let blob = root.as_vector().idx(idx).get_blob().ok()?;

        Some(blob.0)
    }

    #[inline]
    fn iter_layer_desc(&self) -> crate::Result<impl Iterator<Item = impl ILayerDescriptor>> {
        if !self.opts().is_multi() {
            return Err(anyhow!("Record is not a multi-root record").into());
        }

        let system = self.system()?;

        if let Some(iter) = system.as_iter() {
            Ok(iter.map(|p| LayerDesc::Read(p)))
        } else {
            Err(anyhow!("System layer was in an unexpected format").into())
        }
    }

    #[inline]
    fn system(&'p self) -> crate::Result<Peek<'p>> {
       match self.peek().at_idx(1).peek() {
            Some(found) => {
                Ok(found)
            },
            None => {
                Err(anyhow!("Container is in an invalid format").into())
            },
        }
    }
}

/// Trait for types that are able to describe a layer
pub trait ILayerDescriptor {
    /// Total required-size at runtime for this layer
    fn runtime_size(&self) -> u64;

    /// Returns the "distribution" digest of this layer
    fn distribution(&self) -> GenericArray<u8, U32>;

    /// Returns the "content" digest of this layer
    fn content(&self) -> GenericArray<u8, U32>;

    /// Returns true if the layer has been packed
    fn is_packed(&self) -> bool;

    /// Returns true if the layer is an object
    fn is_object(&self) -> bool;

    /// Returns labels stored w/ this layer
    fn labels(&self) -> Option<Peek<'_>>;
}

enum LayerDesc<'p> {
    Build(&'p BuildDescriptor),
    Read(Peek<'p>),
}

impl<'p> ILayerDescriptor for LayerDesc<'p> {
    #[inline]
    fn runtime_size(&self) -> u64 {
        match self {
            LayerDesc::Build(build_descriptor) => build_descriptor.runtime_size(),
            LayerDesc::Read(peek) => peek.runtime_size(),
        }
    }

    #[inline]
    fn distribution(&self) -> GenericArray<u8, U32> {
        match self {
            LayerDesc::Build(build_descriptor) => build_descriptor.distribution(),
            LayerDesc::Read(peek) => peek.distribution(),
        }
    }

    #[inline]
    fn content(&self) -> GenericArray<u8, U32> {
        match self {
            LayerDesc::Build(build_descriptor) => build_descriptor.content(),
            LayerDesc::Read(peek) => peek.content(),
        }
    }

    #[inline]
    fn is_packed(&self) -> bool {
        match self {
            LayerDesc::Build(build_descriptor) => build_descriptor.is_packed(),
            LayerDesc::Read(peek) => peek.is_packed(),
        }
    }

    #[inline]
    fn is_object(&self) -> bool {
        match self {
            LayerDesc::Build(build_descriptor) => build_descriptor.is_object(),
            LayerDesc::Read(peek) => peek.is_object(),
        }
    }

    #[inline]
    fn labels(&self) -> Option<Peek<'_>> {
        match self {
            LayerDesc::Build(build_descriptor) => build_descriptor.labels(),
            LayerDesc::Read(peek) => peek.labels(),
        }
    }
}

#[derive(Clone)]
enum BuildDescriptor {
    New {
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
        /// Labels data
        labels: Vec<u8>,
        /// True if the layer being build is an object
        is_object: bool,
    },
    Existing {
        system: SystemLayer,
        layer: usize,
    },
}

const EMPTY_DIGEST: GenericArray<u8, U32> = GenericArray::from_array([0; 32]);

impl<'p> ILayerDescriptor for Peek<'p> {
    #[inline]
    fn runtime_size(&self) -> u64 {
        self.at("runtime_size").u64().unwrap_or_default()
    }

    #[inline]
    fn distribution(&self) -> GenericArray<u8, U32> {
        self.at("distribution")
            .blob()
            .map(|b| GenericArray::<u8, U32>::from_slice(b).clone())
            .unwrap_or(EMPTY_DIGEST.clone())
    }

    #[inline]
    fn content(&self) -> GenericArray<u8, U32> {
        self.at("content")
            .blob()
            .map(|b| GenericArray::<u8, U32>::from_slice(b).clone())
            .unwrap_or(EMPTY_DIGEST.clone())
    }

    #[inline]
    fn is_packed(&self) -> bool {
        self.at("is_packed").bool().unwrap_or_default()
    }

    #[inline]
    fn is_object(&self) -> bool {
        self.at("is_object").bool().unwrap_or_default()
    }

    #[inline]
    fn labels(&self) -> Option<Peek<'_>> {
        let labels = self.at("labels").blob()?;
        Some(Peek::from(flexbuffers::Reader::get_root(labels).ok()?))
    }
}

impl<'p> ILayerDescriptor for BuildDescriptor {
    /// Total required-size at runtime for this layer
    #[inline]
    fn runtime_size(&self) -> u64 {
        match self {
            BuildDescriptor::New { packed, len, .. } => {
                packed.as_ref().map(|p| p.unpacked).unwrap_or(*len)
            }
            BuildDescriptor::Existing { system, layer } => system
                .desc(*layer)
                .map(|l| l.runtime_size())
                .unwrap_or_default(),
        }
    }

    /// Returns the "distribution" digest of this layer
    #[inline]
    fn distribution(&self) -> GenericArray<u8, U32> {
        match self {
            BuildDescriptor::New {
                content, packed, ..
            } => packed.as_ref().map(|p| p.digest).unwrap_or(*content).into(),
            BuildDescriptor::Existing { system, layer } => match system.desc(*layer) {
                Some(p) => p.distribution(),
                None => [0; 32].into(),
            },
        }
    }

    /// Returns the "content" digest of this layer
    #[inline]
    fn content(&self) -> GenericArray<u8, U32> {
        match self {
            BuildDescriptor::New { content, .. } => (*content).into(),
            BuildDescriptor::Existing { system, layer } => match system.desc(*layer) {
                Some(p) => p.content(),
                None => [0; 32].into(),
            },
        }
    }

    /// Returns true if the layer has been packed
    #[inline]
    fn is_packed(&self) -> bool {
        match self {
            BuildDescriptor::New { packed, .. } => packed.is_some(),
            BuildDescriptor::Existing { system, layer } => match system.desc(*layer) {
                Some(p) => p.is_packed(),
                None => false,
            },
        }
    }

    /// Returns true if the layer is an object
    #[inline]
    fn is_object(&self) -> bool {
        match self {
            BuildDescriptor::New { is_object, .. } => *is_object,
            BuildDescriptor::Existing { system, layer } => match system.desc(*layer) {
                Some(p) => p.is_object(),
                None => false,
            },
        }
    }

    /// Returns labels for the this layer
    #[inline]
    fn labels(&self) -> Option<Peek<'_>> {
        match self {
            BuildDescriptor::New { labels, .. } => flexbuffers::Reader::get_root(labels.as_slice())
                .ok()
                .map(Peek::from),
            BuildDescriptor::Existing { system, layer } => system.labels(*layer),
        }
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
    #[inline]
    fn ns_chk(&self) -> u64 {
        match &self.state {
            State::Build { root, .. } => root.ns_chk(),
            State::Read { record } | State::Run { record, .. } => record.ns_chk(),
        }
    }

    #[inline]
    fn uuid(&self) -> uuid::Uuid {
        match &self.state {
            State::Build { root, .. } => root.uuid(),
            State::Read { record } | State::Run { record, .. } => record.uuid(),
        }
    }

    #[inline]
    fn opts(&self) -> &crate::Opts {
        match &self.state {
            State::Build { root, .. } => root.opts(),
            State::Read { record } | State::Run { record, .. } => record.opts(),
        }
    }

    #[inline]
    fn bytes(&self) -> &[u8] {
        match &self.state {
            State::Build { root, .. } => root.bytes(),
            State::Read { record } | State::Run { record, .. } => {
                record.layer(0).expect("should always have a layer 0")
            }
        }
    }

    #[inline]
    fn to_record(&self) -> Record {
        match &self.state {
            State::Build { root, .. } => root.to_record(),
            State::Read { record } | State::Run { record, .. } => record.to_record(),
        }
    }

    #[inline]
    fn opts_mut(&mut self) -> Option<&mut crate::Opts> {
        match &mut self.state {
            State::Build { root, .. } => root.opts_mut(),
            State::Read { record } | State::Run { record, .. } => record.opts_mut(),
        }
    }
}

impl<'b, P> IRecord for &'b MultiRoot<P> {
    #[inline]
    fn ns_chk(&self) -> u64 {
        MultiRoot::ns_chk(*self)
    }

    #[inline]
    fn uuid(&self) -> uuid::Uuid {
        MultiRoot::uuid(*self)
    }

    #[inline]
    fn opts(&self) -> &crate::Opts {
        MultiRoot::opts(*self)
    }

    #[inline]
    fn bytes(&self) -> &[u8] {
        MultiRoot::bytes(*self)
    }

    #[inline]
    fn to_record(&self) -> Record {
        MultiRoot::to_record(*self)
    }

    #[inline]
    fn opts_mut(&mut self) -> Option<&mut crate::Opts> {
        None
    }
}

/// Type-alias for a shared "Runtime" volume
type SharedRuntimeVolume = SharedVolume<MemoryMappedTarget, AtlasEncoder>;

/// Type-alias for a "Runtime" volume
type RuntimeVolume = Volume<MemoryMappedTarget, AtlasEncoder>;

#[cfg(test)]
mod test {
    use std::io::Read;

    use super::Container;
    use crate::{
        IRecord, Namespace, VecIndex, field, namespace,
        search::iter::Search,
        util::{
            PeekExtensions,
            container::{ILayerDescriptor, MultiRoot},
            packer::GenericPacker,
        },
    };
    use bytes::{BufMut, BytesMut};
    use toml::toml;

    #[test]
    #[tracing_test::traced_test]
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

        let system_layer = container.system_layer().unwrap();
        assert_eq!(
            "test",
            system_layer
                .desc(2)
                .unwrap()
                .labels()
                .at("name")
                .str()
                .unwrap()
        );

        let inner = container.to_record();

        let mut appending = Container::append(inner).unwrap();
        appending
            .push_object(&toml::toml! {
                name = "append layer"
            })
            .unwrap();

        let container_appended = appending.to_read_only().unwrap();
        let obj = container_appended.try_object(3).unwrap();

        assert_eq!(
            "hello world",
            obj.to_obj::<toml::Value>().unwrap()["message"]
                .as_str()
                .unwrap()
        );

        assert_eq!(
            "test2",
            container_appended
                .layer_desc(3)
                .unwrap()
                .labels()
                .at("name")
                .str()
                .unwrap()
        );

        assert_eq!(
            "test",
            container_appended
                .layer_desc(2)
                .unwrap()
                .labels()
                .at("name")
                .str()
                .unwrap()
        );

        assert_eq!(
            "append layer",
            container_appended.object(4).at("name").str().unwrap()
        );

        assert!(container_appended.is_super_set(&container));

        let mut index = VecIndex::default();
        index.index(container.to_record()).result().unwrap();
        index
            .index(container_appended.to_record())
            .result()
            .unwrap();

        assert_eq!(
            "append layer",
            Container::read(index.search(namespace(ns)).next().unwrap())
                .unwrap()
                .object(4)
                .at("name")
                .str()
                .unwrap()
        );
        assert!(
            container_appended.to_run().is_err(),
            "container should not have any packed layers"
        );
        assert!(logs_contain("Detected 'append' mode"));
        assert!(logs_contain(
            "Next container is a super set of the current container, auto-promotion may proceed"
        ));
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

        let bytes = read.layer_bytes(2).unwrap();
        assert_ne!(b"hello world", bytes, "should still be packed");
    }

    #[test]
    #[tracing_test::traced_test]
    fn test_container_index() {
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

        container
            .push_object(&toml! {
                name = "goodbye"

                [test]
                a = "lorem ipsum dolor sit amet"
                b = "consectetur adipiscing elit"
                c = "sed do eiusmod tempor incididunt"
                d = "ut labore et dolore magna aliqua"

                [test2]
                a = "lorem ipsum dolor sit amet"
                b = "consectetur adipiscing elit"
                c = "sed do eiusmod tempor incididunt"
                d = "ut labore et dolore magna aliqua"

                [test3]
                a = "lorem ipsum dolor sit amet"
                b = "consectetur adipiscing elit"
                c = "sed do eiusmod tempor incididunt"
                d = "ut labore et dolore magna aliqua"
            })
            .unwrap();

        let index = container.to_index(vec![]).unwrap();
        assert_eq!(3, index.search(field("name").starts_with("hello")).count());

        eprintln!("{}", index.storage().len());

        for r in crate::Storage::iter_records(index.storage()) {
            eprintln!("{} {:?}", r.peek().val().unwrap(), index.reverse_lookup(r));
        }

        // let layer = index.get(3).unwrap();
        // eprintln!("{} {}", index.storage().len(), layer.peek().val().unwrap());

        let rec = index
            .search(field("name").starts_with("goodbye"))
            .next()
            .unwrap();
        let rec = rec.to_record();
        assert_eq!("goodbye", rec.field("name").str().unwrap());

        assert!(logs_contain(
            "Multi-root record is in a build state, finalizing w/ to_read_only(..) before proceeding"
        ));
        assert!(logs_contain(
            " Multi-root record has packed layers, upgrade w/ to_run(..) before proceeding"
        ));
    }
}
