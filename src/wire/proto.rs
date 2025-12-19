use std::collections::BTreeMap;

use crate::Data;
use crate::Namespace;
use crate::Record;
use crate::RecordInfo;
use crate::opts::Spec;
use crate::util::Intern;
use crate::util::PeekExtensions;
use crate::vol::VolumeTarget;
use crate::wire::Boot;
use crate::wire::Describe;
use crate::wire::Ext;
use crate::wire::Fetch;
use crate::wire::FrameList;
use crate::wire::Root;
use crate::wire::ToolIndex;
use crate::wire::boot;
use crate::wire::cas::Descriptor;
use crate::wire::transport::Transport;
use crate::wire::transport::TransportMut;
use anyhow::anyhow;
use bytes::BufMut;
use flexbuffers::Blob;
use flexbuffers::MapBuilder;
use sha2::Sha256;
use tracing::debug;
use tracing::error;
type Header = flexbuffers::Builder;

/// Builds a "wire" protocol intermediate target
#[derive(Debug, Clone)]
pub struct Proto {
    pub(crate) ns: Namespace,
    /// Record info
    pub(crate) info: RecordInfo,
    /// Transport state
    pub(crate) transport: Transport,
    /// Optional, Virtual index to apply when building
    tool_index: Option<ToolIndex>,
    /// Optional, Tools to
    tools: Vec<&'static str>,
    /// Optional, Uses a root to store paths
    pub(crate) root: Option<Root>,
}

