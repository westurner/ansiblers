//! `ansiblers-parser` — YAML playbook parser producing a typed AST.
//!
//! Parses standard Ansible playbook YAML into Rust structs with `serde_yaml`.
//!
//! ## Entry points
//!
//! ```rust,no_run
//! use ansiblers_parser::{parse_playbook, parse_playbook_str};
//!
//! // From a file:
//! let pb = parse_playbook("site.yml").unwrap();
//!
//! // From an in-memory string:
//! let pb = parse_playbook_str("- hosts: all\n  tasks: []", None).unwrap();
//! ```
//!
//! ## AST overview
//!
//! ```text
//! Playbook
//! └── Vec<Play>
//!     ├── hosts: String       ("all", "webservers", etc.)
//!     ├── vars: HashMap
//!     ├── tasks: Vec<TaskNode>
//!     │   ├── TaskNode::Task(Task)    — regular task
//!     │   └── TaskNode::Block(Block) — block/rescue/always
//!     └── handlers: Vec<Handler>
//! ```
//!
//! ## Supported directives
//!
//! Tasks: `name`, `when`, `register`, `loop`/`with_items`, `loop_control`,
//! `notify`, `tags`, `become`, `ignore_errors`, `failed_when`,
//! `changed_when`, `no_log`, `delegate_to`.
//!
//! Blocks: `block`, `rescue`, `always`, `when`, `tags`.

pub mod ast;
pub mod parse;

pub use ast::{Block, Handler, Play, Playbook, Task, TaskArgs, TaskNode, WhenExpr};
pub use parse::{parse_playbook, parse_playbook_str};
