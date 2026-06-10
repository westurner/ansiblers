//! Role data model — the in-memory representation of a loaded Ansible role.

use std::collections::HashMap;
use std::path::PathBuf;

use ansiblers_core::Value;
use ansiblers_parser::{Handler, Play, TaskNode};
use serde::{Deserialize, Serialize};

/// A fully loaded Ansible role.
#[derive(Debug, Clone)]
pub struct Role {
    /// Role name (directory name).
    pub name: String,

    /// Absolute path to the role root directory.
    pub path: PathBuf,

    /// Tasks from `tasks/main.yml` (and any included task files resolved at load time).
    pub tasks: Vec<TaskNode>,

    /// Handlers from `handlers/main.yml`.
    pub handlers: Vec<Handler>,

    /// Variables from `defaults/main.yml` (lowest precedence).
    pub defaults: HashMap<String, Value>,

    /// Variables from `vars/main.yml` (high precedence, overrides play vars).
    pub vars: HashMap<String, Value>,

    /// Metadata from `meta/main.yml`.
    pub meta: crate::meta::RoleMeta,
}

impl Role {
    pub fn new(name: impl Into<String>, path: PathBuf) -> Self {
        Self {
            name: name.into(),
            path,
            tasks: Vec::new(),
            handlers: Vec::new(),
            defaults: HashMap::new(),
            vars: HashMap::new(),
            meta: crate::meta::RoleMeta::default(),
        }
    }

    /// Return the `tasks/` directory path.
    pub fn tasks_dir(&self) -> PathBuf {
        self.path.join("tasks")
    }

    /// Return the `templates/` directory path.
    pub fn templates_dir(&self) -> PathBuf {
        self.path.join("templates")
    }

    /// Return the `files/` directory path.
    pub fn files_dir(&self) -> PathBuf {
        self.path.join("files")
    }

    /// Returns `true` if this role has any tasks.
    pub fn has_tasks(&self) -> bool {
        !self.tasks.is_empty()
    }
}

/// Resolved file paths within a role.
#[derive(Debug, Clone, Default)]
pub struct RoleFiles {
    pub tasks: Vec<PathBuf>,
    pub handlers: Vec<PathBuf>,
    pub defaults: Vec<PathBuf>,
    pub vars: Vec<PathBuf>,
    pub templates: Vec<PathBuf>,
    pub files: Vec<PathBuf>,
}

/// Default variable set for a role (from `defaults/main.yml`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoleDefaults {
    pub vars: HashMap<String, Value>,
}
