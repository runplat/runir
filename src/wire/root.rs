use std::path::PathBuf;

use bytes::{BufMut, Bytes};
use tracing::{error, trace};
use uuid::Uuid;

use crate::{
    Namespace, RecordInfo,
    vol::{
        MemoryMappedTarget, ReadMemoryMappedTarget, Volume, VolumeTarget, new_mmap_anon_target,
        new_mmap_target, open_mmap_target, read_mmap_target,
    },
    wire::{Boot, FrameList, Wire, boot::NS_HEADER_INLINE_BLOCK_SIZE},
};

#[derive(Debug, Clone)]
pub struct Root(PathBuf);

impl Root {
    /// Opens root in the current directory
    #[inline]
    pub fn current_dir() -> std::io::Result<Self> {
        let current = std::env::current_dir()?;
        Self::open(current)
    }

    /// Opens or initializes `{dir}/.runir`
    ///
    /// Returns an error if `{dir}/.runir` was not a directory or if `{dir}/.runir` could not be created
    #[inline]
    pub fn open(dir: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = dir.into().join(".runir");
        if !path.exists() {
            std::fs::create_dir_all(&path)?;
        } else if !path.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                format!("Expected {path:?} to be a directory, found a file instead"),
            ));
        }

        Ok(Self(path))
    }

    /// Returns an empty anon memory-map target configured for a record info
    ///
    /// w/ capacity described by the transport
    ///
    /// Note: Target must call .commit() in order to persist the target to the root
    #[inline]
    pub fn write_boot_target(
        &self,
        info: &RecordInfo,
        list: Option<FrameList>,
    ) -> std::io::Result<MemoryMappedTarget> {
        let vol_path = self.open_ns_vol_path(info, true)?;

        if let Some(list) = list {
            // TODO: Need to add a guard against required_capacity
            new_mmap_anon_target(
                vol_path,
                NS_HEADER_INLINE_BLOCK_SIZE + list.required_capacity() as usize,
            )
        } else {
            new_mmap_anon_target(vol_path, NS_HEADER_INLINE_BLOCK_SIZE)
        }
    }

    /// Reads a boot target for a specific record info
    #[inline]
    pub fn read_boot_target(&self, info: &RecordInfo) -> std::io::Result<ReadMemoryMappedTarget> {
        let vol_path = self.open_ns_vol_path(info, false)?;
        read_mmap_target(vol_path)
    }

    /// Opens a root catalog
    #[inline]
    pub fn catalog(&self, name: &str) -> std::io::Result<Catalog<MemoryMappedTarget>> {
        /*
            Concurreny Support?
        */
        Catalog::open(self, name)
    }

    /// Returns a new wire w/ the root set to this root
    #[inline]
    pub fn wire(&self, ns: impl Into<Namespace>) -> crate::wire::Wire {
        let mut wire = Wire::new(ns);
        wire.with_root(self.clone());
        wire
    }

    #[inline]
    fn open_ns_vol_path(
        &self,
        info: &RecordInfo,
        create_if_not_exists: bool,
    ) -> std::io::Result<PathBuf> {
        let (uuid, ns_chk, _, _) = info.to_parts();
        Ok(self
            .open_ns_dir(ns_chk, create_if_not_exists)?
            .join(uuid.to_string()))
    }

    #[inline]
    fn open_ns_dir(&self, ns_chk: u64, create_if_not_exists: bool) -> std::io::Result<PathBuf> {
        let uuid = Uuid::from_u64_pair(ns_chk, 0);
        let enc = hex::encode(uuid.as_bytes());
        let path = self.0.join("ns").join(&enc[..2]).join(&enc[2..4]).join(enc);
        if !path.exists() {
            if create_if_not_exists {
                std::fs::create_dir_all(&path)?;
                Ok(path)
            } else {
                error!("Could not open ns_dir at {path:?}");
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Directory does not exist",
                ))
            }
        } else {
            Ok(path)
        }
    }
}

/// Provides a catalog of stored boot records
pub struct Catalog<T> {
    volume: Volume<T, ()>,
}

#[inline]
fn open_catalog_mem_target(catalog: &PathBuf) -> std::io::Result<MemoryMappedTarget> {
    // TODO: If it exists, need to copy it into the me
    // if catalog.exists() {
    //     open_mmap_target(catalog, true)
    // } else {
    //     const CAP: u64 = 1024 * 1024 * 8; // 8000 records
    //     new_mmap_target(catalog, CAP)
    // }
    const CAP: u64 = 1024 * 1024 * 8; // 8000 records
    let mut active = new_mmap_target(catalog, CAP)?;
    if catalog.exists() {
        let opened = open_mmap_target(catalog, false)?;
        active.as_mut()[..opened.len()].copy_from_slice(&opened);
    }
    Ok(active)
}

