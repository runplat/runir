use crate::entropy::ENTROPY;
use crate::interner::InternResult;
use crate::interner::LevelFlags;
use crate::prelude::*;
use crc::Crc;
use std::cell::RefCell;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::OnceLock;
use tracing::trace;

/// Interner that uses crc to build intern handles,
///
pub struct CrcInterner {
    /// Digest builder,
    ///
    digest: RefCell<crc::Digest<'static, u32>>,
    /// Sets the current level flag,
    ///
    flags: LevelFlags,
    /// Stack of tags being interned,
    ///
    tags: Vec<InternHandleThunk>,
    /// Sets the current data,
    ///
    /// **Note**: When applied to the intern handle it will be DATA ^ ENTROPY
    ///
    data: u64,
}

impl Default for CrcInterner {
    fn default() -> Self {
        Self::new()
    }
}

/// CRC for calculating the crc values of an intern handle,
///
static INTERNER_CRC: OnceLock<crc::Crc<u32>> = OnceLock::new();

impl CrcInterner {
    fn new() -> Self {
        let crc = INTERNER_CRC.get_or_init(|| Crc::<u32>::new(&crc::CRC_24_OPENPGP));

        let digest = RefCell::new(crc.digest());

        CrcInterner {
            digest,
            tags: vec![],
            flags: LevelFlags::ROOT,
            data: 0,
        }
    }
}

impl InternerFactory for CrcInterner {
    #[inline]
    fn push_tag<T>(
        &mut self,
        value: T,
        tag: impl Fn(InternHandle) -> anyhow::Result<()> + Send + Sync + 'static,
    ) where
        T: Hash + Send + Sync + 'static,
    {
        value.hash(self);

        self.tags.push(Box::new(tag));
    }

    #[inline]
    fn set_level_flags(&mut self, flags: crate::interner::LevelFlags) {
        self.flags = flags;
    }

    #[inline]
    fn set_data(&mut self, data: u64) {
        self.data = data;
    }

    fn interner(&mut self) -> InternResult {
        // Create an initial field hash
        let field_hash = self.finish();

        // Convert the field hash into an intern handle
        let converter = uuid::Uuid::from_u64_pair(field_hash, 0);

        let (link, register_hi, register_lo, _) = converter.as_fields();

        // Register a new intern handle
        let handle = InternHandle {
            link,
            register_hi: self.flags.bits() | register_hi,
            register_lo,
            data: ENTROPY.get() ^ self.data,
        };

        // Peek at converter state
        trace!("Creating {:04x?}", handle);
        let tags = self.tags.drain(..).collect::<Vec<_>>();

        for tag in tags {
            (tag)(handle)?
        }

        Ok(handle)
    }
}

impl Hasher for CrcInterner {
    fn finish(&self) -> u64 {
        let crc = INTERNER_CRC.get_or_init(|| Crc::<u32>::new(&crc::CRC_24_OPENPGP));

        let hash = self.digest.replace(crc.digest()).finalize();

        let [lo, hi] = bytemuck::cast::<u32, [u16; 2]>(hash);

        let uuid = uuid::Uuid::from_fields(0, hi, lo, &[0; 8]);

        let (key, _) = uuid.as_u64_pair();

        key
    }

    fn write(&mut self, bytes: &[u8]) {
        self.digest.borrow_mut().update(bytes);
    }
}

#[allow(unused)]
mod tests {
    use std::{collections::BTreeMap, time::Duration};

    use crate::{entropy::ENTROPY, interner::LevelFlags, prelude::*};

    struct Test;

    impl Field<0> for Test {
        type ParseType = String;
        type ProjectedType = String;
        type FFIType = String;

