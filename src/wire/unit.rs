use crate::{
    IRecord, Record,
    util::{PeekExtensions, format_ext::ApplyObject},
    wire::Annotate,
};
use bytes::Bytes;
use flexbuffers::Reader;
use serde::{Deserialize, Serialize};

pub trait ToWireUnit: IRecord {
    /// Formats a record (non-destructive) into a wire unit record
    ///
    /// No-op if the record is already in wire-unit format
    ///
    /// [".runir"] {
    ///     mode: transport|native
    ///     size:
    ///     ts:
    ///     type_name:
    ///     --
    ///     sha256: <Digest>
    ///     --
    ///     object:
    ///     --
    ///     error:
    ///     --
    /// }
    #[inline]
    fn to_wire_unit(&self) -> Record {
        if self.opts().is_wire_unit() && self.peek().at(".runir").is_some() {
            let mut record = self.to_record();
            record.opts.enable_wire_unit();
            return record;
        } else {
            /*
                Wire Unit format has two modes, transport and native.

                If the type stored in the Record uses Namespace::wire(..), this is native, and is handled in the above branch.

                Otherwise, the mode is considered transport, which has some subtle issues that need to be addressed.

                (
                    We could force everything to be one-mode, however we would lose the extensibility native mode brings,
                    and vice-versa, native mode requires co-operative coding so that would force use-cases to write code.
                )

                Either the record stores a flexbuffer root or bytes. In either case, we are wrapping record data,
                so that we have a uniform "unit" format.

                We can think of this wrapper as a "Transport" struct.

                Now the issues this brings are,

                a) Need to preserve the chucksum used by the record - since we have the flag enabled, we can adjust the validate function
                b) If the record was content addressable, it means that the digest of the data is the label of the record.
                   If we convert a CA record to generic wire-format, then we need to ensure that we calculate it's content digest from the transport bytes instead of the overall bytes
            */

            let transport = Transport {
                bytes: self.bytes(),
                opts: 0,
                reserved: 0,
            };

            let transport = flexbuffers::Builder::default()
                .apply_object(&transport)
                .take_buffer();

            let mut record = self.to_record();
            record.data = Bytes::from(transport).into();
            record.opts.enable_wire_unit();
            record
        }
    }
}

impl<R: IRecord> ToWireUnit for R {}

#[derive(Serialize, Deserialize, Debug)]
pub struct Transport<'b> {
    /// Options
    opts: u64,
    /// Reserved
    reserved: u64,
    /// Bytes being wrapped by the wire
    #[serde(serialize_with="serialize_bytes")]
    bytes: &'b [u8],
}

impl<'b> Transport<'b> {
    /// Returns a flexbuffer reader over the root
    #[inline]
    pub fn root(&self) -> Option<Reader<&'b [u8]>> {
        Reader::get_root(self.bytes).ok()
    }

    /// Returns the stored bytes in the transport
    #[inline]
    pub fn bytes(&self) -> &[u8] {
        self.bytes
    }
}

fn serialize_bytes<S>(bytes: &[u8], ser: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    ser.serialize_bytes(bytes)
}

impl<'b> Annotate for Transport<'b> {
    fn type_name() -> &'static str {
        "runir::wire::Transport"
    }
}

#[cfg(test)]
mod tests {
    use toml::toml;
    use crate::{IRecord, Namespace, wire::{Describe, Wire}};

    #[test]
    fn test_transport() {
        let ephemeral = Namespace::ephemeral();
        let record = ephemeral.author("test", |mut b| {
            b.start_map().push("value", "hello world");
            b
        });
        assert!(record.is_valid());
        let transport = record.transport();
        assert!(transport.is_valid());
        assert!(transport.opts().is_transport());
        assert_eq!(transport.index_key(), record.index_key());

        let record = ephemeral.store_content(&toml! {
            value = "hello world"
        });
        let record = record.unwrap().transport();
        assert!(record.is_valid());
        assert!(record.opts().is_transport());

        let wire = Wire::from(record.clone());
        // Test CAS trait
        let description = wire.describe();
        assert_eq!(description.object.size, 66);
        assert_eq!(description.object.digest, hex::decode("8146b7f5502455c6e317ac80aa7f85e85b2dfc14cead3622b3ff52ac9cdea90d").unwrap());

        let ns = Namespace::new("test");
        let transferred = ns.transfer(record.clone()).unwrap();
        assert!(transferred.is_valid());
        assert!(transferred.opts().is_transport());
        assert_ne!(transport.index_key(), transferred.index_key());
    }
}