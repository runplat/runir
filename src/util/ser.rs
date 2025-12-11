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
                Some(peek) => Self::Value::deserialize(peek).map_err(|e| E::custom(e.to_string())),
                None => Err(E::custom("Could not deserialize as an object")),
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
                test.peek().at("nested").peek().at("value").str(),
                Some("hello world")
            );

            // Test deserializing nested as a blob back into it's object
            let test: Test = Test::deserialize(test.peek().val().unwrap()).unwrap();
            assert_eq!(test.nested.value, "hello world");
        }
    }
}
