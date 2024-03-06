use std::any::TypeId;
use std::fs::File;
use std::path::PathBuf;

use crate::define_intern_table;
use crate::push_tag;

use crate::prelude::*;

// Intern table for resource type names
define_intern_table!(TYPE_NAME: &'static str);

// Intern table for resource type sizes
define_intern_table!(TYPE_SIZE: usize);

// Intern table for resource type ids
define_intern_table!(TYPE_ID: TypeId);

// Intern table for resource parse type names
define_intern_table!(PARSE_TYPE_NAME: &'static str);

// Intern table for ffi type name
define_intern_table!(FFI_TYPE_NAME: &'static str);

// Intern table for ffi value parser
#[cfg(feature = "util-clap")]
define_intern_table!(FFI_VALUE_PARSER: Option<clap::builder::Resettable<clap::builder::ValueParser>>);

/// Resource level is the lowest level of representation,
///
/// Resource level asserts compiler information for the resource.
///
#[derive(Clone)]
pub struct ResourceLevel {
    /// Rust type id assigned by the compiler,
    ///
    type_id: Tag<TypeId>,
    /// Rust type name assigned by the compiler,
    ///
    type_name: Tag<&'static str>,
    /// Type size assigned by the compiler,
    ///
    type_size: Tag<usize>,
    /// Rust type name of the type used to parse node input,
    ///
    parse_type: Option<Tag<&'static str>>,
    /// (Optional) FFI type name,
    ///
    ffi_type: Option<Tag<&'static str>>,
    /// (Optional) FFI clap value parser,
    ///
    /// **Note** Requires `util-clap` feature
    ///
    #[cfg(feature = "util-clap")]
    ffi_value_parser: Option<Tag<Option<clap::builder::Resettable<clap::builder::ValueParser>>>>,
}

impl ResourceLevel {
    /// Creates a new type level representation,
    ///
    #[inline]
    pub fn new<T: Send + Sync + 'static>() -> Self {
        Self {
            type_id: Tag::new(&TYPE_ID, std::any::TypeId::of::<T>),
            type_name: Tag::new(&TYPE_NAME, std::any::type_name::<T>),
            type_size: Tag::new(&TYPE_SIZE, std::mem::size_of::<T>),
            parse_type: None,
            ffi_type: None,
            #[cfg(feature = "util-clap")]
            ffi_value_parser: None,
        }
    }

    /// Sets the resource parse type,
    ///
    #[inline]
    pub fn set_parse_type<T>(&mut self) {
        self.parse_type = Some(Tag::new(&PARSE_TYPE_NAME, std::any::type_name::<T>));
    }

    /// Sets the ffi type name,
    ///
    #[inline]
    pub fn set_ffi<T: FFI>(&mut self) {
        self.ffi_type = Some(Tag::new(&FFI_TYPE_NAME, T::ffi_type_name));

        #[cfg(feature = "util-clap")]
        {
            self.ffi_value_parser = Some(Tag::new(&FFI_VALUE_PARSER, T::value_parser))
        }
    }
}

impl Level for ResourceLevel {
    fn configure(&self, interner: &mut impl InternerFactory) -> InternResult {
        push_tag!(interner, self.type_id);
        push_tag!(interner, self.type_size);
        push_tag!(interner, self.type_name);

        if let Some(parse_type) = self.parse_type {
            push_tag!(interner, parse_type);
        }

        if let Some(ffi_type_name) = self.ffi_type {
            push_tag!(interner, ffi_type_name);
        }

        #[cfg(feature = "util-clap")]
        if let Some(ffi_value_parser) = self.ffi_value_parser.clone() {
            let ffi_vp_key = format!("{}_value_parser", self.type_name.value());
            push_tag!(as ffi_vp_key, interner, ffi_value_parser);
        }

        interner.set_level_flags(LevelFlags::ROOT);

        interner.interner()
    }

    type Mount = (TypeId, &'static str, usize);

    #[inline]
    fn mount(&self) -> Self::Mount {
        (
            self.type_id.value(),
            self.type_name.value(),
            self.type_size.value(),
        )
    }
}

/// Wrapper struct to access resource tags,
///
pub struct ResourceRepr(pub(crate) InternHandle);

impl ResourceRepr {
    /// Returns true if resource matches type,
    ///
    pub fn is_type<T: 'static>(&self) -> bool {
        self.type_name()
            .filter(|n| *n == std::any::type_name::<T>())
            .is_some()
            && self
                .type_id()
                .filter(|n| *n == std::any::TypeId::of::<T>())
                .is_some()
    }

    /// Returns true if the resource parse type matches,
    ///
    pub fn is_parse_type<T: 'static>(&self) -> bool {
        self.parse_type_name()
            .filter(|n| *n == std::any::type_name::<T>())
            .is_some()
    }

    /// Returns true if the resource parse type matches,
    ///
    pub fn is_ffi_type<T: FFI + 'static>(&self) -> bool {
        self.ffi_type_name()
            .filter(|n| *n == T::ffi_type_name())
            .is_some()
    }

    /// Returns the tag value of the resource type name,
    ///
    #[inline]
    pub fn type_name(&self) -> Option<&'static str> {
        self.0.resource_type_name()
    }

    /// Returns the tag value of the resource type size,
    ///
    #[inline]
    pub fn type_size(&self) -> Option<usize> {
        self.0.resource_type_size()
    }

    /// Returns the tage value of the resource type id,
    ///
    #[inline]
    pub fn type_id(&self) -> Option<TypeId> {
        self.0.resource_type_id()
    }

    /// Returns the tag value of the resource parse type name,
    ///
    #[inline]
    pub fn parse_type_name(&self) -> Option<&'static str> {
        self.0.resource_parse_type_name()
    }

    /// Returns the FFI type name,
    ///
    #[inline]
    pub fn ffi_type_name(&self) -> Option<&'static str> {
        self.0.resource_ffi_type_name()
    }

    /// Returns the FFI clap value parser,
    ///
    #[inline]
    #[cfg(feature = "util-clap")]
    pub fn ffi_value_parser(
        &self,
    ) -> Option<clap::builder::Resettable<clap::builder::ValueParser>> {
        self.0.resource_ffi_value_parser()
    }
}

