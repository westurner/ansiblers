//! Shared test helpers and fixtures for integration tests.

use std::collections::HashMap;
use std::sync::Arc;

use ansiblers_core::{ExecutionContext, Inventory};
use ansiblers_inventory::load_inventory;

pub fn fixture_path(relative: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    format!("{manifest_dir}/../fixtures/{relative}")
}

pub fn empty_ctx() -> ExecutionContext {
    ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new())
}

pub fn ctx_with_inventory(inv_file: &str) -> ExecutionContext {
    let path = fixture_path(&format!("inventories/{inv_file}"));
    let inventory = load_inventory(&path).expect("load inventory");
    ExecutionContext::new(Arc::new(inventory), HashMap::new())
}
