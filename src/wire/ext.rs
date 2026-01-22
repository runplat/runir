use crate::{
    Data, Namespace, RecordInfo, opts::Storage, util::PeekExtensions, wire::cas::Descriptor
};
use flexbuffers::Blob;
use sha2::Sha256;

/// Allows injecting extensions into the wire format
pub trait Ext {
    /// Returns the name of the extension
    fn name(&self) -> &str;

    /// Authors the extension data to the map
    fn author(&self, map: &mut flexbuffers::MapBuilder<'_>);

    /// Applies this extension to a map
    ///
    /// Returns a descriptor for the data that was written
    #[inline]
    fn apply(&self, ns: &Namespace, map: &mut flexbuffers::MapBuilder<'_>) -> Descriptor
    where
        Self: Sized,
    {
        self.default_apply(ns, map)
    }

    #[inline]
    fn default_apply(&self, ns: &Namespace, map: &mut flexbuffers::MapBuilder<'_>) -> Descriptor {
        let mut builder = flexbuffers::Builder::default();
        let mut ext = builder.start_map();
        self.author(&mut ext);
        ext.end_map();
        let desc = Descriptor::create::<Sha256>(ns, Storage::Object.into(), builder.view(), self.name());
        map.push(&desc.label_idx_str(), Blob(builder.take_buffer().as_slice()));
        desc
    }
}

impl Ext for RecordInfo {
    #[inline]
    fn name(&self) -> &str {
        ".runir"
    }

    #[inline]
    fn author(&self, map: &mut flexbuffers::MapBuilder<'_>) {
        let (uuid, ns_chk, ts, opts) = self.to_parts();
        let (label, checksum) = uuid.as_u64_pair();
        map.push("label", label);
        map.push("checksum", checksum);
        map.push("ns_chk", ns_chk);
        map.push("ts", ts);
        map.push("opts", opts.encode());
    }

    fn apply(&self, ns: &Namespace, map: &mut flexbuffers::MapBuilder<'_>) -> Descriptor
    where
        Self: Sized,
    {
        let mut desc = self.default_apply(ns, map);
        desc.opts.set_info_spec(true);
        desc
    }
}

impl Ext for Data {
    fn name(&self) -> &str {
        ".data"
    }

    fn author(&self, _: &mut flexbuffers::MapBuilder<'_>) {}

    fn apply(&self, ns: &Namespace, map: &mut flexbuffers::MapBuilder<'_>) -> Descriptor {
        let storage = if self.val().is_some() {
            Storage::Object
        } else {
            Storage::Content
        };
        let desc = Descriptor::create::<Sha256>(ns, storage.into(), self.as_ref(), ".data");
        map.push(&desc.label_idx_str(), Blob(self.as_ref()));
        desc
    }
}
