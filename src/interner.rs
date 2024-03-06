use std::any::TypeId;
use std::collections::BTreeMap;
use std::hash::Hash;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::Weak;

use anyhow::anyhow;
use serde::Deserialize;
use serde::Serialize;
use tracing::trace;
use tracing::warn;

use crate::entity::ENTITY;
use crate::entropy::ENTROPY;
use crate::prelude::Repr;
use crate::repr::node::SourceSpan;

pub type InternResult = anyhow::Result<InternHandle>;

/// This trait is based on the concept of string interning where the
/// goal is to store distinct string values.
///
/// This trait applies that same concept to active references to software
/// resources. It is used to define the behavior when dealing w/ resource keys
/// assigned to resources in the storage target.
///
pub trait InternerFactory {
    /// Pushes a tag to the current interner state,
    ///
    fn push_tag<T: Hash + Send + Sync + 'static>(
        &mut self,
        value: T,
        assign: impl Fn(InternHandle) -> anyhow::Result<()> + Send + Sync + 'static,
    );

    /// Sets the current level flags for the interner,
    ///
    /// **Note**: The flag should be cleared when interner is called
    ///
    fn set_level_flags(&mut self, flags: LevelFlags);

    /// Sets the current data value for the interner,
    ///
    /// **Note**: The data value will be clearned when interner is called
    ///
    fn set_data(&mut self, data: u64);

    /// Finishes generating the current intern handle and consumes the current stack of tags,
    ///
    fn interner(&mut self) -> InternResult;
}

/// Handle which can be converted into a 64-bit key,
///
#[derive(
    Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct InternHandle {
    /// Link value,
    ///
    pub(crate) link: u32,
    /// Upper register,
    ///
    /// **Note on CrcInterner impl**: The first half of the upper register contains the level bits.
    ///
    pub(crate) register_hi: u16,
    /// Lower register,
    ///
    pub(crate) register_lo: u16,
    /// Data register,
    ///
    pub(crate) data: u64,
}

impl From<u64> for InternHandle {
    fn from(value: u64) -> Self {
        let u = uuid::Uuid::from_u64_pair(value, 0);

        let (link, register_hi, register_lo, _) = u.as_fields();

        Self {
            link,
            register_hi,
            register_lo,
            data: ENTROPY.get(),
        }
    }
}

impl InternHandle {
    /// Returns the current data value,
    ///
    pub fn data(&self) -> u64 {
        self.data ^ ENTROPY.get()
    }

    /// Returns the current entity id of the intern handle,
    ///
    pub fn entity(&self) -> Option<u64> {
        let data = self.data();

        ENTITY.copy(self).filter(|v| *v == data).map(|_| data)
    }

    /// Returns the current level flag enabled for this intern handle,
    ///
    #[inline]
    pub fn level_flags(&self) -> LevelFlags {
        LevelFlags::from_bits_truncate(self.register_hi)
    }

    /// Converts the handle to a u64 value,
    ///
    /// **Note**: This contains the full handle value.
    ///
    #[inline]
    pub fn as_u64(&self) -> u64 {
        self.as_uuid().as_u64_pair().0
    }

    /// Returns as a uuid,
    ///
    #[inline]
    pub fn as_uuid(&self) -> uuid::Uuid {
        uuid::Uuid::from_fields(self.link, self.register_hi, self.register_lo, &[0; 8])
    }

    /// Returns the register value of the current handle,
    ///
    #[inline]
    pub fn register(&self) -> u32 {
        bytemuck::cast::<[u16; 2], u32>([self.register_lo, self.register_hi])
    }

    /// Returns true if the current handle is a root handle,
    ///
    #[inline]
    pub fn is_root(&self) -> bool {
        self.level_flags() == LevelFlags::ROOT
    }

    /// Returns true if the current handle is a node handle,
    ///
    /// **Note** A node handle contains a non-zero link value.
    ///
    #[inline]
    pub fn is_node(&self) -> bool {
        self.link > 0
    }

