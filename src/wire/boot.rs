use crate::{
    Data, Namespace, Opts, RecordInfo,
    record::crc_digest,
    util::PeekExtensions,
    vol::{Volume, VolumeTarget},
    wire::{
        Fetch, TransportMut,
        cas::{Descriptor, FrameList},
        receive::Receive,
    },
};
use anyhow::anyhow;
use asynchronous_codec::{Decoder, FramedRead};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use sha2::Digest;
use std::ops::{Deref, DerefMut};
use tracing::{debug, trace};
use uuid::Uuid;

pub const NS_BLOCK_SIZE: usize = 128;
pub const NS_HEADER_BLOCK_SIZE: usize = 512;
pub const INLINE_BLOCK_SIZE: usize = 512;
pub const NS_HEADER_INLINE_BLOCK_SIZE: usize = 1024;

/// `Wire` protocol communication centers around a single component called a `Boot`
///
/// A `Boot` is an immutable fixed-size blob of data that contains all information
/// required to start a `Container`
///
/// A `Container` pulls frames to rebuild the original `Protocol` state. When all
/// frames have been pulled, the container can return a `Wire` to `receive` the
/// `Record` that was originally sent by the `Protocol`
///
/// This trait contains the key settings of the binary format of a `Boot`.
/// --
///
///
/// --
/// ## Glossary
/// - `block`: A fixed byte-length slice that represents a single unit of a logical component
/// - `prelude`: A `block` that identifies the protocol, encodes the namespace, and identifies the associated frame-list
/// - `framelist`: Contains a list of descriptors that identify each `Frame` that was encoded by the `Protocol`
/// - `inline`: A `block` that will always be accompanied by the `header` block
trait _Format {
    /// Size of the `prelude` block
    const PRELUDE_BLOCK_SIZE: usize;

    /// Size of the `header` block, which contains the `prelude` block and `framelist`
    ///
    /// Notes:
    /// - Since the framelist is included in the header, this size limits the frame capacity
    /// - Can fit about 10 frame descriptors total in a 512 block, which is more than enough at the moment
    const HEADER_BLOCK_SIZE: usize;

    /// Size of the `inline` block
    ///
    /// Notes:
    /// - This determines how "enriched" a single `Boot` can be
    ///     - In theory could mean different size `Boot`'s for different purposes
    const INLINE_BLOCK_SIZE: usize;

    /// Maximum size of an encoded `Framelist` per `Boot`
    const MAX_FRAMELIST_SIZE: usize = Self::HEADER_BLOCK_SIZE - Self::PRELUDE_BLOCK_SIZE;

    /// Total size of a single encoded `Boot`
    const BOOT_SIZE: usize = Self::HEADER_BLOCK_SIZE + Self::INLINE_BLOCK_SIZE;

    /// Size of a single `Container` `Unit`
    ///
    /// Notes:
    /// - 8 MiB can fit about 8000 `Boot` blobs
    const CONTAINER_UNIT_SIZE: usize = 1024 * 1024 * 8; // 8 MiB

    fn format_block() {}
}

struct _V1;

impl _Format for _V1 {
    const PRELUDE_BLOCK_SIZE: usize = 128;

    const HEADER_BLOCK_SIZE: usize = 512;

    const INLINE_BLOCK_SIZE: usize = 512;
}

/// Wrapper struct for encoding boot properties
pub(crate) struct Codec<T, S> {
    pub volume: Volume<T, S>,
    ns: Namespace,
}

impl<T: VolumeTarget + BufMut> Codec<T, ()> {
    #[inline]
    pub fn encode_boot_start(&mut self, ns: &Namespace) {
        let [ab, cd, optschk] = ns.encode();
        let (a, b) = ab.as_u64_pair();
        let (c, d) = cd.as_u64_pair();
        let (chk, opts) = optschk.as_u64_pair();

        // Apply magic word + namespace
        self.volume.target_mut().put(b"runir\0".as_slice());
        {
            self.encode_u64(a);
            self.encode_u64(b);
            self.encode_u64(c);
            self.encode_u64(d);
            self.encode_u64(opts);
            self.encode_u64(chk);
        }
        self.ns = ns.clone();
    }

