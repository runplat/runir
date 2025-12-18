use tracing::debug;

use crate::{
    opts::Storage,
    util::PeekExtensions,
    wire::{Describe, Fetch, FrameList, cas::Descriptor, receive::Receive},
};

/// Variants of record data state
#[derive(Debug, Clone)]
pub enum Transport<'wire> {
    /// Data is stored and received inline
    Inline(crate::Data),
    /// Data is stored and received as a list of frames
    Frame(Frame<'wire>),
    /// Data is being received
    Receive(Receive<'wire>),
}

pub type Frame<'wire> = (FrameList<'wire>, crate::Data);

impl<'a> Transport<'a> {
    /// Returns a reference to the underlying data in the transport
    #[inline]
    pub fn data(&self) -> &crate::Data {
        match self {
            Transport::Inline(data) => data,
            Transport::Frame((_, data)) => data,
            Transport::Receive(receive) => &receive.data,
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
pub enum TransportMut<'wire> {
    /// No mutable side is available
    #[default]
    Empty,
    /// Data was transported w/ header bytes
    Inline,
    /// Receiver
    Receive(Receive<'wire>),
}

impl<'wire> TransportMut<'wire> {
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
    pub fn as_receive(&self) -> Option<&Receive<'wire>> {
        match self {
            TransportMut::Empty => None,
            TransportMut::Inline => None,
            TransportMut::Receive(receive) => Some(receive),
        }
    }
}

impl<'wire> Describe<'wire> for Transport<'wire> {
    fn describe(&'wire self) -> FrameList<'wire> {
        match self {
            Transport::Inline(data) => {
                let storage = if data.val().is_some() {
                    Storage::Object
                } else {
                    Storage::Content
                };
                FrameList {
                    frames: vec![Descriptor::create(storage.into(), data, "inline")],
                }
            }
            Transport::Frame((frames, _)) => frames.clone(),
            Transport::Receive(receive) => receive.list.clone(),
        }
    }
}

impl<'wire> Fetch<'wire> for Transport<'wire> {
    fn fetch(&'wire self, desc: &Descriptor<'_>) -> Option<&'wire [u8]> {
        match self {
            Transport::Inline(data) => Some(&data),
            Transport::Frame((frames, data)) => {
                if let Some(frame) = frames.frames.iter().find(|f| *f == desc) {
                    data.fetch(frame)
                } else {
                    debug!("Unknown descriptor {desc:?}");
                    None
                }
            }
            Transport::Receive(receive) => {
                if let Some(frame) = receive.list.frames.iter().find(|f| *f == desc) {
                    receive.data.fetch(frame)
                } else {
                    debug!("Unknown descriptor {desc:?}");
                    None
                }
            }
        }
    }
}
