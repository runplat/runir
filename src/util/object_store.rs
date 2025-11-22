// use std::collections::BTreeSet;
// use std::fmt::Display;
// use std::ops::Range;
// use async_trait::async_trait;
// use bytes::Bytes;
// use futures::stream::BoxStream;
// use object_store::multipart::{MultipartStore, PartId};
// use object_store::{
//     GetOptions, GetRange, GetResult, GetResultPayload, ListResult, MultipartId, MultipartUpload, ObjectMeta, ObjectStore, PutMode, PutMultipartOptions, PutOptions, PutPayload, PutResult
// };
// use object_store::Result;
// use object_store::Error;
// use object_store::path::Path as ObjectPath;

// use crate::Namespace;

// /// Implements `object-store` using runir systems
// /// 
// /// TODO:
// /// This is essentially the same implementation as what `runir kv` does.
// /// So port that logic here, and then the cli can use this as it's implementation instead.
// #[derive(Debug)]
// struct RunirStore {
//     todo: String,
// }

// #[async_trait]
// impl ObjectStore for RunirStore {
//     async fn put_opts(
//         &self,
//         location: &ObjectPath,
//         payload: PutPayload,
//         opts: PutOptions,
//     ) -> Result<PutResult> {
//         /*
//             TODO:
//             let record = Namespace::record(location);
//             let record = record.commit(Wire(payload));
//             let build = self.next.build();
//             build.push_object_with(record, opts.into());
//             let e_tag = record.index_key();
//         */

//         // let mut storage = self.storage.write();
//         // let etag = storage.next_etag;
//         // let entry = Entry::new(payload.into(), Utc::now(), etag, opts.attributes);
//         // match opts.mode {
//         //     PutMode::Overwrite => todo!(),
//         //     PutMode::Create => todo!(),
//         //     PutMode::Update(v) => todo!(),
//         // }
//         // // storage.next_etag += 1;
//         // Ok(PutResult {
//         //     e_tag: None,
//         //     version: None,
//         // })
//         todo!()
//     }

//     async fn put_multipart_opts(
//         &self,
//         location: &ObjectPath,
//         opts: PutMultipartOptions,
//     ) -> Result<Box<dyn MultipartUpload>> {
//         // Ok(Box::new(InMemoryUpload {
//         //     location: location.clone(),
//         //     attributes: opts.attributes,
//         //     parts: vec![],
//         //     storage: Arc::clone(&self.storage),
//         // }))
//         todo!()
//     }

//     async fn get_opts(&self, location: &ObjectPath, options: GetOptions) -> Result<GetResult> {
//         // let entry = self.entry(location)?;
//         // let e_tag = entry.e_tag.to_string();

//         // let meta = ObjectMeta {
//         //     location: todo!(),
//         //     last_modified: todo!(),
//         //     size: todo!(),
//         //     e_tag,
//         //     version: todo!(),
//         // };
//         // options.check_preconditions(&meta)?;

//         // let (range, data) = match options.range {
//         //     Some(range) => {
//         //         let r = range
//         //             .as_range(entry.data.len() as u64)
//         //             .map_err(|source| Error::Range { source })?;
//         //         (
//         //             r.clone(),
//         //             entry.data.slice(r.start as usize..r.end as usize),
//         //         )
//         //     }
//         //     None => (0..entry.data.len() as u64, entry.data),
//         // };
//         // let stream = futures::stream::once(futures::future::ready(Ok(data)));

//         // Ok(GetResult {
//         //     payload: GetResultPayload::Stream(stream.boxed()),
//         //     attributes: entry.attributes,
//         //     meta,
//         //     range,
//         // })
//         todo!()
//     }

//     async fn get_ranges(&self, location: &ObjectPath, ranges: &[Range<u64>]) -> Result<Vec<Bytes>> {
//         // let entry = self.entry(location)?;
//         // ranges
//         //     .iter()
//         //     .map(|range| {
//         //         let r = GetRange::Bounded(range.clone())
//         //             .as_range(entry.data.len() as u64)
//         //             .map_err(|source| Error::Range { source })?;
//         //         let r_end = usize::try_from(r.end).map_err(|_e| Error::Range {
//         //             source: Error::InvalidGetRange::TooLarge {
//         //                 requested: r.end,
//         //                 max: usize::MAX as u64,
//         //             },
//         //         })?;
//         //         let r_start = usize::try_from(r.start).map_err(|_e| Error::Range {
//         //             source: Error::InvalidGetRange::TooLarge {
//         //                 requested: r.start,
//         //                 max: usize::MAX as u64,
//         //             },
//         //         })?;
//         //         Ok(entry.data.slice(r_start..r_end))
//         //     })
//         //     .collect()
//         todo!()
//     }

//     async fn head(&self, location: &ObjectPath) -> Result<ObjectMeta> {
//         // let entry = self.entry(location)?;

//         // Ok(ObjectMeta {
//         //     location: location.clone(),
//         //     last_modified: entry.last_modified,
//         //     size: entry.data.len() as u64,
//         //     e_tag: Some(entry.e_tag.to_string()),
//         //     version: None,
//         // })
//         todo!()
//     }

//     async fn delete(&self, location: &ObjectPath) -> Result<()> {
//         // self.storage.write().map.remove(location);
//         // Ok(())
//         todo!()
//     }