impl Catalog<MemoryMappedTarget> {
    /// Opens a persistent catalog
    ///
    /// Scans for any existing items and advances the internal cursor
    #[inline]
    fn open(root: &Root, name: &str) -> std::io::Result<Self> {
        let catalog = root.0.join(name);
        let map = open_catalog_mem_target(&catalog)?;
        Self::new(map)
    }
}

impl<T: VolumeTarget + BufMut> Catalog<T> {
    /// Returns a new catalog
    #[inline]
    pub fn new(target: T) -> std::io::Result<Self> {
        let mut catalog = Self {
            volume: Volume::from_parts((target, ())),
        };
        catalog.scan()?;
        Ok(catalog)
    }

    /// List all boot headers stored in the volume
    ///
    /// Skips any boot records that could not be decoded
    #[inline]
    pub fn list(&self) -> Vec<Boot> {
        let mut list = vec![];
        for (idx, header) in self.volume.target().filled().chunks_exact(1024).enumerate() {
            match Boot::decode(Bytes::copy_from_slice(header)) {
                Ok(boot) => {
                    list.push(boot);
                }
                Err(err) => {
                    error!(idx, "boot_error: {err:#?}");
                }
            }
        }
        list
    }

    /// Add a boot to the catalog
    #[inline]
    pub fn add(&mut self, boot: Boot) {
        self.volume.target_mut().put(boot.header());
    }

    /// Returns the inner target
    #[inline]
    pub fn into_inner(self) -> T {
        self.volume.into_parts().0
    }

    /// Prunes any records in the catalog that are not committed to the root
    #[inline]
    pub fn prune(&mut self) {}

    /// Advances the cursor to the open position in the target
    #[inline]
    fn scan(&mut self) -> std::io::Result<()> {
        let last = self
            .volume
            .target()
            .view()
            .chunks_exact(1024)
            .enumerate()
            .filter(|(_, r)| Boot::decode(Bytes::copy_from_slice(*r)).is_ok())
            .last();

        if let Some((idx, _)) = last {
            let advance_to = (idx + 1) * 1024;
            trace!(advance_to = advance_to, "scan");
            self.volume.target_mut().advance(advance_to)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        IRecord,
        test::test_cas_record,
        util::PeekExtensions,
        vol::{VolumeTarget, new_memory_target, new_mmap_anon_target},
        wire::{Root, root::Catalog, tool::Tool},
    };
    use sha2::Sha256;

    #[test]
    fn mmap_sanity_check_len() {
        let target = new_mmap_anon_target("", 4096).unwrap();
        assert_eq!(target.len(), 4096);
    }

    #[test]
    fn test_catalog() {
        crate::test_boot!(let boot = "test_catalog");
        let target = new_memory_target("", 4096);
        let mut test = Catalog::new(target).unwrap();
        test.add(boot);

        let target = test.into_inner();
        let recover = Catalog::new(target).unwrap();
        recover.list().first().unwrap();
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn test_root_wire_push_pull() {
        color_eyre::install().unwrap();

        let root1 = Root::current_dir().unwrap();
        let root2 = Root::open(".runir2").unwrap();

        let mut wire1 = root1.wire("test");
        wire1.install_tool("test", "hello world".to_string());
        let wire2 = root2.wire("test");

        // Test pushing to a wire
        let mut pushed = wire1.push(test_cas_record(), Some(vec!["test"])).unwrap();

        // Test fetching w/ different wire
        let container = wire2.fetch(pushed.snapshot().unwrap()).await.unwrap();
        assert_eq!(
            container.tool(".settings").at("test").str().unwrap(),
            "hello world"
        );

        // Test adding to a catalog
        let mut catalog = root2.catalog("test").unwrap();
        catalog.add(container.boot);
        catalog.into_inner().commit().unwrap();

        // Test pulling from a different wire
        pushed.reset_cursor();
        let wire = wire2.pull(pushed).await.unwrap();

        // Test committing wire data to root
        // assert!(wire.commit().is_ok());

        let rec = wire.read_record_checked().unwrap();
        assert!(rec.is_valid());
        assert_eq!(rec.peek().at("value").str().unwrap(), "hello world");
        assert!(rec.matches_content::<Sha256>(wire2.proto.ns.clone()));

        // let catalog = root2.catalog("test").unwrap();
        // for b in catalog.list() {
        //     let reading = root2.read_boot_target(&b.info().unwrap()).unwrap();
        //     let reading = wire2.pull(reading).await.unwrap();
        //     let rec = reading.receive().unwrap();
        //     debug!("{:?}", rec.info);
        // }
    }
}