    #[inline]
    pub fn encode_bytes_desc(&mut self, bytes: &[u8]) {
        let digest = sha2::Sha256::digest(bytes);
        let mut crc = crate::record::crc_digest();
        crc.update(&(bytes.len() as u64).to_be_bytes());
        self.encode_u64(bytes.len() as u64); // len
        self.volume.target_mut().put(digest.as_slice()); // digest (4 * u64)
        crc.update(digest.as_slice());
        self.encode_u64(crc.finalize());
    }

    #[inline]
    pub fn encode_boot_end(&mut self) {
        if !self.align_block(NS_BLOCK_SIZE) {
            // TODO: MUST fit in a 128 byte block
            todo!()
        }
    }

    #[inline]
    pub fn encode_inline(&mut self, fl_bin: &[u8], header: &[u8]) {
        trace!(fl_bin_len = fl_bin.len());
        self.put(fl_bin);
        if !self.align_block(NS_HEADER_BLOCK_SIZE) {
            // TODO:
            todo!(
                "Could not align block {} > {NS_HEADER_BLOCK_SIZE}",
                self.volume.target().pos()
            );
        }

        let framelist: FrameList = fl_bin.to_obj().unwrap(); // Zero-copy deserialize
        for f in framelist.frames.iter() {
            // TODO: Validate this frame
            if f.is_inline() {
                let label = f.label_idx_str();
                if let Some(blob) = header.at(&label).blob() {
                    self.put(blob);
                }
            }
        }

        if !self.align_block(NS_HEADER_INLINE_BLOCK_SIZE) {
            // TODO: Must fit inside of the 1024 inline block
            todo!()
        }
    }

    #[inline]
    fn encode_u64(&mut self, v: u64) {
        self.volume.target_mut().put_u64(v);
        // self.volume.target_mut().put_u8(b'\0');
    }

    /// Aligns the current encoded buffer to a block size
    ///
    /// Returns false if the current filled encoded buffer does not fit within the block size
    #[inline]
    #[must_use = "Must check for an alignment issue"]
    fn align_block(&mut self, block_size: usize) -> bool {
        let filled = self.volume.target().filled().len();
        if filled > block_size {
            return false;
        }
        let padding = block_size - filled;
        trace!(block_size, padding_bytes = padding);
        self.volume.target_mut().put_bytes(b'\0', padding);
        true
    }
}

impl<T> From<T> for Codec<T, ()> {
    fn from(value: T) -> Self {
        Self {
            volume: Volume::from_parts((value, ())),
            ns: Namespace::ephemeral(),
        }
    }
}

impl<T: VolumeTarget + BufMut> Deref for Codec<T, ()> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.volume.target()
    }
}

impl<T: VolumeTarget + BufMut> DerefMut for Codec<T, ()> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.volume.target_mut()
    }
}

/// Contains boot information for subsequent framed reads
#[derive(Clone, Debug)]
pub struct Boot {
    /// Namespace
    ns: Namespace,
    /// Frame layout
    layout: FrameList,
    /// Header bytes
    header: Bytes,
}

#[derive(Debug)]
pub enum BootError {
    TooShort,
    MissingDelim,
    NotABootHeader,
    NamespaceChecksumMismatch,
    ManifestDigestMismatch,
    ManifestInvalidFormat,
    ManifestChecksumMismatch,
    /// Unexpected error that has created a fault in the boot process
    Fault(crate::Error),
}

impl From<crate::Error> for BootError {
    fn from(value: crate::Error) -> Self {
        Self::Fault(value)
    }
}

impl Boot {
    /// Returns the namespace of the boot
    #[inline]
    pub fn ns(&self) -> &Namespace {
        &self.ns
    }

    /// Returns the frame layout of the boot
    #[inline]
    pub fn layout(&self) -> &FrameList {
        &self.layout
    }

    /// Returns a split view of the header bytes
    ///
    /// 0: Namespace/Manifest Definition
    /// 1: Manifest
    /// 2: Inline
    #[inline]
    pub fn split_header(&self) -> (&[u8], &[u8], &[u8]) {
        (
            &self.header[..NS_BLOCK_SIZE],
            &self.header[NS_BLOCK_SIZE..NS_HEADER_BLOCK_SIZE],
            &self.header[NS_HEADER_BLOCK_SIZE..],
        )
    }