    /// Returns a split view of the current intern handle providing the current and previous nodes,
    ///
    pub fn node(&self) -> (Option<InternHandle>, InternHandle) {
        let prev = self.link ^ self.register();

        let [lo, hi] = bytemuck::cast::<u32, [u16; 2]>(prev);

        let prev_level = LevelFlags::from_bits_truncate(hi);

        let mut prev_handle = None;
        if prev_level.bits() << 1 == self.level_flags().bits() {
            let _ = prev_handle.insert(InternHandle {
                link: 0,
                register_hi: hi,
                register_lo: lo,
                data: ENTROPY.get(),
            });
        }

        let mut current = *self;
        current.link = 0;

        (prev_handle, current)
    }

    /// Returns the resource type id,
    ///
    #[inline]
    pub fn resource_type_id(&self) -> Option<TypeId> {
        crate::repr::resource::TYPE_ID.copy(self)
    }

    /// Returns the resource type name,
    ///
    #[inline]
    pub fn resource_type_name(&self) -> Option<&'static str> {
        crate::repr::resource::TYPE_NAME.copy(self)
    }

    /// Returns the resource type size,
    ///
    #[inline]
    pub fn resource_type_size(&self) -> Option<usize> {
        crate::repr::resource::TYPE_SIZE.copy(self)
    }

    /// Returns the resource parse type name,
    ///
    #[inline]
    pub fn resource_parse_type_name(&self) -> Option<&'static str> {
        crate::repr::resource::PARSE_TYPE_NAME.copy(self)
    }

    /// Returns the resource ffi type name,
    ///
    #[inline]
    pub fn resource_ffi_type_name(&self) -> Option<&'static str> {
        crate::repr::resource::FFI_TYPE_NAME.copy(self)
    }

    /// Returns the resource ffi value parser,
    ///
    #[inline]
    #[cfg(feature = "util-clap")]
    pub fn resource_ffi_value_parser(
        &self,
    ) -> Option<clap::builder::Resettable<clap::builder::ValueParser>> {
        crate::repr::resource::FFI_VALUE_PARSER
            .clone(self)
            .and_then(|v| v)
    }

    /// Returns the parent of the dependency,
    ///
    #[inline]
    pub fn dependency_parent(&self) -> Option<Repr> {
        crate::repr::dependency::DEPENDENCY_PARENT.copy(self)
    }

    /// Returns the name of the dependency,
    ///
    #[inline]
    pub fn dependency_name(&self) -> Option<Arc<String>> {
        crate::repr::dependency::DEPENDENCY_NAME.strong_ref(self)
    }

    /// Returns the name of the receiver,
    ///
    #[inline]
    pub fn recv_name(&self) -> Option<Arc<String>> {
        crate::repr::recv::RECV_NAMES.strong_ref(self)
    }

    /// Returns the name of the receiver fields,
    ///
    #[inline]
    pub fn recv_fields(&self) -> Option<Arc<Vec<Repr>>> {
        crate::repr::recv::RECV_FIELDS.strong_ref(self)
    }

    /// Returns the type id of the owner of this field,
    ///
    #[inline]
    pub fn owner_type_id(&self) -> Option<TypeId> {
        crate::repr::field::OWNER_ID.copy(self)
    }

    /// Returns the type name of the owner of this field,
    ///
    #[inline]
    pub fn owner_name(&self) -> Option<&'static str> {
        crate::repr::field::OWNER_NAME.copy(self)
    }

    /// Returns the type size of the owner of this field,
    ///
    #[inline]
    pub fn owner_size(&self) -> Option<usize> {
        crate::repr::field::OWNER_SIZE.copy(self)
    }

    /// Returns the field offset,
    ///
    #[inline]
    pub fn field_offset(&self) -> Option<usize> {
        crate::repr::field::FIELD_OFFSET.copy(self)
    }

    /// Returns the field name,
    ///
    #[inline]
    pub fn field_name(&self) -> Option<&'static str> {
        crate::repr::field::FIELD_NAME.copy(self)
    }

    /// Returns the node symbol,
    ///
    #[inline]
    pub fn symbol(&self) -> Option<Arc<String>> {
        crate::repr::node::SYMBOL.strong_ref(self)
    }

    /// Returns a strong reference to the input,
    ///
    #[inline]
    pub fn input(&self) -> Option<Arc<String>> {
        crate::repr::node::INPUT.strong_ref(self)
    }

    /// Returns a strong reference to the tag,
    ///
    #[inline]
    pub fn tag(&self) -> Option<Arc<String>> {
        crate::repr::node::TAG.strong_ref(self)
    }

    /// Returns a strong reference to the path,
    ///
    #[inline]
    pub fn path(&self) -> Option<Arc<String>> {
        crate::repr::node::PATH.strong_ref(self)
    }

    /// Returns a strong reference to the node idx,
    ///
    #[inline]
    pub fn node_idx(&self) -> Option<usize> {
        crate::repr::node::NODE_IDX.copy(self)
    }

    /// Returns a strong reference to node source,
    ///
    #[inline]
    pub fn node_source(&self) -> Option<Arc<String>> {
        crate::repr::node::SOURCE.strong_ref(self)
    }

    /// Returns a strong reference to doc_headers,
    ///
    #[inline]
    pub fn doc_headers(&self) -> Option<Arc<Vec<String>>> {
        crate::repr::node::DOC_HEADERS.strong_ref(self)
    }

    /// Returns a strong reference to annotations,
    ///
    #[inline]
    pub fn annotations(&self) -> Option<Arc<BTreeMap<String, String>>> {
        crate::repr::node::ANNOTATIONS.strong_ref(self)
    }

    /// Returns the node source's parsed span,
    ///
    #[inline]
    pub fn source_span(&self) -> Option<Arc<SourceSpan>> {
        crate::repr::node::SOURCE_SPAN.strong_ref(self)
    }

    /// Returns the node source's relative path,
    ///
    #[inline]
    pub fn source_relative(&self) -> Option<Arc<PathBuf>> {
        crate::repr::node::SOURCE_RELATIVE.strong_ref(self)
    }

    /// Returns the host address,
    ///
    #[inline]
    pub fn host_address(&self) -> Option<Arc<String>> {
        crate::repr::host::ADDRESS.strong_ref(self)
    }

    /// Returns extensions added to this host,
    ///
    #[inline]
    pub fn host_extensions(&self) -> Option<Arc<Vec<Repr>>> {
        crate::repr::host::EXTENSIONS.strong_ref(self)
    }
}

