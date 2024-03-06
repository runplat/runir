use std::any::TypeId;
use std::str::FromStr;

use crate::define_intern_table;
use crate::prelude::*;
use crate::push_tag;

use super::resource::FFI;

// Intern table for owner type ids
define_intern_table!(OWNER_ID: TypeId);

// Intern table for owner names
define_intern_table!(OWNER_NAME: &'static str);

// Intern table for owner type sizes
define_intern_table!(OWNER_SIZE: usize);

// Intern table for field offsets
define_intern_table!(FIELD_OFFSET: usize);

// Intern table for field names
define_intern_table!(FIELD_NAME: &'static str);

/// Trait allowing a type to identify one of it's fields by offset,
///
pub trait Field<const OFFSET: usize>: Send + Sync + 'static {
    /// Associated type that implements FromStr and is the resulting type
    /// when a field has been parsed for this field,
    ///
    type ParseType: FromStr + Send + Sync + 'static;

    /// Associated type that is projected by the implementing type for this field,
    ///
    /// **TODO**: By default this can be the same as the parse type. If associated type defaults
    /// existed, then the default would just be the ParseType.
    ///
    type ProjectedType: Send + Sync + 'static;

    /// Type to use to represent this field over FFI boundaries,
    ///
    /// **Note** When derived will default to `unit` which can only communicate existance.
    ///
    type FFIType: FFI + Send + Sync + 'static;

    /// Name of the field,
    ///
    fn field_name() -> &'static str;

    /// Creates and returns a linker for this field,
    ///
    fn linker<I: InternerFactory + Default>() -> anyhow::Result<Linker<I>>
    where
        Self: Sized,
    {
        let mut linker = Linker::<I>::default();

        let mut resource = ResourceLevel::new::<Self::ProjectedType>();
        resource.set_parse_type::<Self::ParseType>();
        resource.set_ffi::<Self::FFIType>();

        linker.push_level(resource)?;
        linker.push_level(FieldLevel::new::<OFFSET, Self>())?;

        Ok(linker)
    }
}

/// Field level is the next level of representation,
///
/// Field level asserts the relationship between some owning resource and a field
/// this resource owns.
///
#[derive(Clone, Copy)]
pub struct FieldLevel {
    /// Owner type id,
    ///
    owner_type_id: Tag<TypeId>,
    /// Owner type name,
    ///
    owner_name: Tag<&'static str>,
    /// Owner size,
    ///
    owner_size: Tag<usize>,
    /// Field offset,
    ///
    field_offset: Tag<usize>,
    /// Field name,
    ///
    field_name: Tag<&'static str>,
}

impl FieldLevel {
    /// Creates a new field level representation,
    ///
    pub fn new<const OFFSET: usize, Owner>() -> Self
    where
        Owner: Field<OFFSET> + Send + Sync + 'static,
    {
        Self {
            owner_type_id: Tag::new(&OWNER_ID, std::any::TypeId::of::<Owner>),
            owner_name: Tag::new(&OWNER_NAME, std::any::type_name::<Owner>),
            owner_size: Tag::new(&OWNER_SIZE, std::mem::size_of::<Owner>),
            field_offset: Tag::new(&FIELD_OFFSET, || OFFSET),
            field_name: Tag::new(&FIELD_NAME, Owner::field_name),
        }
    }
}

impl Level for FieldLevel {
    fn configure(&self, interner: &mut impl InternerFactory) -> InternResult {
        push_tag!(interner, self.owner_type_id);
        push_tag!(interner, self.owner_name);
        push_tag!(interner, self.owner_size);
        push_tag!(interner, self.field_offset);
        push_tag!(interner, self.field_name);

        interner.set_level_flags(LevelFlags::LEVEL_1);

        interner.interner()
    }

    type Mount = (TypeId, &'static str, usize, usize, &'static str);

    #[inline]
    fn mount(&self) -> Self::Mount {
        (
            self.owner_type_id.value(),
            self.owner_name.value(),
            self.owner_size.value(),
            self.field_offset.value(),
            self.field_name.value(),
        )
    }
}

/// Wrapper struct to access field tags,
///
pub struct FieldRepr(pub(crate) InternHandle);

impl FieldRepr {
    /// Returns the tag value of the field name,
    ///
    #[inline]
    pub fn name(&self) -> Option<&'static str> {
        self.0.field_name()
    }

    /// Returns the tag value of the field offset,
    ///
    #[inline]
    pub fn offset(&self) -> Option<usize> {
        self.0.field_offset()
    }

    /// Returns the tag value of the owner type name,
    ///  
    #[inline]
    pub fn owner_name(&self) -> Option<&'static str> {
        self.0.owner_name()
    }

    /// Returns the tag value of the owner type size,
    ///
    #[inline]
    pub fn owner_size(&self) -> Option<usize> {
        self.0.owner_size()
    }

    /// Returns the tag value of the owner's type id,
    ///
    #[inline]
    pub fn owner_type_id(&self) -> Option<TypeId> {
        self.0.owner_type_id()
    }
}
