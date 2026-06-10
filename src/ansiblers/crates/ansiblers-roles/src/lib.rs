//! `ansiblers-roles` — Ansible role loading, dependency resolution, and galaxy support.
//!
//! ## Role directory structure
//!
//! ```text
//! roles/
//! └── my_role/
//!     ├── defaults/
//!     │   └── main.yml        ← lowest-precedence role variables
//!     ├── vars/
//!     │   └── main.yml        ← high-precedence role variables
//!     ├── tasks/
//!     │   ├── main.yml        ← task entry point
//!     │   └── subtasks.yml    ← included via include_tasks:
//!     ├── handlers/
//!     │   └── main.yml
//!     ├── templates/          ← Jinja2 templates (.j2)
//!     ├── files/              ← static files
//!     ├── meta/
//!     │   └── main.yml        ← role metadata and dependencies
//!     └── README.md
//! ```
//!
//! ## Role resolution
//!
//! Roles are searched in order through a configurable [`RolePath`]:
//! 1. `roles/` relative to the playbook
//! 2. `~/.ansible/roles`
//! 3. `/etc/ansible/roles`
//! 4. Paths from `ANSIBLE_ROLES_PATH` env var
//!
//! ## Usage
//!
//! ```rust,no_run
//! use ansiblers_roles::{RoleLoader, RolePath};
//! use std::path::PathBuf;
//!
//! let role_path = RolePath::new(vec![PathBuf::from("roles")]);
//! let loader = RoleLoader::new(role_path);
//! let role = loader.load("my_role").unwrap();
//! println!("{} tasks", role.tasks.len());
//! ```

pub mod dependency;
pub mod galaxy;
pub mod loader;
pub mod meta;
pub mod role;

pub use dependency::{resolve_dependencies, DependencyGraph};
pub use galaxy::{GalaxyRequirements, RoleRequirement};
pub use loader::{RoleLoader, RolePath};
pub use meta::RoleMeta;
pub use role::{Role, RoleDefaults, RoleFiles};
