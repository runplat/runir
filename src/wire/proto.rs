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
use crate::wire::boot::Container;
use crate::wire::cas::Descriptor;
use crate::wire::tool::Tool;
use crate::wire::transport::Transport;
use crate::wire::transport::TransportMut;
use anyhow::anyhow;
use bytes::BufMut;
use flexbuffers::Blob;
use flexbuffers::MapBuilder;
use futures::AsyncRead;
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
    pub(crate) tools: Vec<&'static str>,
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
    pub fn receive(&self, container: &Container) -> crate::Result<Proto> {
        let proto = self.clone();
        Ok(Proto {
            ns: container.boot.ns().clone(),
            info: container
                .boot
                .info()
                .map(Ok)
                .unwrap_or_else(|| Err(anyhow!("Boot is not ready")))?,
            transport: match &container.transport {
                TransportMut::Inline => match container.boot.inline_data() {
                    Some(inline) => Transport::Inline(inline),
                    None => return Err(anyhow!("Could not find inline data from Boot").into()),
                },
                TransportMut::Receive(receive) => Transport::Receive(receive.clone()),
                TransportMut::Empty => unreachable!(), // This would only be possible if called from within `Container::pull`
            },
            tool_index: None,
            tools: vec![],
            root: proto.root.clone(),
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

    /// Sends the current wire protocol state to target
    pub fn send<T>(&'wire self, target: T) -> crate::Result<T>
    where
        T: VolumeTarget + AsyncRead + Send + BufMut,
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

        match Boot::decode(codec.snapshot()?) {
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
        if let Some(Tool::Framing(framing)) = self.tool_index.as_ref().and_then(|m| m.map.get(name))
        {
            let rec = Record::from_parts((self.info.clone(), self.transport.data().clone()));
            match framing(&rec) {
                Ok(framing) => {
                    self.transport = Transport::Frames {
                        list: framing,
                        data: self.transport.data().clone(),
                    }
                }
                Err(err) => {
                    error!("Could not use framing tool: {err}");
                }
            }
        } else {
            self.tools.push(name.intern());
        }
    }

    /// Enables a virtual index table w/ protocol
    #[inline]
    pub(crate) fn tool_index(&mut self) -> &mut ToolIndex {
        self.tool_index.get_or_insert_default()
    }

    /// Returns an installed tool or None if the tool is not installed
    #[inline]
    pub(crate) fn tool(&self, name: &str) -> Option<Tool> {
        self.tool_index
            .as_ref()
            .and_then(|i| i.map.get(name))
            .cloned()
    }

    /// Returns an iterator over the tools being used
    #[inline]
    pub(crate) fn tools(&self) -> impl Iterator<Item = (&str, Tool)> {
        self.tools
            .iter()
            .filter_map(|t| self.tool(t).map(|_t| (*t, _t)))
    }

    #[inline]
    fn apply_transport_inline(&'wire self, map: &mut MapBuilder<'wire>, framelist: &mut FrameList) {
        let frame = self.transport.data().apply(&self.ns, map);
        framelist.push(frame);
    }

    #[inline]
    fn apply_transport(&'wire self, framelist: &mut FrameList) {
        let transport = self.transport.describe(&self.ns);
        framelist.append(transport);
    }

    /// Apply tools
    #[inline]
    fn apply_tools(&'wire self, map: &mut MapBuilder<'wire>, framelist: &mut FrameList) {
        use crate::wire::tool::Tool::{Config, Framing, Indexer, Setting};

        if self.tools.is_empty() {
            return;
        }
        let mut builder = flexbuffers::Builder::default();
        let mut settings = builder.start_map();
        for (tool_name, tool) in self.tools() {
            match tool {
                Setting(setting) => {
                    debug!("Evaluated tool `.settings`, Setting(`{tool_name}`)");
                    settings.push(tool_name, setting.as_str());
                }
                Config(config) => {
                    debug!("Evaluated tool `.settings`, Confg(`{tool_name}`)");
                    settings.push(tool_name, Blob(config.as_slice()));
                }
                Indexer(indexer) => {
                    let _index = indexer(&Record::from_parts((
                        self.info.clone(),
                        self.transport.data().clone(),
                    )));

                    match _index {
                        Ok(index) => {
                            let mut opts = crate::Opts::from(Spec::Tool);
                            opts.set_object_storage(true);
                            let desc = Descriptor::create::<Sha256>(
                                &self.ns,
                                opts,
                                index.as_slice(),
                                tool_name,
                            );
                            map.push(&desc.label_idx_str(), Blob(index.as_slice()));
                            framelist.push(desc);
                            debug!("Evaluated tool `{tool_name}`");
                        }
                        Err(err) => {
                            error!("Skipping tool `{tool_name}`, error: {err}");
                        }
                    }
                }
                Framing(_) => {
                    // Already handled
                }
            }
        }
        settings.end_map();

        if !builder.view().is_empty() {
            let opts = crate::Opts::from(Spec::Tool);
            let desc = Descriptor::create::<Sha256>(&self.ns, opts, &builder.view(), ".settings");
            map.push(&desc.label_idx_str(), Blob(builder.view()));
            framelist.push(desc);
        }
    }
}
