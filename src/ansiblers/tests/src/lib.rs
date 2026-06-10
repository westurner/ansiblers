// Integration test library — shared fixtures and helpers.
// Test binaries import this via `use ansiblers_it::*;`.

use std::collections::HashMap;
use std::sync::Arc;

use ansiblers_core::{ExecutionContext, Inventory};
use ansiblers_inventory::load_inventory;

/// Return absolute path to a fixture file.
///
/// Fixtures live at `<workspace_root>/tests/fixtures/<relative>`.
pub fn fixture_path(relative: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    format!("{manifest_dir}/fixtures/{relative}")
}

/// Build an empty [`ExecutionContext`] pointing at an empty inventory.
pub fn empty_ctx() -> ExecutionContext {
    ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new())
}

/// Build an [`ExecutionContext`] from a fixture inventory file.
pub fn ctx_with_inventory(inv_file: &str) -> ExecutionContext {
    let path = fixture_path(&format!("inventories/{inv_file}"));
    let inventory = load_inventory(&path).expect("load inventory");
    ExecutionContext::new(Arc::new(inventory), HashMap::new())
}
