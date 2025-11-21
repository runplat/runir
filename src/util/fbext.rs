use crate::{util::annotate::Annotate, wire::ContentAddress};

use std::{marker::PhantomData, ops::Deref};

use anyhow::anyhow;
use flexbuffers::{Blob, Buffer};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use time::UtcDateTime;
use zstd::zstd_safe::WriteBuf;

/// Trait for embedding an object into a receiver
pub trait Object {
    /// Packages a serializable object into this type
    fn object<T: Serialize + Annotate>(self, obj: &T) -> Self;
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

impl<'a> Object for flexbuffers::MapBuilder<'a> {
    #[inline]
    fn object<T: Serialize + Annotate>(mut self, obj: &T) -> Self {
        let mut ser = flexbuffers::FlexbufferSerializer::new();
        self.push("version", "runir");
        self.push("ts", UtcDateTime::now().unix_timestamp());
        self.push("type_name", T::type_name());
        match obj.serialize(&mut ser) {
            Ok(()) => {
                let bytes = ser.view();
                self.push("size", bytes.len() as u64);
                if let Some("sha256") = T::value("cas") {
                    let digest = sha2::Sha256::digest(bytes);
                    self.push("sha256", Blob(digest.as_slice()));
                }
                self.push("object", Blob(bytes));
            }
            Err(err) => {
                self.push("err", err.to_string().as_str());
            }
        }
        self
    }
}

impl Object for flexbuffers::Builder {
    #[inline]
    fn object<T: Serialize + Annotate>(mut self, obj: &T) -> Self {
        self.start_map().object(obj).end_map();
        self
    }
}

impl<'a> PushObject<'a> for flexbuffers::MapBuilder<'a> {
    /// Enables pushing an object into a builder as an annotated blob
    #[inline]
    fn push_object<T: Serialize + Annotate>(&mut self, name: &str, obj: &T) -> &mut Self {
        self.start_map(name).object(obj).end_map();
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
            Err(anyhow!("Does not have version set").into())
        }
    }
}

impl<B> AsObject<B> for flexbuffers::Reader<B>
where
    B: Buffer,
    B::BufferString: AsRef<str>,
{
    fn as_object<T: Annotate>(&self) -> Option<ObjectReader<B, T>> {
        self.as_map()
            .idx("version")
            .get_str()
            .ok()
            .filter(|v| v.as_ref() == "runir")
            .and(
                self.as_map()
                    .idx("type_name")
                    .get_str()
                    .ok()
                    .filter(|t| t.as_ref() == T::type_name()),
            )
            .map(|_| ObjectReader {
                reader: self.clone(),
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

    /// Returns Some(valid) if sha256 CAS is enabled
    /// 
    /// Otherwise; returns None
    #[inline]
    pub fn check_sha256(&self, matches: ContentAddress) -> Option<bool> {
        self.idx("sha256").get_blob().ok().map(|b| {
            b.0.as_slice() == matches.digest
        })
    }

    fn idx(&self, key: &str) -> flexbuffers::Reader<B> {
        self.as_map().idx(key)
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

        let object = flexbuffers::Builder::default().object(&test);

        let reader = flexbuffers::Reader::get_root(builder.view()).unwrap();
        let obj = reader.as_map().idx("test").get_object::<Test>().unwrap();
        assert_eq!(obj.as_map().idx("value").as_str(), "hello world");

        // Test the object extension function returns the same view of the object
        let object = flexbuffers::Reader::get_root(object.view()).unwrap();
        assert_eq!(obj.buffer(), object.as_map().idx("object").as_blob().0);

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
