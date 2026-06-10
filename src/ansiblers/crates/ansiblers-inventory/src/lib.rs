//! `ansiblers-inventory` — inventory loading from INI and YAML files.
//!
//! Parses Ansible inventory files into [`ansiblers_core::Inventory`] (a flat
//! collection of [`Host`]s and [`Group`]s with variable maps).
//!
//! ## Supported formats
//!
//! | Format | Extension | Notes |
//! |--------|-----------|-------|
//! | INI | `.ini` (default) | Sections, `:vars`, `:children` |
//! | YAML | `.yml` / `.yaml` | `all.hosts`, `all.children`, group `vars:` |
//!
//! ## Usage
//!
//! ```rust,no_run
//! use ansiblers_inventory::load_inventory;
//!
//! let inventory = load_inventory("inventory.ini").unwrap();
//! let hosts = inventory.matching_hosts("webservers");
//! ```
//!
//! ## Variable precedence (inventory level)
//!
//! `group_vars/all` < `group_vars/<group>` < `host_vars/<host>`
//!
//! The returned [`ansiblers_core::Inventory::host_vars`] method merges these
//! tiers in the correct order.

pub mod dynamic;
pub mod group_host_vars;
pub mod ini;
pub mod loader;
pub mod yaml;

pub use dynamic::{parse_list_output, DynamicInventoryScript};
pub use group_host_vars::{load_vars_file, merge_group_and_host_vars};
pub use loader::{
    load_inventory, FileInventoryLoader, InlineFormat, InlineInventoryLoader, InventoryLoader,
};