    /// Returns the record info
    ///
    /// Returns None if the record info could not be read
    ///
    /// Note: Record info will always be inline
    #[inline]
    pub fn info(&self) -> Option<RecordInfo> {
        self.fetch(self.ns(), self.layout.frames.first()?)
            .and_then(|f| {
                let peek = f
                    .val()?
                    .at_many(&["label", "checksum", "ns_chk", "opts", "ts"]);

                match peek.as_slice() {
                    [
                        Some(label),
                        Some(checksum),
                        Some(ns_chk),
                        Some(opts),
                        Some(ts),
                        ..,
                    ] => Some(RecordInfo {
                        key: Uuid::from_u64_pair(label.as_u64(), checksum.as_u64()),
                        ns_chk: ns_chk.as_u64(),
                        opts: Opts::decode(opts.as_u64()),
                        ts: ts.as_u64(),
                    }),
                    _ => None,
                }
            })
    }

    /// Returns data for an inline tool
    #[inline]
    pub fn tool(&self, name: &str) -> Option<&[u8]> {
        self.fetch(
            self.ns(),
            self.layout
                .frames
                .iter()
                .find(|f| f.is_inline() && f.opts.is_tool() && f.label == self.ns().key(name))?,
        )
    }

    /// Returns a new Boot configured from a new header slice
    #[inline]
    pub fn decode(header: Bytes) -> std::result::Result<Boot, BootError> {
        use super::boot::BootError::*;

        if header.len() < NS_HEADER_INLINE_BLOCK_SIZE {
            return Err(TooShort); // MUST be at least 1024 bytes
        }

        fn decode_u64(settings: &mut impl Buf) -> std::result::Result<u64, BootError> {
            let next = settings.get_u64();
            // if settings.get_u8() != b'\0' {
            //     return Err(MissingDelim); // MUST be delimited by a null-terminator
            // } else {
            //     Ok(next)
            // }
            Ok(next)
        }

        let ns_header = &header[..NS_BLOCK_SIZE];

        let magic_word = &ns_header[..6];
        if magic_word != b"runir\0" {
            return Err(NotABootHeader);
        }

        let mut settings = BytesMut::from(&ns_header[6..]);
        let ns_a = decode_u64(&mut settings)?;
        let ns_b = decode_u64(&mut settings)?;
        let ns_c = decode_u64(&mut settings)?;
        let ns_d = decode_u64(&mut settings)?;
        let ns_opts = decode_u64(&mut settings)?;
        let ns_chk = decode_u64(&mut settings)?;

        let ns = Namespace::const_new([ns_a, ns_b, ns_c, ns_d], Opts::decode(ns_opts));
        if ns.chk() != ns_chk {
            return Err(NamespaceChecksumMismatch); // MUST resolve to the expected ns_chk value
        }

        let mut crc = crc_digest();
        let manifest_size = decode_u64(&mut settings)?;
        crc.update(manifest_size.to_be_bytes().as_slice());

        // let a = settings.get_u64();
        // let b = settings.get_u64();
        // let c = settings.get_u64();
        // let d = settings.get_u64();

        /*
            (.runir/ns)
            |-----> (info)
                    |------> (label)    // Can be a name, content digest, etc
                    |------> (crc)      // crc of (data/ts)
                    |------> (ts)
                    |------> (ns_chk)   // ns.chk() that created the record
                    |------> (opts)
            (.data/ns)
            |------> (len)
            |------> (digest)
                     |---------> (data_ns) // Could generate a namespace from the digest
                                 - But then what would be the purpose?
                                 - What question is the data_ns answering?
                                 - What blocks do I have?
            .runir
            .settings
            {ext}/{ns}/:frames/
            - When record has been committed...
            
        */

        let manifest_digest = &settings.chunk()[..32].to_vec();
        settings.advance(32);
        crc.update(manifest_digest);

        let mchk = settings.get_u64();
        if mchk != crc.finalize() {
            return Err(ManifestChecksumMismatch);
        }

        // Parse the top top of the entry block
        let entry = &header[NS_BLOCK_SIZE..];
        let manifest = &entry[..manifest_size as usize];
        let digest = sha2::Sha256::digest(manifest);
        if digest.as_slice() != manifest_digest.as_slice() {
            tracing::error!(
                "{} != {}",
                hex::encode(digest.as_slice()),
                hex::encode(manifest_digest.as_slice())
            );
            return Err(ManifestDigestMismatch); // MUST match digest from archive extension
        }

        match manifest.val() {
            Some(_m) => {
                let _manifest: FrameList = _m.try_to_obj().map_err(Fault)?;
                Ok(Boot {
                    ns,
                    header: header,
                    layout: _manifest,
                })
            }
            None => Err(ManifestInvalidFormat), // MUST deserialize into cas::Manifest
        }
    }

