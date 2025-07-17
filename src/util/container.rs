use std::marker::PhantomData;

use anyhow::anyhow;
use bytes::{BufMut, Bytes, BytesMut};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{IRecord, Record};

use super::{Peek, PeekExtensions};

/// Type-alias for the default multi-root record construct
///
/// - Any content over 128 bytes will be compressed
/// - The default compression level will be used
pub type Container = MultiRoot<GenericPacker<128, 0>>;

/// Container provides functions for building a "multi" root record
///
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
    Read {
        /// Record that contains all layers
        record: Record,
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
    pub fn push_content(
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
    pub fn push_object<T: Serialize>(
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
            State::Read { .. } => {
                return Err(anyhow!("Cannot push content to a read-only container").into());
            }
        }

        self.push(content_len, content.finalize().into(), labels, true)
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
        }
    }

    /// Returns the labels for a layer
    ///
    /// Note: Root and System layers will never have labels
    ///
    /// Note: This only supports inline, non-packed objects
    #[inline]
    pub fn labels(&self, layer: usize) -> impl PeekExtensions {
        self.try_labels(layer).ok()
    }

    /// Returns labels stored for a layer
    ///
    /// Returns an error if unable to access labels for layer
    ///
    /// Note: This only supports inline, non-packed objects
    #[inline]
    pub fn try_labels(&self, layer: usize) -> crate::Result<Peek> {
        match &self.state {
            State::Build { layers, .. } => {
                if layer == 0 {
                    return Err(anyhow!("Root does not have labels").into());
                }

                if layer == 1 {
                    return Err(anyhow!("System layer does not have labels").into());
                }

                layers
                    .get(layer - 2)
                    .map(|l| {
                        Ok(Peek::from(flexbuffers::Reader::get_root(
                            l.0.labels.as_slice(),
                        )?))
                    })
                    .unwrap_or(Err(anyhow!("Layer not found").into()))
            }
            State::Read { record } => {
                let system = flexbuffers::Reader::get_root(record.bytes())
                    .and_then(|root| root.as_vector().idx(1).get_blob())
                    .and_then(|obj| Ok(Peek::from(flexbuffers::Reader::get_root(obj.0)?)))?;

                let labels = system
                    .as_vector()
                    .idx(layer)
                    .as_map()
                    .idx("labels")
                    .as_blob();
                Ok(Peek::from(flexbuffers::Reader::get_root(labels.0)?))
            }
        }
    }

    /// Transitions the internal multi-root state to read-only
    ///
    /// Returns an error if the current state is already in read-only mode
    #[inline]
    pub fn finish(self) -> crate::Result<Self> {
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
                    layer.push("content", desc.content().as_slice());
                    layer.push("distribution", desc.distribution().as_slice());
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
                record.opts_mut().set_multi_root_storage(true);

                Ok(Self {
                    state: State::Read { record },
                    _p: PhantomData,
                })
            }
            State::Read { .. } => Ok(self),
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
            State::Read { .. } => {
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

    fn unpack_bytes<'u>(from: &[u8], to: &'u mut [u8]) -> crate::Result<()> {
        if from.len() == to.len() {
            to.copy_from_slice(from);
            Ok(())
        } else {
            let decoded = zstd::decode_all(from)?;
            to.copy_from_slice(&decoded);
            Ok(())
        }
    }
}

/// Packer handles packing and unpacking bytes
pub trait Packer: private::Sealed {
    /// Pack bytes into a layer
    fn pack_bytes(bytes: &[u8], buffer: &mut BytesMut) -> crate::Result<()>;

    /// Unpack bytes from a layer
    fn unpack_bytes<'u>(from: &[u8], to: &'u mut [u8]) -> crate::Result<()>;

    /// Pack an object into a layer
    fn pack_object<T: Serialize>(
        obj: &T,
        ser: &mut flexbuffers::FlexbufferSerializer,
        buffer: &mut BytesMut,
    ) -> crate::Result<()> {
        obj.serialize(&mut *ser)?;
        Self::pack_bytes(ser.view(), buffer)
    }

    /// Unpack an object from a layer
    fn unpack_object<'de, T: Deserialize<'de>>(from: &[u8], to: &'de mut [u8]) -> crate::Result<T> {
        Self::unpack_bytes(from, to)?;
        Ok(flexbuffers::from_slice(to)?)
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

impl BuildDescriptor {
    /// Total required-size at runtime for this layer
    #[inline]
    pub fn runtime_size(&self) -> u64 {
        self.packed.as_ref().map(|p| p.unpacked).unwrap_or(self.len)
    }

    /// Returns the "distribution" digest of this layer
    #[inline]
    pub fn distribution(&self) -> &[u8; 32] {
        self.packed
            .as_ref()
            .map(|p| &p.digest)
            .unwrap_or(&self.content)
    }

    /// Returns the "content" digest of this layer
    #[inline]
    pub fn content(&self) -> &[u8; 32] {
        &self.content
    }

    /// Returns true if the layer has been packed
    #[inline]
    pub fn is_packed(&self) -> bool {
        self.packed.is_some()
    }

    /// Returns true if the layer is an object
    #[inline]
    pub fn is_object(&self) -> bool {
        self.is_object
    }
}

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
            State::Read { record } => record.ns_chk(),
        }
    }

    fn uuid(&self) -> uuid::Uuid {
        match &self.state {
            State::Build { root, .. } => root.uuid(),
            State::Read { record } => record.uuid(),
        }
    }

    fn opts(&self) -> &crate::Opts {
        match &self.state {
            State::Build { root, .. } => root.opts(),
            State::Read { record } => record.opts(),
        }
    }

    fn bytes(&self) -> &[u8] {
        match &self.state {
            State::Build { root, .. } => root.bytes(),
            State::Read { record } => record.bytes(),
        }
    }

    fn peek<'peek>(&'peek self) -> impl super::PeekExtensions<'peek> {
        match &self.state {
            // While in the build state, layers are not active to maintain idempotency
            State::Build { root, .. } => root.peek().val(),
            // The canonical record when reading a container is always the root
            State::Read { record } => record
                .peek()
                .val()
                .map(|p| p.as_vector().idx(0))
                .map(super::Peek::from),
        }
    }

    fn to_record(&self) -> Record {
        match &self.state {
            State::Build { root, .. } => root.to_record(),
            State::Read { record } => record.to_record(),
        }
    }
}

#[cfg(test)]
mod test {
    use super::Container;
    use crate::{Namespace, util::PeekExtensions};
    use toml::toml;

    #[test]
    fn test_container() {
        let ns = Namespace::ephemeral();

        let mut container = Container::build(ns.commit("test", b"hello world".as_slice()));

        container
            .push_content(
                b"good bye world".as_slice(),
                Some(toml! {
                    name = "test"
                }),
            )
            .unwrap();

        container
            .push_object(
                &toml! {
                    message = "hello world"
                },
                Some(toml! {
                    name = "test2"
                }),
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
            container.try_labels(3).unwrap().at("name").str().unwrap()
        );

        assert_eq!(
            "test",
            container.try_labels(2).unwrap().at("name").str().unwrap()
        );

        let container = container.finish().unwrap();
        let obj = container.try_object(3).unwrap();

        assert_eq!(
            "hello world",
            obj.to_obj::<toml::Value>().unwrap()["message"]
                .as_str()
                .unwrap()
        );

        assert_eq!(
            "test2",
            container.try_labels(3).unwrap().at("name").str().unwrap()
        );

        assert_eq!(
            "test",
            container.try_labels(2).unwrap().at("name").str().unwrap()
        );
    }
}
