//! `ansiblers-core` — foundational types shared across all ansiblers crates.
//!
//! This crate provides the data structures that thread through every stage of
//! playbook execution:
//!
//! | Type | Role |
//! |------|------|
//! | [`ExecutionContext`] | Global mutable state: inventory, vars, facts, verbosity |
//! | [`Inventory`] / [`Host`] / [`Group`] | Loaded host/group topology |
//! | [`HostState`] | Per-host runtime state (failed, changed count, …) |
//! | [`TaskResult`] / [`TaskStatus`] | Module execution outcome |
//! | [`Value`] | Dynamic JSON value (re-export of `serde_json::Value`) |
//!
//! ## Design notes
//!
//! - No I/O in this crate — pure data structures and error types.
//! - All public types derive `Debug`, `Clone`, `Serialize`, `Deserialize`.
//! - [`AnsiblersError`] covers every error category; downstream crates use
//!   `anyhow` for context attachment.

pub mod context;
pub mod error;
pub mod host;
pub mod inventory;
pub mod result;

pub use context::ExecutionContext;
pub use error::{AnsiblersError, Result};
pub use host::HostState;
pub use inventory::{Group, Host, Inventory};
pub use result::{TaskResult, TaskStatus};

/// Dynamic value type used throughout ansiblers (re-export of serde_json::Value).
pub use serde_json::Value;