        fn field_name() -> &'static str {
            "test"
        }
    }

    #[test]
    fn test_interner() {
        let mut interner = CrcInterner::new();
        /*
           NOTE: These are "canary" tests so may be unstable initially. The idea is to assert
           if the inner type representation from the compiler has changed unexpectedly. In theory, this
           wouldn't matter too much since an intern handle only needs to be valid during runtime.
        */

        // Test creating a type level
        let rhandle = ResourceLevel::new::<String>()
            .configure(&mut interner)
            .unwrap();

        assert_eq!(LevelFlags::ROOT, rhandle.level_flags());

        // Test field level
        let handle = FieldLevel::new::<0, Test>()
            .configure(&mut interner)
            .unwrap();

        assert_eq!(LevelFlags::LEVEL_1, handle.level_flags());

        // Test input level
        let handle_1 = NodeLevel::new_with(
            Some("test"),
            Some("hello world"),
            Some(""),
            Some(""),
            Some(0),
            None,
            Some(""),
            Some(vec![""]),
            None,
        )
        .configure(&mut interner)
        .unwrap();

        // Test no unexpected side effects exist
        let handle_2 = NodeLevel::new_with(
            Some("test"),
            Some("hello world"),
            Some(""),
            Some(""),
            Some(0),
            Some(0),
            Some(""),
            Some(vec![""]),
            None,
        )
        .configure(&mut interner)
        .unwrap();

        assert_eq!(LevelFlags::LEVEL_2, handle_1.level_flags());
        assert_eq!(LevelFlags::LEVEL_2, handle_2.level_flags());
        assert_eq!(handle_1, handle_2);

        // Test host level
        let handle = HostLevel::new("test://").configure(&mut interner).unwrap();

        assert_eq!(LevelFlags::LEVEL_3, handle.level_flags());

        let a = rhandle.resource_type_name();
        let b = rhandle.resource_type_name();
        assert_eq!(a, b);

        let address = handle.host_address();

        ()
    }

    #[test]
    fn test_linker() {
        let mut repr = Linker::<CrcInterner>::describe_resource::<String>();

        // Assert the level is at the root
        assert_eq!(0, repr.level());

        repr.push_level(FieldLevel::new::<0, Test>()).unwrap();
        repr.push_level(FieldLevel::new::<0, Test>())
            .expect_err("should be an error");
        repr.push_level(NodeLevel::new_with(
            Some("test"),
            Some("hello world"),
            Some(""),
            Some(""),
            Some(0),
            Some(0),
            Some(""),
            Some(vec!["hello"]),
            Some(BTreeMap::new()),
        ))
        .unwrap();
        repr.push_level(HostLevel::new("engine://")).unwrap();

        assert_eq!(3, repr.level());

        // TODO: convert eprintln to assert_eq
        let repr = repr.link().unwrap();
        eprintln!("{:x?}", repr);

        let levels = repr.get_levels();
        eprintln!("{:#x?}", levels);
        eprintln!("{:x?}", repr.as_u64());

        let mut drepr = Linker::<CrcInterner>::describe_resource::<String>();
        drepr
            .push_level(DependencyLevel::new("cool dep").with_parent(repr))
            .unwrap();

        let mut _drepr = drepr.link().unwrap();
        eprintln!("{:x?}", _drepr);

        let levels = _drepr.get_levels();
        eprintln!("{:#x?}", levels);
        eprintln!("{:x?}", _drepr.as_u64());

        let drepr = _drepr.as_dependency().unwrap();

        // ketchup().await;

        let parent = drepr.parent();
        eprintln!("{:x?}", parent);

        let name = drepr.name();
        eprintln!("{:?}", name);

        let parent_type_name = parent.unwrap().as_resource().unwrap().type_name().unwrap();
        eprintln!("{}", parent_type_name);

        let before_upgrade = _drepr.clone();
        let upgrade = NodeLevel::new().with_input("hello world");
        _drepr.upgrade(CrcInterner::default(), upgrade).unwrap();

        let input = _drepr.as_node().unwrap().input().unwrap();
        eprintln!("{:?}", input);
        ()
    }
}