/// Inner intern table map,
/// 
pub struct InternMap<T> {
    pub(crate) map: BTreeMap<InternHandle, Arc<T>>
}

impl<T> InternMap<T> {
    /// Returns an iterator for exporting this map,
    /// 
    pub fn iter_for_export(&self) -> impl Iterator<Item = (uuid::Uuid, bytes::Bytes)> + '_ 
    where
        T: Serialize
    {
        self.iter_entries().filter_map(|(k, i)| {
            i.upgrade().and_then(|i| {
                bincode::serialize(i.deref()).ok().map(|s| {
                    (k.as_uuid(), bytes::Bytes::copy_from_slice(s.as_ref()))
                })
            })
        })
    }

    /// Returns an iterator over inner entries,
    /// 
    /// **Note**: Does not create a strong reference to entry, instead creates a weak reference.
    /// 
    pub fn iter_entries(&self) -> impl Iterator<Item = (InternHandle, Weak<T>)> + '_ {
        self.map.iter().map(|(h, e)| {
            (*h, Arc::downgrade(e))
        })
    }

    /// Prune any entries that do not have strong references,
    /// 
    fn _prune(&mut self) {

    }
}

impl<T> Default for InternMap<T> {
    fn default() -> Self {
        Self { map: Default::default() }
    }
}

