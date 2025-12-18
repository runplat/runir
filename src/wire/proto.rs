use crate::Data;
use crate::Namespace;
use crate::Record;
use crate::RecordInfo;
use crate::util::PeekExtensions;
use crate::util::ser::BytesMap;
use crate::vol::VolumeTarget;
use crate::wire::Boot;
use crate::wire::Describe;
use crate::wire::Ext;
use crate::wire::Fetch;
use crate::wire::FrameList;
use crate::wire::Root;
use crate::wire::boot;
use crate::wire::cas::Descriptor;
use crate::wire::transport::Transport;
use crate::wire::transport::TransportMut;
use anyhow::anyhow;
use bytes::BufMut;
use flexbuffers::Blob;
use flexbuffers::MapBuilder;
use serde::Deserialize;
use serde::Serialize;
type Header = flexbuffers::Builder;

/// Builds a "wire" protocol intermediate target
#[derive(Debug, Clone)]
pub struct Proto<'wire> {
    pub(crate) ns: Namespace,
    /// Record info
    pub(crate) info: RecordInfo,
    /// Transport state
    pub(crate) transport: Transport<'wire>,
    /// Optional, Index of data to include with this record
    index: Option<Index<'wire>>,
    /// Optional, Uses a root to store paths
    pub(crate) root: Option<Root>,
}

/// State for storing additional data for this record
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Index<'wire> {
    /// Map of indexes
    ///
    /// Each entry contains a value that can be viewed using a Peek interface
    #[serde(borrow)]
    map: BytesMap<'wire>,
}

impl<'wire> Ext for Index<'wire> {
    fn name(&self) -> &str {
        ".index"
    }

    fn author(&self, map: &mut flexbuffers::MapBuilder<'_>) {
        for (k, v) in self.map.0.iter() {
            map.push(k, Blob(v.as_ref()));
        }
    }

    fn apply<'a>(&'a self, map: &mut flexbuffers::MapBuilder<'_>) -> Descriptor<'a>
        where
            Self: Sized
    {
        let mut desc = self.default_apply(map);
        desc.opts.set_ext_spec(true);
        desc
    }
}

