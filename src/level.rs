use crate::interner::{InternResult, InternerFactory};

/// Trait for each level of representation that defines how
/// each level configures the intern handle representing a resource.
///
pub trait Level {
    /// Return type of Self::mount()
    ///
    type Mount;

    /// Configures the representation state,
    ///
    fn configure(&self, interner: &mut impl InternerFactory) -> InternResult;

    /// "Mounts" the current level and returns the current tag state,
    ///
    fn mount(&self) -> Self::Mount;
}
