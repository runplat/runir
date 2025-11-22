use serde::Serialize;

use crate::util::format_ext::ObjectFormat;

/// Formats an object into wire-unit format
#[inline]
fn wire_unit<'a, T: Serialize + Annotate>(object: &T, map: &mut flexbuffers::MapBuilder<'a>) {
    use sha2::Digest;
    use flexbuffers::Blob;

    let mut ser = flexbuffers::FlexbufferSerializer::new();
    let mut map = map.start_map(".runir");
    map.push("ts", time::UtcDateTime::now().unix_timestamp());
    map.push("type_name", T::type_name());
    match object.serialize(&mut ser) {
        Ok(()) => {
            let bytes = ser.view();
            map.push("size", bytes.len() as u64);
            if let Some("sha256") = T::config("cas") {
                let digest = sha2::Sha256::digest(bytes);
                map.push("sha256", Blob(digest.as_slice()));
            }
            map.push("object", Blob(bytes));
        }
        Err(err) => {
            map.push("err", err.to_string().as_str());
        }
    }
    map.end_map();
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
    /// config("cas") - enables content-address storage mode
    ///
    /// Supported values: sha256
    const CAS: &str = "";

    /// config("secret") - enables secret storage mode
    ///
    /// Indicates to co-operators the "secret" settings to use
    /// when transporting or storing this type
    ///
    /// Supported values: <TODO>
    const SECRET: &str = "";

    /// Returns the object format function to use when converting this type
    fn object_format() -> ObjectFormat<Self>
    where
        Self: Serialize + Sized,
    {
        /*
            This will create a nested map under the `.runir` key
            
            .runir : {
                type_name:
                size:
                object:
                error:
            }
        */
        wire_unit
    
        /*
            TODO: This opens up the possibility of other formats, ex:
            .meta : {
                ...
            }

            .connect : {
                ...
            }
        */
    }

    /// Returns a type name to represent this type
    fn type_name() -> &'static str;

    /// Returns a name template to use when formatting names
    /// for an instance of this type
    #[inline]
    fn name_template() -> Option<&'static str> {
        None
    }

    /// Returns true if SHA256 CAS is enabled for this type
    ///
    /// Cooperating Operators can use this internally to know whether to generate
    /// a content digest of this type's serialized forms, or whether to validate
    /// a content digest when deserializing this type's serialized form
    #[inline]
    fn is_sha256_cas_enabled() -> bool {
        matches!(Self::config("cas"), Some("sha256"))
    }

    /// Returns the value of a meta-configuration specified by this type
    #[inline]
    fn config(name: &str) -> Option<&'static str> {
        match name {
            "cas" if Self::CAS != "" => Some(Self::CAS),
            "secret" if Self::SECRET != "" => Some(Self::SECRET),
            _ => None,
        }
    }
}
