use crate::{Record, wire::FrameList};

/// Type-alias for an indexer function
pub type Indexer = fn(&Record) -> std::io::Result<Vec<u8>>;

/// Type-alias for a framing function
pub type Framing = fn(&Record) -> std::io::Result<FrameList>;

/// Various types of "tool" functions that can be applied during the protocol encoding
#[derive(Debug, Clone)]
pub enum Tool {
    /// Tool creates an index blob
    Indexer(Indexer),
    /// Tool that creates a frame list for a record transport
    Framing(Framing),
    /// Tool adds a setting value
    Setting(String),
    /// Tool adds a config value
    Config(Vec<u8>),
}

impl From<Indexer> for Tool {
    fn from(value: Indexer) -> Self {
        Tool::Indexer(value)
    }
}

impl From<Framing> for Tool {
    fn from(value: Framing) -> Self {
        Tool::Framing(value)
    }
}

impl From<String> for Tool {
    fn from(value: String) -> Self {
        Tool::Setting(value)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Namespace,
        util::PeekExtensions,
        vol::VolumeTarget,
        wire::{Boot, Wire, tool::Tool},
    };

    #[test]
    #[tracing_test::traced_test]
    fn test_builtin_from_toml() {
        let mut wire = Wire::new(Namespace::new("test"));

        wire.enable_builtin_tools();
        wire.install_tool("label", Tool::Setting("hello label".to_string()));
        wire.install_tool("label2", Tool::Setting("another hello label".to_string()));

        let rec = Namespace::ephemeral().content(
            r#"
            [test]
            value = "hello world"
        "#);

        let target = wire
            .push(rec, Some(vec![".from_toml", "label", "label2"]))
            .unwrap();

        let boot = Boot::decode(target.snapshot().unwrap()).unwrap();

        assert!(boot.is_boot_inline());
        assert_eq!(boot.layout().frames.len(), 4);
        eprintln!(
            "{}",
            boot.tool(".from_toml").unwrap().val().unwrap()
        );

        assert!(
            boot.tool(".from_toml")
                .unwrap()
                .at_dot("test.value")
                .str_match("hello world")
        );

        assert_eq!(
            boot.tool(".settings").at("label").str().unwrap(),
            "hello label"
        );
        assert_eq!(
            boot.tool(".settings").at("label2").str().unwrap(),
            "another hello label"
        );
    }
}
