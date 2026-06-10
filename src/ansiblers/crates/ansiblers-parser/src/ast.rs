use std::collections::HashMap;
use std::path::PathBuf;

use ansiblers_core::Value;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Playbook / Play
// ---------------------------------------------------------------------------

/// Top-level playbook document (list of plays).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playbook {
    pub plays: Vec<Play>,
    pub path: Option<PathBuf>,
}

/// A single play (maps `hosts:` → task list).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Play {
    pub name: Option<String>,
    pub hosts: String,
    pub tasks: Vec<TaskNode>,
    pub handlers: Vec<Handler>,
    pub vars: HashMap<String, Value>,
    pub vars_files: Vec<String>,
    #[serde(default, rename = "become")]
    pub r#become: bool,
    pub become_user: Option<String>,
    #[serde(default = "default_true")]
    pub gather_facts: bool,
    pub tags: Vec<String>,
    pub any_errors_fatal: bool,
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Task node variants
// ---------------------------------------------------------------------------

/// A node inside a task list — either a plain task or a block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TaskNode {
    Block(Block),
    Task(Task),
}

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

/// A single task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Human-readable task name.
    pub name: Option<String>,
    /// Module name (e.g. `shell`, `debug`, `copy`).
    pub module: String,
    /// Module arguments.
    pub args: TaskArgs,
    pub when: Option<WhenExpr>,
    pub register: Option<String>,
    pub loop_items: Option<Value>,
    /// `loop_var` overrides the default `item` loop variable.
    pub loop_var: String,
    pub notify: Vec<String>,
    pub tags: Vec<String>,
    #[serde(rename = "become")]
    pub r#become: Option<bool>,
    pub become_user: Option<String>,
    #[serde(default)]
    pub ignore_errors: bool,
    pub failed_when: Option<String>,
    pub changed_when: Option<String>,
    #[serde(default)]
    pub no_log: bool,
    pub delegate_to: Option<String>,
}

impl Task {
    pub fn new(module: impl Into<String>, args: TaskArgs) -> Self {
        Self {
            name: None,
            module: module.into(),
            args,
            when: None,
            register: None,
            loop_items: None,
            loop_var: "item".to_string(),
            notify: Vec::new(),
            tags: Vec::new(),
            r#become: None,
            become_user: None,
            ignore_errors: false,
            failed_when: None,
            changed_when: None,
            no_log: false,
            delegate_to: None,
        }
    }

    /// Returns true if this task has a loop directive.
    pub fn has_loop(&self) -> bool {
        self.loop_items.is_some()
    }
}

/// Handler is structurally identical to a Task.
pub type Handler = Task;

// ---------------------------------------------------------------------------
// Task arguments
// ---------------------------------------------------------------------------

/// Module arguments: either a free-form string or a key/value dict.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TaskArgs {
    /// e.g. `shell: echo hello` — entire value is the command string.
    FreeForm(String),
    /// e.g. `debug:\n  msg: "..."` — dict of named parameters.
    Dict(HashMap<String, Value>),
}

impl TaskArgs {
    /// Look up a named argument.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Self::Dict(m) => m.get(key),
            Self::FreeForm(_) => None,
        }
    }

    /// The raw free-form string (or `_raw_params` from a dict).
    pub fn as_free_form(&self) -> Option<&str> {
        match self {
            Self::FreeForm(s) => Some(s.as_str()),
            Self::Dict(m) => m.get("_raw_params").and_then(|v| v.as_str()),
        }
    }

    /// Flatten to a HashMap (free-form string becomes `_raw_params`).
    pub fn as_dict(&self) -> HashMap<String, Value> {
        match self {
            Self::Dict(m) => m.clone(),
            Self::FreeForm(s) => {
                let mut m = HashMap::new();
                m.insert("_raw_params".to_string(), Value::String(s.clone()));
                m
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Block
// ---------------------------------------------------------------------------

/// A `block:` / `rescue:` / `always:` grouping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub name: Option<String>,
    pub block: Vec<TaskNode>,
    #[serde(default)]
    pub rescue: Vec<TaskNode>,
    #[serde(default)]
    pub always: Vec<TaskNode>,
    pub when: Option<WhenExpr>,
    pub tags: Vec<String>,
}

// ---------------------------------------------------------------------------
// When expression
// ---------------------------------------------------------------------------

/// `when:` accepts either a single string or a list of strings (AND logic).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WhenExpr {
    Single(String),
    List(Vec<String>),
}

impl WhenExpr {
    /// Returns each condition as a borrowed string slice.
    pub fn conditions(&self) -> Vec<&str> {
        match self {
            Self::Single(s) => vec![s.as_str()],
            Self::List(v) => v.iter().map(|s| s.as_str()).collect(),
        }
    }
}
