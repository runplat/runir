use serde::Serialize;

use crate::util::format_ext::{FormatObject, ReadObject};

/// Formats an object into wire-unit format
#[inline]
fn wire_unit<'a, T: Serialize + Annotate>(object: &T, map: &mut flexbuffers::MapBuilder<'a>) {
    use flexbuffers::Blob;
    use sha2::Digest;

    let mut ser = flexbuffers::FlexbufferSerializer::new();
    let mut map = map.start_map(".runir");
    map.push("ts", time::UtcDateTime::now().unix_timestamp());
    map.push("type_name", T::type_name());
    match object.serialize(&mut ser) {
        Ok(()) => {
            let bytes = ser.view();
            map.push("size", bytes.len() as u64);
            let digest = sha2::Sha256::digest(bytes);
            map.push("sha256", Blob(digest.as_slice()));
            map.push("object", Blob(bytes));
        }
        Err(err) => {
            map.push("err", err.to_string().as_str());
        }
    }
    map.end_map();
}

/// Advances the position of the current reader to the map w/ the wire-unit data
#[inline]
fn read_wire_unit<T, B>(reader: flexbuffers::Reader<B>) -> Option<flexbuffers::Reader<B>>
where
    T: Annotate,
    B: flexbuffers::Buffer,
    B::BufferString: AsRef<str>,
{
    Some(reader.as_map().idx(".runir")).filter(|r| {
        r.flexbuffer_type().is_map()
            && r.as_map().idx("type_name").as_str().as_ref() == T::type_name()
    })
}

/// Enables self-annotation of types programatically
///
/// This is a meta-programming axis, that enables writing
/// code that can systematically enable derive features defined
/// by the author of the type.
///
/// For example, if a type is to be transported over a wire protocol,
/// and restored, it's useful to know if CAS should be enabled. But it's
/// possible not all types transported over a wire protocol require CAS.
///
/// By implementing this trait, a type can communicate this type of
/// information to "co-operators" (code that co-operates with a plan
/// or design).
///
/// Another example, is type_name, which is supported by Rust,
/// however cannot be used outside of diagnostic purposes
pub trait Annotate {
    /// Returns a type name to represent this type
    fn type_name() -> &'static str;

    /// Returns the format object function to use when converting this type
    #[inline]
    fn object_format() -> FormatObject<Self>
    where
        Self: Serialize + Sized,
    {
        Self::default_object_format()
    }

    /// Returns the object reader function
    #[inline]
    fn object_reader<B>() -> ReadObject<B>
    where
        Self: Sized,
        B: flexbuffers::Buffer,
        B::BufferString: AsRef<str>,
    {
        Self::default_object_reader()
    }

    /// Returns the format object function to use when converting this type
    #[inline]
    fn default_object_format() -> FormatObject<Self>
    where
        Self: Serialize + Sized,
    {
        /*
            This will create a nested map under the `.runir` key
            {
                .runir : {
                    type_name:
                    size:
                    object:
                    error:
                }
            }
        */
        wire_unit
    }

    /// Returns the object reader function
    #[inline]
    fn default_object_reader<B>() -> ReadObject<B>
    where
        Self: Sized,
        B: flexbuffers::Buffer,
        B::BufferString: AsRef<str>,
    {
        read_wire_unit::<Self, B>
    }

    /// Returns the value of a meta-configuration specified by this type
    #[inline]
    fn config(name: &str) -> Option<&'static str> {
        match name {
            _ => None,
        }
    }
}