/// Trait to provide foreign function interface support,
///
pub trait FFI {
    /// FFI type name,
    ///
    fn ffi_type_name() -> &'static str;

    /// Clap value parser to use w/ this resource type,
    ///
    #[cfg(feature = "util-clap")]
    fn value_parser() -> Option<clap::builder::Resettable<clap::builder::ValueParser>>;
}

impl FFI for () {
    fn ffi_type_name() -> &'static str {
        "unit"
    }

    #[cfg(feature = "util-clap")]
    fn value_parser() -> std::option::Option<clap::builder::Resettable<clap::builder::ValueParser>>
    {
        None
    }
}

impl FFI for String {
    fn ffi_type_name() -> &'static str {
        "string"
    }

    #[cfg(feature = "util-clap")]
    fn value_parser() -> Option<clap::builder::Resettable<clap::builder::ValueParser>> {
        use clap::builder::IntoResettable;

        Some(clap::value_parser!(Self).into_resettable())
    }
}

impl FFI for PathBuf {
    fn ffi_type_name() -> &'static str {
        "path_buf"
    }

    #[cfg(feature = "util-clap")]
    fn value_parser() -> Option<clap::builder::Resettable<clap::builder::ValueParser>> {
        use clap::builder::IntoResettable;

        Some(clap::value_parser!(Self).into_resettable())
    }
}

impl FFI for File {
    fn ffi_type_name() -> &'static str {
        "file"
    }

    #[cfg(feature = "util-clap")]
    fn value_parser() -> Option<clap::builder::Resettable<clap::builder::ValueParser>> {
        None
    }
}

