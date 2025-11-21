/// Enables consistent self-annotation for supported types
pub trait Annotate {
    /// Returns a type name to represent this type
    fn type_name() -> &'static str;

    /// Enables annotating meta-level settings of the type
    #[inline]
    fn value(key: &str) -> Option<&'static str> {
        match key {
            _ => None
        }
    }
}