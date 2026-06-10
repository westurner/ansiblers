use anyhow::{Context, Result};
use ansiblers_core::Inventory;

use crate::ini::parse_ini_inventory;
use crate::yaml::parse_yaml_inventory;

/// Load an inventory file, auto-detecting format (INI vs YAML).
pub fn load_inventory(path: &str) -> Result<Inventory> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading inventory '{path}'"))?;
    if path.ends_with(".yml") || path.ends_with(".yaml") {
        parse_yaml_inventory(&content)
    } else {
        parse_ini_inventory(&content)
    }
}

/// Trait for pluggable inventory sources.
pub trait InventoryLoader: Send + Sync {
    fn load(&self, source: &str) -> Result<Inventory>;
}

/// File-based inventory loader.
pub struct FileInventoryLoader;

impl InventoryLoader for FileInventoryLoader {
    fn load(&self, source: &str) -> Result<Inventory> {
        load_inventory(source)
    }
}

/// Inline string loader (used in tests).
pub struct InlineInventoryLoader {
    pub format: InlineFormat,
}

pub enum InlineFormat {
    Ini,
    Yaml,
}

impl InventoryLoader for InlineInventoryLoader {
    fn load(&self, source: &str) -> Result<Inventory> {
        match self.format {
            InlineFormat::Ini => parse_ini_inventory(source),
            InlineFormat::Yaml => parse_yaml_inventory(source),
        }
    }
}
