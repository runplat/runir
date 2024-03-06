use std::sync::Arc;

use crate::define_intern_table;
use crate::prelude::*;
use crate::push_tag;

// Intern table for dependency names
define_intern_table!(DEPENDENCY_NAME: String);

// Intern table for parents of dependencies
define_intern_table!(DEPENDENCY_PARENT: Repr);

/// Dependency level contains tags identifying a dependency,
///
pub struct DependencyLevel {
    /// Parent of this dependency,
    ///
    parent: Option<Tag<Repr, Arc<Repr>>>,
    /// Name of the dependency,
    ///
    name: Tag<String, Arc<String>>,
}

impl DependencyLevel {
    /// Returns a new dependency level w/ name,
    ///
    #[inline]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            parent: None,
            name: Tag::new(&DEPENDENCY_NAME, Arc::new(name.into())),
        }
    }

    /// Sets the parent of this dependency,
    ///
    #[inline]
    pub fn with_parent(mut self, parent: Repr) -> Self {
        self.parent = Some(Tag::new(&DEPENDENCY_PARENT, Arc::new(parent)));
        self
    }
}

impl Level for DependencyLevel {
    fn configure(&self, interner: &mut impl InternerFactory) -> InternResult {
        if let Some(parent) = self.parent.as_ref() {
            push_tag!(dyn interner, parent);
        }
        push_tag!(dyn interner, &self.name);

        interner.set_level_flags(LevelFlags::LEVEL_1);

        interner.interner()
    }

    type Mount = (Option<Repr>, Arc<String>);

    #[inline]
    fn mount(&self) -> Self::Mount {
        (
            self.parent.as_ref().map(|p| p.value()),
            self.name.create_value.clone(),
        )
    }
}

/// Wrapper struct with access to dependency tags,
///
pub struct DependencyRepr(pub(crate) InternHandle);

impl DependencyRepr {
    /// Returns the name of this dependency,
    ///
    #[inline]
    pub fn name(&self) -> Option<Arc<String>> {
        self.0.dependency_name()
    }

    /// Returns the parent of this dependency,
    ///
    #[inline]
    pub fn parent(&self) -> Option<Repr> {
        self.0.dependency_parent()
    }
}
