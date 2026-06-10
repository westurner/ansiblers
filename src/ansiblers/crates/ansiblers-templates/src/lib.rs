//! `ansiblers-templates` — Jinja2 template rendering for ansiblers.
//!
//! Backed by [`jinja2rs`] (minijinja + Ansible-standard filters).
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use ansiblers_templates::{render_string, AnsibleTemplateEngine, TemplateEngineConfig, TrustLevel};
//! use std::collections::HashMap;
//! use serde_json::Value;
//!
//! let vars: HashMap<String, Value> = [("name".into(), Value::String("Alice".into()))].into();
//!
//! // Simple render (trusted, zero overhead):
//! let out = render_string("Hello {{ name }}!", &vars).unwrap();
//! assert_eq!(out, "Hello Alice!");
//!
//! // Sandboxed render (untrusted content — strict undefined, deny-list):
//! let engine = AnsibleTemplateEngine::with_config(TemplateEngineConfig::untrusted());
//! let out = engine.render("{{ name | upper }}", &vars).unwrap();
//! assert_eq!(out, "ALICE");
//! ```
//!
//! ## Trust levels
//!
//! | [`TrustLevel`] | When to use | Overhead |
//! |----------------|-------------|----------|
//! | `Trusted` (default) | Operator-written playbooks | Zero |
//! | `PartiallyTrusted` | Downloaded roles, inventory plugins | ~0 ns (no callable warnings) |
//! | `Untrusted` | Remote fetched content, API responses | ~2–8 µs/render (context walk) |
//!
//! The `sandboxed` feature flag must be enabled to activate
//! `jinja2rs::SandboxedEnvironment`; without it the engine logs a warning and
//! falls back to the standard engine.
//!
//! ## Ansible filters included
//!
//! `combine`, `regex_search`, `regex_replace`, `regex_findall`,
//! `to_nice_json`, `to_nice_yaml`, `from_json`, `from_yaml`,
//! `quote`, `path_join`, `upper`, `lower`, `default`, …

pub mod backend;
pub mod engine;

pub use backend::{
    render_via_minijinja, render_via_python, render_with_selected_backend, BackendSelector,
    TemplateBackend,
};
pub use engine::{
    render_string, render_string_sandboxed, render_value, AnsibleTemplateEngine,
    TemplateEngineConfig, TrustLevel,
};
