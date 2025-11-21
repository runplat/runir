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
    fn is_sha256_cas_enabled(&self) -> bool {
        matches!(Self::config("cas"), Some("sha256"))
    }

    /// Returns the value of a meta-configuration specified by this type
    #[inline]
    fn config(name: &str) -> Option<&'static str> {
        match name {
            "cas" if Self::CAS != "" => {
                Some(Self::CAS)
            },
            "secret" if Self::SECRET != "" => {
                Some(Self::SECRET)
            }
            _ => None
        }
    }
}