    /// Returns the total number of bytes required for this boot
    #[inline]
    pub fn required_size(&self) -> u64 {
        self.layout.frames.iter().map(|d| d.size).sum()
    }

    /// Returns true if the entire record is available in the header
    #[inline]
    pub fn is_boot_inline(&self) -> bool {
        self.required_size() < INLINE_BLOCK_SIZE as u64
    }

    /// Returns a new container to complete the boot process
    #[inline]
    pub fn start(&self) -> crate::Result<Container> {
        Ok(Container {
            boot: self.clone(),
            transport: if self.is_boot_inline() {
                TransportMut::Inline
            } else {
                let mut recv = Receive::new(self.ns(), self.layout.clone())?;

                // Configure the receive so that it starts at the right frame
                for (idx, f) in self.layout.frames.iter().enumerate() {
                    if self.fetch(self.ns(), f).is_some() {
                        recv.last = idx;
                    } else {
                        break;
                    }
                }
                TransportMut::Receive(recv)
            },
            frames: vec![],
        })
    }

    /// Returns the inline header data
    #[inline]
    pub fn inline(&self) -> &[u8] {
        &self.header[NS_HEADER_BLOCK_SIZE..]
    }

    /// Returns the inline record `.data`
    #[inline]
    pub fn inline_data(&self) -> Option<Data> {
        self.layout
            .frames
            .iter()
            .find(|f| f.label == self.ns.key(".data"))
            .and_then(|f| self.fetch(&self.ns, f))
            .map(Data::from)
    }

    /// Returns the header bytes
    #[inline]
    pub fn header(&self) -> &[u8] {
        &self.header
    }
}

/// Container that reassembles a protocol from a byte stream
#[derive(Debug)]
pub struct Container {
    pub(super) boot: Boot,
    pub(super) transport: TransportMut,
    frames: Vec<Descriptor>,
}

impl Container {
    /// Returns true if the boot is complete
    #[inline]
    pub fn is_completed(&self) -> bool {
        self.transport.is_inline()
            || self
                .transport
                .as_receive()
                .map(Receive::is_complete)
                .unwrap_or_default()
    }

    /// Returns a reference to the inner `Boot`
    #[inline]
    pub fn boot(&self) -> &Boot {
        &self.boot
    }

    /// Pulls frames to complete the boot process
    ///
    /// Returns an error if an invalid digest is encountered while reading frames
    /// or if reading from the input i/o failed
    ///
    /// If all no error occurs, future will complete when all frames have been
    /// received
    #[inline]
    pub async fn pull<R>(&mut self, input: R) -> crate::Result<()>
    where
        R: futures::AsyncRead + Send + Unpin,
    {
        use Frame::{PullBytes, Received};
        use futures::TryStreamExt;
        if self.transport.is_inline() || !self.transport.is_receive() {
            return Ok(());
        }

        let transport = std::mem::replace(&mut self.transport, TransportMut::Empty);
        let mut framed = FramedRead::new(input, transport);

        let mut last_frame = PullBytes(self.boot.layout.required_capacity() as usize);
        while let Some(next) = framed.try_next().await? {
            let last_frame = std::mem::replace(&mut last_frame, next.clone());
            match next {
                PullBytes(needs_more) if matches!(last_frame, PullBytes(last_demand) if needs_more == last_demand) =>
                {
                    // stalled: no bytes have been read
                    break;
                }
                PullBytes(needs_more) => {
                    debug!("Needs more bytes {needs_more}")
                }
                Received(descriptor) => {
                    self.frames.push(descriptor);
                }
            }
        }
        let _ = std::mem::replace(&mut self.transport, framed.into_parts().decoder);
        Ok(())
    }

