use std::{
    collections::BTreeMap,
    ops::{Deref, DerefMut},
};

/// Serializes a byte slice as a byte slice
///
/// By default some implementations choose to serialize as a seq
#[inline]
pub fn bytes<S>(bytes: &[u8], ser: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    ser.serialize_bytes(bytes)
}

/// Field type for an ordered map of bytes fields
#[derive(
    Ord,
    PartialOrd,
    Debug,
    Default,
    PartialEq,
    Eq,
    Clone,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
#[repr(transparent)]
#[serde(transparent)]
pub struct BytesMap<'peek>(#[serde(borrow)] pub BTreeMap<&'peek str, BytesField<'peek>>);

impl<'peek> Deref for BytesMap<'peek> {
    type Target = BTreeMap<&'peek str, BytesField<'peek>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'peek> DerefMut for BytesMap<'peek> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Field type for a vector of bytes
#[derive(
    Ord,
    PartialOrd,
    Debug,
    Default,
    PartialEq,
    Eq,
    Clone,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
#[repr(transparent)]
#[serde(transparent)]
pub struct BytesVec<'peek>(#[serde(borrow)] pub Vec<BytesField<'peek>>);

impl<'peek> Deref for BytesVec<'peek> {
    type Target = Vec<BytesField<'peek>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'peek> DerefMut for BytesVec<'peek> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Field type for a bytes slice type that gurantees that serialization will call serialize_bytes
#[derive(Ord, PartialOrd, Debug, Eq, Clone, Hash, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum BytesField<'peek> {
    Borrowed(#[serde(serialize_with = "crate::util::ser::bytes")] &'peek [u8]),
    Owned(#[serde(serialize_with = "crate::util::ser::bytes")] Vec<u8>),
}

impl<'peek> PartialEq for BytesField<'peek> {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl<'peek> Deref for BytesField<'peek> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        match self {
            BytesField::Borrowed(items) => *items,
            BytesField::Owned(items) => items,
        }
    }
}

impl<'peek> AsRef<[u8]> for BytesField<'peek> {
    fn as_ref(&self) -> &[u8] {
        self.deref()
    }
}

pub mod obj {
    use crate::util::PeekExtensions;
    use serde::Deserialize;
    use std::marker::PhantomData;

    /// Serializes an object as a flexbuffer root and stores as a bytes slice
    #[inline]
    pub fn serialize<O, S>(object: &O, ser: S) -> Result<S::Ok, S::Error>
    where
        O: serde::Serialize,
        S: serde::Serializer,
    {
        let mut fb = flexbuffers::FlexbufferSerializer::new();
        object
            .serialize(&mut fb)
            .map_err(|e| <S::Error as serde::ser::Error>::custom(e.to_string()))?;
        ser.serialize_bytes(fb.view())
    }

    /// Deserializes an object from a bytes buffer
    #[inline]
    pub fn deserialize<'de, D, T: Deserialize<'de>>(deser: D) -> Result<T, D::Error>
    where
        T: 'de,
        D: serde::Deserializer<'de>,
    {
        deser.deserialize_bytes(ObjectVisitor { _u: PhantomData })
    }

    struct ObjectVisitor<'de, T> {
        _u: PhantomData<&'de T>,
    }

    impl<'de, T> serde::de::Visitor<'de> for ObjectVisitor<'de, T>
    where
        T: Deserialize<'de>,
    {
        type Value = T;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(formatter, "Expecting a borrowed bytes slice")
        }

        fn visit_borrowed_bytes<E>(self, v: &'de [u8]) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            match v.val() {
                Some(peek) if peek.flexbuffer_type().is_blob() => peek
                    .as_blob()
                    .0
                    .val()
                    .map(|p| p.try_to_obj().map_err(E::custom))
                    .unwrap_or_else(|| {
                        Err(E::custom(
                            "Could not deserialize blob value as a flexbuffer root",
                        ))
                    }),
                Some(peek) => peek.try_to_obj().map_err(E::custom),
                None => Err(E::custom("Could not deserialize as a flexbuffer root")),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use serde::{Deserialize, Serialize};

        use crate::{IRecord, Namespace, util::PeekExtensions};

        #[derive(Serialize, Deserialize)]
        struct Test<'peek> {
            #[serde(borrow, with = "crate::util::ser::obj")]
            nested: Nested<'peek>,
        }

        #[derive(Serialize, Deserialize)]
        struct Nested<'peek> {
            value: &'peek str,
        }

        #[test]
        fn test_serde_with() {
            let test = Namespace::ephemeral()
                .store_content(&Test {
                    nested: Nested {
                        value: "hello world",
                    },
                })
                .unwrap();

            // Test nested got stored as a flexbuffer root
            assert_eq!(
                test.peek_at("nested").at("value").str(),
                Some("hello world")
            );

            // Test deserializing nested as a blob back into it's object
            let test: Test = test.peek().val().unwrap().try_to_obj().unwrap();
            assert_eq!(test.nested.value, "hello world");
        }
    }
}
