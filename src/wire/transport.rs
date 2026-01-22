use tracing::debug;

use crate::{
    Namespace,
    opts::Storage,
    util::PeekExtensions,
    wire::{Describe, Fetch, FrameList, cas::Descriptor, receive::Receive},
};

/// Variants of record data state
#[derive(Debug, Clone)]
pub enum Transport {
    /// Data is stored and received inline
    Inline(crate::Data),
    Frames {
        list: FrameList,
        data: crate::Data,
    },
    /// Data is being received
    Receive(Receive),
}

// pub type Frame = (FrameList, crate::Data);

impl Transport {
    /// Returns a reference to the underlying data in the transport
    #[inline]
    pub fn data(&self) -> &crate::Data {
        match self {
            Transport::Inline(data) => data,
            Transport::Receive(receive) => &receive.snapshot,
            Transport::Frames { data, .. } => data,
        }
    }

    /// Returns true if the transport stores data inline
    #[inline]
    pub fn is_inline(&self) -> bool {
        matches!(self, Transport::Inline(..))
    }
}

/// Mutable side of the transport
#[derive(Debug, Default)]
pub enum TransportMut {
    /// No mutable side is available
    #[default]
    Empty,
    /// Data was transported w/ header bytes
    Inline,
    /// Receiver
    Receive(Receive),
}

impl TransportMut {
    /// Returns true if transport mut is inline
    #[inline]
    pub fn is_inline(&self) -> bool {
        matches!(self, TransportMut::Inline)
    }

    /// Returns true if transport mut is receive
    #[inline]
    pub fn is_receive(&self) -> bool {
        matches!(self, TransportMut::Receive(..))
    }

    /// Returns transport as a receive
    #[inline]
    pub fn as_receive(&self) -> Option<&Receive> {
        match self {
            TransportMut::Empty => None,
            TransportMut::Inline => None,
            TransportMut::Receive(receive) => Some(receive),
        }
    }
}

impl<'wire> Describe for Transport {
    fn describe(&self, ns: &Namespace) -> FrameList {
        match self {
            Transport::Inline(data) => {
                let storage = if data.val().is_some() {
                    Storage::Object
                } else {
                    Storage::Content
                };
                FrameList {
                    frames: vec![Descriptor::create::<sha2::Sha256>(
                        ns,
                        storage.into(),
                        data,
                        "inline",
                    )],
                }
            }
            Transport::Receive(receive) => receive.list.clone(),
            Transport::Frames { list, .. } => list.clone(),
        }
    }
}

impl<'wire> Fetch<'wire> for Transport {
    fn fetch(&'wire self, ns: &Namespace, desc: &Descriptor) -> Option<&'wire [u8]> {
        match self {
            Transport::Inline(data) => data.fetch(ns, desc),
            Transport::Receive(receive) => {
                if let Some(frame) = receive.list.frames.iter().find(|f| *f == desc) {
                    receive.snapshot.fetch(ns, frame)
                } else {
                    debug!("Unknown descriptor {desc:?}");
                    None
                }
            }
            Transport::Frames { list, data } => {
                if let Some(frame) = list.frames.iter().find(|f| *f == desc) {
                    data.fetch(ns, frame)
                } else {
                    debug!("Unknown descriptor {desc:?}");
                    None
                }
            }
        }
    }
}
