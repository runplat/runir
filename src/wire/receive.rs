use crate::{
    Namespace, vol::{MemoryMappedTarget, VolumeTarget, new_mmap_anon_target}, wire::{Fetch, FrameList, cas::Descriptor}
};
use bytes::{BufMut, BytesMut};
use tracing::error;

pub struct Receive {
    ns: Namespace,
    /// Currently received data
    pub(crate) data: crate::Data,
    /// List of frames that must be received
    pub(crate) list: FrameList,
    /// Last frame from list received
    pub(crate) last: usize,
    /// Volume storing the received bytes
    writer: Option<MemoryMappedTarget>,
}

/// Errors definition that can occur when decoding data to receive
#[derive(Debug)]
pub enum ReceiveError {
    /// Receive needs more bytes to continue
    ///
    /// Can be handled by pulling more data into the buffer
    Pull(usize),
    /// Current frame in buffer did not match the current frame descriptor
    ///
    /// Can be handled by putting the right data on the buffer.
    ///
    /// Ideally, business logic doesn't have this issue, but theoretically, this
    /// could be used to fuzzy match input buffers to match the receive with the buffer.
    InvalidFrame,
    /// An unexpected error occured and caused a fault
    Fault(crate::Error),
}

type Result<T> = std::result::Result<T, ReceiveError>;

impl From<crate::Error> for ReceiveError {
    fn from(value: crate::Error) -> Self {
        Self::Fault(value)
    }
}

impl From<std::io::Error> for ReceiveError {
    fn from(value: std::io::Error) -> Self {
        Self::Fault(crate::Error::from(value))
    }
}

impl<'wire> Clone for Receive {
    fn clone(&self) -> Self {
        Self {
            ns: self.ns.clone(),
            data: self.data.clone(),
            list: self.list.clone(),
            writer: None,
            last: self.last.clone(),
        }
    }
}

impl Receive {
    /// Creates a new Receive for a manifest
    ///
    /// Returns an error if a volume to receive data could not be created
    #[inline]
    pub fn new(ns: &Namespace, manifest: FrameList) -> crate::Result<Self> {
        /*
           TODO:
           - Fallback to BytesMut
           - Need to add guard against manifest.required_capacity() since that is user-input (malicious or otherwise)
        */
        let map = new_mmap_anon_target("", manifest.required_capacity() as usize)?;
        Ok(Self {
            ns: ns.clone(),
            data: crate::Data::default(),
            list: manifest,
            writer: Some(map),
            last: 0,
        })
    }

    /// Returns the length of bytes received
    #[inline]
    pub fn len(&self) -> usize {
        self.writer.as_ref().map(|v| v.len()).unwrap_or_default()
    }

    /// Commits the current snapshot into a target
    #[inline]
    pub fn commit(&self, target: &mut [u8]) -> crate::Result<()> {
        let required_capacity = self.list.required_capacity();
        if target.len() >= required_capacity as usize {
            // TODO: Return an error because the target was too short
        }

        if self.data.len() <= target.len() {
            if let Some(target) = target.get_mut(..self.data.len()) {
                let slice: &[u8] = &target;
                if slice == self.data.as_ref() {
                    // Skip, committing if the destination already matches the snapshot
                    return Ok(());
                }
                target.copy_from_slice(&self.data);
            } else {
                unreachable!(
                    "Must return a mutable slice because bounds were checked before get_mut was called"
                )
            }
        } else {
            // TODO: Return an error because the target was too short
        }
        Ok(())
    }

    /// Returns true if all bytes have been received
    #[inline]
    pub fn is_complete(&self) -> bool {
        self.data.len() == self.list.required_capacity() as usize
    }

    /// Returns the next frame waiting to be received
    #[inline]
    pub fn view_next_frame(&self) -> Option<(usize, Descriptor)> {
        let next_frame = if self.data.is_empty() {
            self.last
        } else {
            self.last + 1
        };
        self.list
            .frames
            .get(next_frame)
            .cloned()
            .map(|f| (next_frame, f))
    }

    /// Decodes the next frame from the buffer
    pub fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Descriptor>> {
        match self.view_next_frame() {
            Some((next_frame, next)) => {
                // 2) Check if buf has the next frame (Allow partial writes?)
                if buf.len() >= next.size as usize {
                    let view = &buf[..next.size as usize];
                    let mut desc = Descriptor::recover(
                        &self.ns,
                        next.opts,
                        view,
                        next.label,
                    );
                    desc.offset = next.offset;
                    if desc == next {
                        let view = buf.split_to(desc.size as usize);

                        // 3) Copy bytes from buf to volume
                        self.writer
                            .as_mut()
                            .expect("MUST have write to decode a buffer")
                            .put(view.as_ref());

                        // 4) Update data pointer in Receive
                        self.data = self
                            .writer
                            .as_mut()
                            .expect("MUST have write to decode a buffer")
                            .snapshot()?;

                        // 5) Update last decoded frame
                        self.last = next_frame;

                        Ok(Some(next))
                    } else {
                        error!("Received an invalid frame:\n{desc:#?}\nExpected: {next:#?}");
                        return Err(ReceiveError::InvalidFrame);
                    }
                } else {
                    return Err(ReceiveError::Pull(next.size as usize));
                }
            }
            None => {
                // Volume is completely decoded
                Ok(None)
            }
        }
    }
}

impl<'wire> Fetch<'wire> for Receive {
    fn fetch(&'wire self, ns: &Namespace, desc: &Descriptor) -> Option<&'wire [u8]> {
        self.data.fetch(ns, desc)
    }
}

impl<'wire> std::fmt::Debug for Receive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Receive").field("data", &self.data).finish()
    }
}

#[cfg(test)]
mod tests {
    use bytes::{BufMut, BytesMut};

    use crate::{
        Data, Namespace,
        test::test_cas_record,
        util::PeekExtensions,
        vol::VolumeTarget,
        wire::{Boot, Fetch, Wire, boot::NS_HEADER_INLINE_BLOCK_SIZE, receive::Receive},
    };

    #[test]
    #[tracing_test::traced_test]
    fn test_receive() {
        let ns = Namespace::new("test");
        let wire = Wire::new(ns.clone());
        let target = wire.encode(test_cas_record(), None).unwrap();

        let bytes = target.filled();
        assert_eq!(bytes.len(), NS_HEADER_INLINE_BLOCK_SIZE);

        // Test decoding boot
        let boot = Boot::decode(&bytes).unwrap();
        let list = boot.layout();

        // Note: Receive is typically only used when the boot is not inline
        // However, it MUST work with any valid list
        let mut recv = Receive::new(&ns, list.clone()).unwrap();

        let mut buf = BytesMut::new();
        for f in list.frames.iter() {
            let bytes = boot.fetch(&ns, f).unwrap();
            buf.put(bytes);
        }

        recv.decode(&mut buf).unwrap();
        recv.decode(&mut buf).unwrap();
        assert!(recv.is_complete());

        let mut target = BytesMut::zeroed(recv.data.len());
        recv.commit(&mut target.get_mut(..recv.data.len()).unwrap())
            .unwrap();

        let data: Data = target.freeze().into();
        let val = data
            .fetch(&ns, &recv.list.frames[0])
            .unwrap()
            .val()
            .unwrap();
        eprintln!("{val}");
        let val = data
            .fetch(&ns, &recv.list.frames[1])
            .unwrap()
            .val()
            .unwrap();
        eprintln!("{val}");
    }
}