impl<'wire> Proto<'wire> {
    /// Restores a protocol from boot loader parts
    /// 
    /// If restoring from a non-inline transport, it's possible
    /// the full record data has not been received
    /// 
    /// However, protocol allows this so that partial record views can be created
    #[inline]
    pub fn restore(
        boot: &Boot<'wire>,
        transport: &TransportMut<'wire>,
    ) -> crate::Result<Proto<'wire>> {
        Ok(Proto {
            ns: boot.ns().clone(),
            info: boot
                .info()
                .map(Ok)
                .unwrap_or_else(|| Err(anyhow!("Boot is not ready")))?,
            transport: match transport {
                TransportMut::Empty => Transport::Inline(Data::default()),
                TransportMut::Inline => Transport::Inline(Data::from(boot.header())),
                TransportMut::Receive(receive) => Transport::Receive(receive.clone()),
            },
            index: None, // TODO:
            root: None, // TODO:
        })
    }

    /// Creates a new wire builder from a namespace
    #[inline]
    pub fn new(ns: Namespace) -> Self {
        Self {
            ns,
            info: RecordInfo::default(),
            transport: Transport::Inline(Data::default()),
            index: None,
            root: None,
        }
    }

    /// Sets the root to use with this protocol
    ///
    /// When the root is set, it enables creating targets
    /// from the standard namespace volume path
    #[inline]
    pub fn use_root(&mut self, root: Root) {
        self.root = Some(root);
    }

    /// Sets the record properties on the wire builder
    #[inline]
    pub fn use_record(&mut self, rec: Record) {
        self.info = rec.info.clone();
        self.transport = Transport::Inline(rec.data);
        self.index.take();
    }

    /// Enables frame transport with layout
    ///
    /// Returns an error if the layout is not valid for the current data
    #[inline]
    pub fn use_frame_transport(&mut self, frames: FrameList<'wire>) -> crate::Result<()> {
        self.transport = Transport::Frame((frames, self.transport.data().clone()));
        Ok(())
    }

    /// Builds the wire protocol into an intermediate volume target
    pub fn build<T>(&'wire self, target: T) -> crate::Result<T>
    where
        T: VolumeTarget + BufMut,
    {
        // Build Header
        let mut header = Header::default();
        let mut map = header.start_map();
        let rec = self.info.apply(&mut map);
        if !rec.is_inline() {
            todo!() // Record info MUST be inline
        }
        let mut framelist = FrameList { frames: vec![rec] };
        if self.transport.is_inline() {
            self.apply_transport_inline(&mut map, &mut framelist);
            
            // Since index is optional it's placed at the end of the encoding
            self.apply_index(&mut map, &mut framelist);
        } else {
            // In this case, the index is likely much smaller than the transport
            // It's useful to place the index ahead of the transport so that
            // partial views can include the index
            self.apply_index(&mut map, &mut framelist);
            self.apply_transport(&mut framelist); // If the transport is not stored inline
        }
        map.end_map();

        // Begin encodering boot
        let mut boot_enc = boot::EncodeBoot::from(target);

        // START of HEADER
        // START of boot
        boot_enc.encode_boot_start(&self.ns);

        let fl_bin = flexbuffers::to_vec(&framelist).unwrap();
        // BOOT framelist
        boot_enc.encode_bytes_desc(&fl_bin);
        // END of boot
        boot_enc.encode_boot_end();

        // INLINE block
        boot_enc.encode_inline(&fl_bin, header.view());
        // END of HEADER

        match Boot::decode(boot_enc.filled()) {
            Ok(_) => {
                // SUCCESS
            }
            Err(err) => match err {
                // TODO: For now these are logic errors so panic to find the issue loudly
                boot::BootError::TooShort => panic!(),
                boot::BootError::MissingDelim => panic!(),
                boot::BootError::NotABootHeader => panic!(),
                boot::BootError::NamespaceChecksumMismatch => panic!(),
                boot::BootError::ManifestDigestMismatch => panic!(),
                boot::BootError::ManifestInvalidFormat => panic!(),
                boot::BootError::ManifestChecksumMismatch => panic!(),
                boot::BootError::Fault(error) => {
                    return Err(error);
                }
            },
        }

        if let Some(frames) = fl_bin.at("frames").as_iter() {
            for f in frames {
                let desc: Descriptor = f.try_to_obj()?;
                if desc.is_inline() {
                    continue;
                }
                if let Some(frame) = header
                    .view()
                    .at(&desc.label)
                    .blob()
                    .or_else(|| self.transport.fetch(&desc))
                {
                    boot_enc.put(frame);
                }
            }
        }
        Ok(boot_enc.volume.into_parts().0)
    }

    /// Enables an index
    #[inline]
    pub fn use_index(&mut self, index: Index<'wire>) {
        self.index = Some(index);
    }

    #[inline]
    fn apply_transport_inline(
        &'wire self,
        map: &mut MapBuilder<'wire>,
        framelist: &mut FrameList<'wire>,
    ) {
        let mut frame = self.transport.data().apply(map);
        frame.offset = framelist.frames.last().map(|f| f.size).unwrap_or_default();
        framelist.frames.push(frame);
    }

    #[inline]
    fn apply_transport(&'wire self, framelist: &mut FrameList<'wire>) {
        let mut transport = self.transport.describe();
        transport.align_offset(
            framelist
                .frames
                .last()
                .map(|f| f.offset)
                .unwrap_or_default(),
        );
        framelist.frames.append(&mut transport.frames);
    }

    /// Apply index to the map and framelist
    #[inline]
    fn apply_index(&'wire self, map: &mut MapBuilder<'wire>, framelist: &mut FrameList<'wire>) {
        if let Some(index) = self.index.as_ref() {
            let mut index = index.apply(map);
            index.offset = framelist.frames.last().map(|f| f.size).unwrap_or_default();
            framelist.frames.push(index);
        }
    }
}
