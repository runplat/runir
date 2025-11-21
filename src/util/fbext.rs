use crate::util::annotate::Annotate;
use flexbuffers::{Blob, Buffer};
use serde::Serialize;
use sha2::Digest;
use time::UtcDateTime;

pub trait Object {
    /// Enables pushing an object into a builder as an annotated blob
    ///
    /// Returns the builder
    fn object<T: Serialize + Annotate>(self, obj: &T) -> Self;
}

pub trait PushObject<'a> {
    /// Enables pushing an object into a builder as an annotated blob
    fn push_object<T: Serialize + Annotate>(&mut self, name: &str, obj: &T) -> &mut Self;
}

pub trait GetObject<B> {
    /// Enables pushing an object into a builder as an annotated blob
    fn get_object<'a, T: Annotate + 'a>(&self) -> Option<flexbuffers::Reader<B>>;
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
    fn get_object<'a, T: Annotate + 'a>(&self) -> Option<flexbuffers::Reader<B>> {
        if let Some(Blob(object)) = self
            .as_map()
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
            .and(self.as_map().idx("object").get_blob().ok())
        {
            flexbuffers::Reader::get_root(object).ok()
        } else {
            None
        }
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
    fn test_object_exts() {
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
        )
    }
}