impl FFI for bool {
    fn ffi_type_name() -> &'static str {
        "bool"
    }

    #[cfg(feature = "util-clap")]
    fn value_parser() -> Option<clap::builder::Resettable<clap::builder::ValueParser>> {
        use clap::builder::IntoResettable;

        Some(clap::value_parser!(Self).into_resettable())
    }
}

impl FFI for u8 {
    fn ffi_type_name() -> &'static str {
        "u8"
    }

    #[cfg(feature = "util-clap")]
    fn value_parser() -> Option<clap::builder::Resettable<clap::builder::ValueParser>> {
        use clap::builder::IntoResettable;

        Some(clap::value_parser!(Self).into_resettable())
    }
}

impl FFI for u16 {
    fn ffi_type_name() -> &'static str {
        "u16"
    }

    #[cfg(feature = "util-clap")]
    fn value_parser() -> Option<clap::builder::Resettable<clap::builder::ValueParser>> {
        use clap::builder::IntoResettable;

        Some(clap::value_parser!(Self).into_resettable())
    }
}

impl FFI for u32 {
    fn ffi_type_name() -> &'static str {
        "u32"
    }

    #[cfg(feature = "util-clap")]
    fn value_parser() -> Option<clap::builder::Resettable<clap::builder::ValueParser>> {
        use clap::builder::IntoResettable;

        Some(clap::value_parser!(Self).into_resettable())
    }
}

impl FFI for u64 {
    fn ffi_type_name() -> &'static str {
        "u64"
    }

    #[cfg(feature = "util-clap")]
    fn value_parser() -> Option<clap::builder::Resettable<clap::builder::ValueParser>> {
        use clap::builder::IntoResettable;

        Some(clap::value_parser!(Self).into_resettable())
    }
}

impl FFI for i8 {
    fn ffi_type_name() -> &'static str {
        "i8"
    }

    #[cfg(feature = "util-clap")]
    fn value_parser() -> Option<clap::builder::Resettable<clap::builder::ValueParser>> {
        use clap::builder::IntoResettable;

        Some(clap::value_parser!(Self).into_resettable())
    }
}

impl FFI for i16 {
    fn ffi_type_name() -> &'static str {
        "i16"
    }

    #[cfg(feature = "util-clap")]
    fn value_parser() -> Option<clap::builder::Resettable<clap::builder::ValueParser>> {
        use clap::builder::IntoResettable;

        Some(clap::value_parser!(Self).into_resettable())
    }
}

impl FFI for i32 {
    fn ffi_type_name() -> &'static str {
        "i32"
    }

    #[cfg(feature = "util-clap")]
    fn value_parser() -> Option<clap::builder::Resettable<clap::builder::ValueParser>> {
        use clap::builder::IntoResettable;

        Some(clap::value_parser!(Self).into_resettable())
    }
}

impl FFI for i64 {
    fn ffi_type_name() -> &'static str {
        "i64"
    }

    #[cfg(feature = "util-clap")]
    fn value_parser() -> Option<clap::builder::Resettable<clap::builder::ValueParser>> {
        use clap::builder::IntoResettable;

        Some(clap::value_parser!(Self).into_resettable())
    }
}

impl FFI for f32 {
    fn ffi_type_name() -> &'static str {
        "f32"
    }

    #[cfg(feature = "util-clap")]
    fn value_parser() -> Option<clap::builder::Resettable<clap::builder::ValueParser>> {
        use clap::builder::IntoResettable;

        Some(clap::value_parser!(Self).into_resettable())
    }
}

impl FFI for f64 {
    fn ffi_type_name() -> &'static str {
        "f64"
    }

    #[cfg(feature = "util-clap")]
    fn value_parser() -> Option<clap::builder::Resettable<clap::builder::ValueParser>> {
        use clap::builder::IntoResettable;

        Some(clap::value_parser!(Self).into_resettable())
    }
}
