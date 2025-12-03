use crate::wire::Annotate;

use std::{marker::PhantomData, ops::Deref};

use anyhow::anyhow;
use flexbuffers::{Blob, Buffer, MapBuilder};
use serde::{Deserialize, Serialize};
use time::UtcDateTime;

/// Type-alias for a format object function
pub type FormatObject<T> = for<'a> fn(&T, &mut MapBuilder<'a>);

/// Type-alias for a read object function
pub type ReadObject<B> = fn(flexbuffers::Reader<B>) -> Option<flexbuffers::Reader<B>>;

/// Trait for embedding an object into a receiver
pub trait ApplyObject {
    /// Applies a serializable object into this container
    fn apply_object<T: Serialize + Annotate>(self, obj: &T) -> Self;
}

/// Trait for pushing an object into a receiver
pub trait PushObject<'a> {
    /// Enables pushing an object into a builder as an annotated blob
    fn push_object<T: Serialize + Annotate>(&mut self, name: &str, obj: &T) -> &mut Self;
}

/// Trait for recovering an object from a receiver
pub trait GetObject<B> {
    /// Enables pushing an object into a builder as an annotated blob
    fn get_object<'a, T: Annotate + 'a>(&self) -> crate::Result<flexbuffers::Reader<B>>;
}

pub trait AsObject<B> {
    /// Returns the current reader as a flexbuffer::Reader over an Object root
    ///
    /// Returns None if the type_name and/or version have incorrect values
    fn as_object<T: Annotate>(&self) -> Option<ObjectReader<B, T>>;
}

impl<'a> ApplyObject for flexbuffers::MapBuilder<'a> {
    #[inline]
    fn apply_object<T: Serialize + Annotate>(mut self, obj: &T) -> Self {
        T::object_format()(obj, &mut self);
        self
    }
}

impl ApplyObject for flexbuffers::Builder {
    #[inline]
    fn apply_object<T: Serialize + Annotate>(mut self, obj: &T) -> Self {
        self.start_map().apply_object(obj).end_map();
        self
    }
}

impl<'a> PushObject<'a> for flexbuffers::MapBuilder<'a> {
    /// Enables pushing an object into a builder as an annotated blob
    #[inline]
    fn push_object<T: Serialize + Annotate>(&mut self, name: &str, obj: &T) -> &mut Self {
        self.start_map(name).apply_object(obj).end_map();
        self
    }
}

impl<B> GetObject<B> for flexbuffers::Reader<B>
where
    B: Buffer,
    B::BufferString: AsRef<str>,
{
    fn get_object<'a, T: Annotate + 'a>(&self) -> crate::Result<flexbuffers::Reader<B>> {
        if let Some(object) = self.as_object::<T>() {
            let span = tracing::trace_span!(
                "get_object",
                type_name = object.type_name().as_ref(),
                size = object.size(),
                created = object.created().unix_timestamp(),
            );
            span.in_scope(|| object.read())
        } else {
            Err(anyhow!("Current reader is not an object").into())
        }
    }
}

impl<B> AsObject<B> for flexbuffers::Reader<B>
where
    B: Buffer,
    B::BufferString: AsRef<str>,
{
    fn as_object<T: Annotate>(&self) -> Option<ObjectReader<B, T>> {
        T::object_reader()(self.clone())
            .map(|reader| ObjectReader {
                reader,
                _t: Default::default(),
            })
    }
}

/// Wrapper returned after validating the stored type
pub struct ObjectReader<B, T> {
    reader: flexbuffers::Reader<B>,
    _t: PhantomData<T>,
}

impl<B, T> ObjectReader<B, T>
where
    B: flexbuffers::Buffer,
    B::BufferString: AsRef<str>,
{
    /// Returns the value of the version field
    #[inline]
    pub fn version(&self) -> impl AsRef<str> {
        self.idx("version").as_str()
    }

    /// Returns the type name of the stored object
    #[inline]
    pub fn type_name(&self) -> impl AsRef<str> {
        self.idx("type_name").as_str()
    }

    /// Returns the size of the object's flexbuffer root
    #[inline]
    pub fn size(&self) -> u64 {
        self.idx("ts").as_u64()
    }

    /// Returns the timestamp this object was created
    #[inline]
    pub fn created(&self) -> UtcDateTime {
        UtcDateTime::from_unix_timestamp(self.idx("ts").as_i64()).unwrap_or(UtcDateTime::UNIX_EPOCH)
    }

    /// Reads the flexbuffer root of this object
    ///
    /// Returns an error if the object map is invalid
    #[inline]
    pub fn read(&self) -> crate::Result<flexbuffers::Reader<B>> {
        if let Some(Blob(object)) = self.idx("object").get_blob().ok() {
            Ok(flexbuffers::Reader::get_root(object)?)
        } else if let Some(error) = self.idx("error").get_str().ok() {
            Err(anyhow!(error.to_string()).into())
        } else {
            Err(anyhow!("Invalid object map").into())
        }
    }

    fn idx(&self, key: &str) -> flexbuffers::Reader<B> {
        self.reader.as_map().idx(key)
    }
}

impl<'a, T: Deserialize<'a>> ObjectReader<&'a [u8], T> {
    /// Deserializes the stored object that borrows from the underlying buffer
    ///
    /// Returns an error if the object mapped is invalid or if deserialization failed
    #[inline]
    pub fn to_object(self) -> crate::Result<T> {
        Ok(flexbuffers::from_slice(self.read()?.buffer())?)
    }
}

impl<B, T> Deref for ObjectReader<B, T> {
    type Target = flexbuffers::Reader<B>;

    fn deref(&self) -> &Self::Target {
        &self.reader
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    struct Test<'a> {
        value: &'a str,
    }

    impl<'a> Annotate for Test<'a> {
        fn type_name() -> &'static str {
            "fbext::tests"
        }
    }

    #[test]
    fn test_flexbuffer_exts() {
        let test = Test {
            value: "hello world",
        };
        let mut builder = flexbuffers::Builder::default();
        let mut map = builder.start_map();
        map.push_object("test", &test);
        map.end_map();

        let object = flexbuffers::Builder::default().apply_object(&test);

        let reader = flexbuffers::Reader::get_root(builder.view()).unwrap();
        let obj = reader.as_map().idx("test").get_object::<Test>().unwrap();
        assert_eq!(obj.as_map().idx("value").as_str(), "hello world");

        // Test the object extension function returns the same view of the object
        let object = flexbuffers::Reader::get_root(object.view()).unwrap();
        assert_eq!(
            obj.buffer(),
            object
                .as_map()
                .idx(".runir")
                .as_map()
                .idx("object")
                .as_blob()
                .0
        );

        // Test the flow beween object() -> get_object()
        assert_eq!(
            object
                .get_object::<Test>()
                .unwrap()
                .as_map()
                .idx("value")
                .as_str(),
            "hello world"
        );

        let test = object.as_object::<Test>().unwrap().to_object().unwrap();
        assert_eq!(test.value, "hello world");
    }
}