impl<'wire> Proto {
    /// Restores a protocol from boot loader parts
    ///
    /// If restoring from a non-inline transport, it's possible
    /// the full record data has not been received
    ///
    /// However, protocol allows this so that partial record views can be created
    #[inline]
    pub fn receive(boot: &Boot<'wire>, transport: &TransportMut) -> crate::Result<Proto> {
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
            tool_index: None,
            tools: vec![],
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
            tool_index: None,
            tools: vec![],
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
        self.tools.clear();
    }

    /// Enables frame transport with layout
    ///
    /// Returns an error if the layout is not valid for the current data
    #[inline]
    pub fn use_frame_transport(&mut self, frames: FrameList) -> crate::Result<()> {
        self.transport = Transport::Frame((frames, self.transport.data().clone()));
        Ok(())
    }

    /// Sends the current wire protocol state to target
    pub fn send<T>(&'wire self, target: T) -> crate::Result<T>
    where
        T: VolumeTarget + BufMut,
    {
        // Build Header
        let mut header = Header::default();
        let mut map = header.start_map();
        let rec = self.info.apply(&self.ns, &mut map);
        if !rec.is_inline() {
           return Err(anyhow!("Record info must be inline").into());
        }
        let mut framelist = FrameList { frames: vec![rec] };
        if self.transport.is_inline() {
            // If the transport can fit inside of the header than we apply it inline
            self.apply_transport_inline(&mut map, &mut framelist);

            // Since tools are optional they are placed at the end of the framelist
            self.apply_tools(&mut map, &mut framelist);
        } else {
            // In this case, the index is likely much smaller than the transport
            // It's useful to place the index ahead of the transport so that
            // partial views can include the index
            self.apply_tools(&mut map, &mut framelist);

            // We know data can't be stored inline, so it's frames are always at the end
            self.apply_transport(&mut framelist); // If the transport is not stored inline
        }
        map.end_map();

        // Check inline frame configuration
        for f in framelist.frames.iter() {
            if f.is_inline()
                && !header
                    .view()
                    .at(&f.label_idx_str())
                    .map(|b| b.flexbuffer_type().is_blob())
                    .unwrap_or_default()
            {
                return Err(anyhow!("Invalid inline frame {f:?}").into());
            }
        }

        // Begin encoding boot
        let mut codec = boot::Codec::from(target);

        // START of HEADER
        // START of boot
        codec.encode_boot_start(&self.ns);

        let fl_bin = flexbuffers::to_vec(&framelist).unwrap();
        // BOOT framelist
        codec.encode_bytes_desc(&fl_bin);
        // END of boot
        codec.encode_boot_end();

        // INLINE block
        codec.encode_inline(&fl_bin, header.view());
        // END of HEADER

        match Boot::decode(codec.filled()) {
            Ok(boot) => {
                boot.info().expect("MUST build a valid record info");
                // SUCCESS
                /*
                    TODO:
                    - Could validate inline frames
                */
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

        // Move frames into target
        if let Some(frames) = fl_bin.at("frames").as_iter() {
            for f in frames {
                let desc: Descriptor = f.try_to_obj()?;
                if desc.is_inline() {
                    continue; // All inline header frames have been written
                }
                // Read from the header first if there were any frames from the header
                // that could not be stored inline
                // Next, read from the transport directly
                if let Some(frame) = header
                    .view()
                    .at(&desc.label_idx_str())
                    .blob()
                    .or_else(|| self.transport.fetch(&self.ns, &desc))
                {
                    codec.put(frame);
                }
            }
        }
        Ok(codec.volume.into_parts().0)
    }

    /// Enables a tool on the protocol
    #[inline]
    pub fn use_tool(&mut self, name: &str) {
        self.tools.push(name.intern());
    }

    /// Enables a virtual index table w/ protocol
    #[inline]
    pub(crate) fn tool_index(&mut self) -> &mut ToolIndex {
        self.tool_index.get_or_insert_default()
    }

    #[inline]
    fn apply_transport_inline(&'wire self, map: &mut MapBuilder<'wire>, framelist: &mut FrameList) {
        let mut frame = self.transport.data().apply(&self.ns, map);
        frame.offset = framelist
            .frames
            .last()
            .map(|f| f.offset + f.size)
            .unwrap_or_default();
        framelist.frames.push(frame);
    }

    #[inline]
    fn apply_transport(&'wire self, framelist: &mut FrameList) {
        let mut transport = self.transport.describe(&self.ns);
        transport.align_offset(
            framelist
                .frames
                .last()
                .map(|f| f.offset)
                .unwrap_or_default(),
        );
        framelist.frames.append(&mut transport.frames);
    }

    /// Apply tools
    #[inline]
    fn apply_tools(&'wire self, map: &mut MapBuilder<'wire>, framelist: &mut FrameList) {
        use crate::wire::tool::Tool::{Indexer, Setting};

        if self.tools.is_empty() {
            return;
        }
        if let Some(tool_index) = self.tool_index.as_ref() {
            let mut settings = BTreeMap::new();
            for t in self.tools.iter() {
                if let Some((_tool_name, tool)) = tool_index.map.get_key_value(*t) {
                    match tool {
                        Indexer(indexer) => {
                            let _index = indexer(&Record::from_parts((
                                self.info.clone(),
                                self.transport.data().clone(),
                            )));

                            match _index {
                                Ok(index) => {
                                    let mut opts = crate::Opts::from(Spec::Tool);
                                    opts.set_object_storage(true);
                                    let mut desc =
                                        Descriptor::create::<Sha256>(&self.ns, opts, index.as_slice(), *t);
                                    desc.offset = framelist
                                        .frames
                                        .last()
                                        .map(|f| f.offset + f.size)
                                        .unwrap_or_default();
                                    debug!("Evaluated tool `{t}`:\n`{desc:#?}`");
                                    map.push(&desc.label_idx_str(), Blob(index.as_slice()));
                                    framelist.frames.push(desc);
                                }
                                Err(err) => {
                                    error!("Skipping tool `{t}`, error: {err}");
                                }
                            }
                        }
                        Setting(setting) => {
                            settings.insert(_tool_name, setting);
                        }
                    }
                }
            }

            match flexbuffers::to_vec(settings) {
                Ok(settings) => {
                    let opts = crate::Opts::from(Spec::Tool);
                    let mut desc = Descriptor::create::<Sha256>(&self.ns, opts, &settings, "setting");
                    desc.offset = framelist
                        .frames
                        .last()
                        .map(|f| f.offset + f.size)
                        .unwrap_or_default();
                    map.push(&desc.label_idx_str(), Blob(settings.as_ref()));
                    framelist.frames.push(desc);
                }
                Err(err) => {
                    error!("Could not encode settings: {err}");
                },
            }
        }
    }
}
