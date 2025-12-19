use crate::Record;

pub type Indexer = fn(&Record) -> std::io::Result<Vec<u8>>;

/// Various types of "tool" functions that can be applied during the protocol encoding
#[derive(Debug, Clone)]
pub enum Tool {
    Indexer(Indexer),
    Setting(String),
}

impl From<Indexer> for Tool {
    fn from(value: Indexer) -> Self {
        Tool::Indexer(value)
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
        "#,
        );

        let target = wire
            .encode(rec, Some(vec!["projection/from_toml", "label", "label2"]))
            .unwrap();

        let boot = Boot::decode(target.filled()).unwrap();

        assert!(boot.is_boot_inline());
        assert_eq!(boot.layout().frames.len(), 4);
        eprintln!(
            "{}",
            boot.tool("projection/from_toml").unwrap().val().unwrap()
        );

        assert!(
            boot.tool("projection/from_toml")
                .unwrap()
                .at_dot("test.value")
                .str_match("hello world")
        );
        assert_eq!(
            boot.tool("setting").unwrap().at("label").str().unwrap(),
            "hello label"
        );
        assert_eq!(
            boot.tool("setting").unwrap().at("label2").str().unwrap(),
            "another hello label"
        );
    }
}