//     fn list(&self, prefix: Option<&ObjectPath>) -> BoxStream<'static, Result<ObjectMeta>> {
//         // let root = ObjectPath::default();
//         // let prefix = prefix.unwrap_or(&root);
//         // let storage = self.storage.read();
//         // let values: Vec<_> = storage
//         //     .map
//         //     .range((prefix)..)
//         //     .take_while(|(key, _)| key.as_ref().starts_with(prefix.as_ref()))
//         //     .filter(|(key, _)| {
//         //         // Don't return for exact prefix match
//         //         key.prefix_match(prefix)
//         //             .map(|mut x| x.next().is_some())
//         //             .unwrap_or(false)
//         //     })
//         //     .map(|(key, value)| {
//         //         Ok(ObjectMeta {
//         //             location: key.clone(),
//         //             last_modified: value.last_modified,
//         //             size: value.data.len() as u64,
//         //             e_tag: Some(value.e_tag.to_string()),
//         //             version: None,
//         //         })
//         //     })
//         //     .collect();
//         // futures::stream::iter(values).boxed()
//         todo!()
//     }

//     /// The memory implementation returns all results, as opposed to the cloud
//     /// versions which limit their results to 1k or more because of API
//     /// limitations.
//     async fn list_with_delimiter(&self, prefix: Option<&ObjectPath>) -> Result<ListResult> {
//         // let root = ObjectPath::default();
//         // let prefix = prefix.unwrap_or(&root);

//         // let mut common_prefixes = BTreeSet::new();

//         // // Only objects in this base level should be returned in the
//         // // response. Otherwise, we just collect the common prefixes.
//         // let mut objects = vec![];
//         // for (k, v) in self.storage.read().map.range((prefix)..) {
//         //     if !k.as_ref().starts_with(prefix.as_ref()) {
//         //         break;
//         //     }

//         //     let mut parts = match k.prefix_match(prefix) {
//         //         Some(parts) => parts,
//         //         None => continue,
//         //     };

//         //     // Pop first element
//         //     let common_prefix = match parts.next() {
//         //         Some(p) => p,
//         //         // Should only return children of the prefix
//         //         None => continue,
//         //     };

//         //     if parts.next().is_some() {
//         //         common_prefixes.insert(prefix.child(common_prefix));
//         //     } else {
//         //         let object = ObjectMeta {
//         //             location: k.clone(),
//         //             last_modified: v.last_modified,
//         //             size: v.data.len() as u64,
//         //             e_tag: Some(v.e_tag.to_string()),
//         //             version: None,
//         //         };
//         //         objects.push(object);
//         //     }
//         // }

//         // Ok(ListResult {
//         //     objects,
//         //     common_prefixes: common_prefixes.into_iter().collect(),
//         // })
//         todo!()
//     }

//     async fn copy(&self, from: &ObjectPath, to: &ObjectPath) -> Result<()> {
//         // let entry = self.entry(from)?;
//         // self.storage
//         //     .write()
//         //     .insert(to, entry.data, entry.attributes);
//         // Ok(())
//         todo!()
//     }

//     async fn copy_if_not_exists(&self, from: &ObjectPath, to: &ObjectPath) -> Result<()> {
//         // let entry = self.entry(from)?;
//         // let mut storage = self.storage.write();
//         // if storage.map.contains_key(to) {
//         //     return Err(Error::AlreadyExists {path:to.to_string(), source: todo!() }
//         //     .into());
//         // }
//         // storage.insert(to, entry.data, entry.attributes);
//         // Ok(())
//         todo!()
//     }
// }

// #[async_trait]
// impl MultipartStore for RunirStore {
//     async fn create_multipart(&self, _path: &ObjectPath) -> Result<MultipartId> {
//         // let mut storage = self.storage.write();
//         // let etag = storage.next_etag;
//         // storage.next_etag += 1;
//         // storage.uploads.insert(etag, Default::default());
//         // Ok(etag.to_string())
//         todo!()
//     }

//     async fn put_part(
//         &self,
//         _path: &ObjectPath,
//         id: &MultipartId,
//         part_idx: usize,
//         payload: PutPayload,
//     ) -> Result<PartId> {
//         // let mut storage = self.storage.write();
//         // let upload = storage.upload_mut(id)?;
//         // if part_idx <= upload.parts.len() {
//         //     upload.parts.resize(part_idx + 1, None);
//         // }
//         // upload.parts[part_idx] = Some(payload.into());
//         // Ok(PartId {
//         //     content_id: Default::default(),
//         // })
//         todo!()
//     }

//     async fn complete_multipart(
//         &self,
//         path: &ObjectPath,
//         id: &MultipartId,
//         _parts: Vec<PartId>,
//     ) -> Result<PutResult> {
//         // let mut storage = self.storage.write();
//         // let upload = storage.remove_upload(id)?;

//         // let mut cap = 0;
//         // for (part, x) in upload.parts.iter().enumerate() {
//         //     cap += x.as_ref().ok_or(Error::MissingPart { part })?.len();
//         // }
//         // let mut buf = Vec::with_capacity(cap);
//         // for x in &upload.parts {
//         //     buf.extend_from_slice(x.as_ref().unwrap())
//         // }
//         // let etag = storage.insert(path, buf.into(), Default::default());
//         // Ok(PutResult {
//         //     e_tag: Some(etag.to_string()),
//         //     version: None,
//         // })
//         todo!()
//     }

//     async fn abort_multipart(&self, _path: &ObjectPath, id: &MultipartId) -> Result<()> {
//         // self.storage.write().remove_upload(id)?;
//         // Ok(())
//         todo!()
//     }
// }

// impl Display for RunirStore {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         Ok(())
//     }
// }