/// Type-alias for inner table container,
/// 
type InnerTable<T> = tokio::sync::watch::Sender<InternMap<T>>;

/// Struct maintaining an inner shared intern table,
///
pub struct InternTable<T: Send + Sync + 'static> {
    /// Inner table,
    ///
    inner: OnceLock<InnerTable<T>>,
}

impl<T: Send + Sync + 'static> InternTable<T> {
    /// Creates a new empty intern table,
    ///
    #[inline]
    pub const fn new() -> Self {
        Self {
            inner: OnceLock::new(),
        }
    }

    /// Assigns an intern handle for an immutable value,
    ///
    /// **Note** If the intern handle already has been assigned a value this will result in a no-op.
    ///
    pub fn assign_intern(&self, handle: InternHandle, value: T) -> anyhow::Result<()> {
        // Skip if the value has already been created
        {
            if self.inner().borrow().map.contains_key(&handle) {
                trace!("Skipping interning {:?}", handle);
                return Ok(());
            }
        }
        self.inner().send_modify(|t| {
            if t.map.insert(handle, Arc::new(value)).is_some() {
                warn!(
                    "Replacing intern handle {:?} {:x?}",
                    handle.level_flags(),
                    handle
                );
            }
        });

        Ok(())
    }

    /// Returns a handle to the interned value,
    ///
    /// **Errors** Returns an error if the value is not currently interned, or if the
    /// inner table lock is poisoned.
    ///
    pub fn get(&self, handle: &InternHandle) -> anyhow::Result<Weak<T>> {
        if let Some(value) = self.inner().borrow().map.get(handle) {
            Ok(Arc::downgrade(value))
        } else {
            Err(anyhow!("Not interned {:?}", handle))
        }
    }

    /// Returns a copy of the interned value from a handle,
    ///
    pub fn copy(&self, handle: &InternHandle) -> Option<T>
    where
        T: Copy,
    {
        self.get(handle)
            .ok()
            .as_ref()
            .and_then(Weak::upgrade)
            .as_deref()
            .copied()
    }

    /// Returns a clone of the interned value from a handle,
    ///
    pub fn clone(&self, handle: &InternHandle) -> Option<T>
    where
        T: Clone,
    {
        self.get(handle)
            .ok()
            .as_ref()
            .and_then(Weak::upgrade)
            .as_deref()
            .cloned()
    }

    /// Returns a new strong reference to the value,
    ///
    pub fn strong_ref(&self, handle: &InternHandle) -> Option<Arc<T>> {
        self.get(handle)
            .ok()
            .as_ref()
            .and_then(Weak::upgrade)
            .clone()
    }

    /// Returns a reference to the inner table,
    /// 
    fn inner(&self) -> &InnerTable<T> {
        self.inner.get_or_init(|| {
            let (tx, _) = tokio::sync::watch::channel(InternMap::<T>::default());

            tx
        })
    }

    /// Returns a file name to use for the table,
    /// 
    fn table_file_name(&self) {

    }
}

impl<T: Send + Sync + 'static> Default for InternTable<T> {
    fn default() -> Self {
        Self::new()
    }
}

bitflags::bitflags! {
    /// Representation level flags,
    ///
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub struct LevelFlags : u16 {
        /// Root representation level
        ///
        const ROOT = 0x0100;

        /// Representation Level 1
        ///
        const LEVEL_1 = 0x0100 << 1;

        /// Representation Level 2
        ///
        const LEVEL_2 = 0x0100 << 2;

        /// Representation Level 3
        ///
        const LEVEL_3 = 0x0100 << 3;

        /// Representation level 4
        ///
        const LEVEL_4 = 0x0100 << 4;

        /// Representation level 5
        ///
        const LEVEL_5 = 0x0100 << 5;

        /// Representation level 6
        ///
        const LEVEL_6 = 0x0100 << 6;

        /// Representation level 7
        ///
        const LEVEL_7 = 0x0100 << 7;
    }
}