    /// Returns a tool `Frame`
    ///
    /// Searches inline first, and then checks Transport
    ///
    /// Returns None if the frame could not be found
    #[inline]
    pub fn tool(&self, name: &str) -> Option<&[u8]> {
        self.boot.tool(name).or_else(|| {
            self.frames
                .iter()
                .find(|f| f.opts.is_tool() && f.label == self.boot.ns().key(name))
                .and_then(|f| self.fetch(self.boot.ns(), f))
        })
    }
}

/// Frames returned while decoding received bytes
#[derive(Clone, Debug)]
pub enum Frame {
    PullBytes(usize),
    Received(Descriptor),
}

impl<'peek> Decoder for TransportMut {
    type Item = Frame;

    type Error = crate::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        use super::receive::ReceiveError::*;
        use TransportMut::*;

        match self {
            Inline | Empty => Ok(None),
            Receive(receive) => match receive.decode(src) {
                Ok(next) => Ok(next.map(|f| Frame::Received(f))),
                Err(err) => match err {
                    Pull(pull_bytes) => Ok(Some(Frame::PullBytes(pull_bytes))),
                    InvalidFrame => Err(anyhow!("Received an invalid frame").into()),
                    Fault(error) => Err(error),
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use ascii::AsAsciiStr;
    use bytes::{BufMut, BytesMut};
    use uuid::Uuid;
    use zstd::zstd_safe::WriteBuf;

    use crate::{
        Namespace,
        test::test_cas_record,
        util::PeekExtensions,
        vol::VolumeTarget,
        wire::{
            Fetch, Wire,
            boot::{Boot, NS_HEADER_BLOCK_SIZE, NS_HEADER_INLINE_BLOCK_SIZE},
        },
    };

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_encode_decode_cas_record_boot() {
        let ns = Namespace::new("test");
        let wire = Wire::new(ns.clone());
        let target = wire.push(test_cas_record(), None).unwrap();

        let bytes = target.snapshot().unwrap();
        assert_eq!(bytes.len(), NS_HEADER_INLINE_BLOCK_SIZE);

        // Test decoding boot
        let boot = Boot::decode(bytes.clone()).unwrap();
        let (prelude, manifest, inline) = boot.split_header();
        fn test_boot(ns: Namespace, boot: Boot) {
            let (_, manifest, _) = boot.split_header();
            assert_eq!(boot.ns.chk(), ns.chk());
            assert_eq!(boot.layout.frames.len(), 2);
            assert_eq!(boot.layout.frames[0].label, ns.key(".runir"));
            assert_eq!(boot.layout.frames[1].label, ns.key(".data"));

            let _container = boot
                .fetch(&ns, &boot.layout.frames[0])
                .unwrap()
                .val()
                .unwrap();
            let _object = boot
                .fetch(&ns, &boot.layout.frames[1])
                .unwrap()
                .val()
                .unwrap();

            // Test manifest was decoded correctly
            assert_eq!(manifest.val().at("frames").as_iter().unwrap().count(), 2);

            let data_start = NS_HEADER_BLOCK_SIZE;
            let _runir = &boot.header[data_start..data_start + 112];
            let data = &boot.header[data_start + 112..data_start + 112 + 29];
            assert_eq!(data.at("value").str(), Some("hello world"));

            eprintln!("{:#?}", boot.layout());
            eprintln!("{}", _runir.val().unwrap());

            boot.info().unwrap();
        }
        test_boot(ns.clone(), boot.clone());

        // Test creating a loader and doing a boot->load
        let mut loader = boot.start().unwrap();

        // MOCK: This would be an i/o stream to the frame bytes
        //       If inline, this is a No-op
        loader.pull(bytes.as_ref()).await.unwrap();

        // Debug print boot blocks
        fn debug_print_block(block: &[u8], block_size: usize) {
            for line in block
                .chunks(block_size)
                .inspect(|block| {
                    let padding = block.iter().rev().take_while(|v| **v == 0).count();
                    println!("\n--");
                    println!(
                        "size: {}, bytes: {}, padding_bytes: {}",
                        block.len(),
                        block.len() - padding,
                        padding
                    )
                })
                .flat_map(|c| c.chunks(8))
            {
                for b in line {
                    print!("{b:02x}   ");
                }
                println!()
            }
        }
        debug_print_block(prelude, 128);
        debug_print_block(manifest, 128);
        debug_print_block(inline, 256);
    }
}
