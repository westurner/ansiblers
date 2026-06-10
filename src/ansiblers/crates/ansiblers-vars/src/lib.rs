//! `ansiblers-vars` — variable resolution engine with Ansible precedence tiers.
//!
//! Implements the [Ansible variable precedence][ap] model as a 9-level
//! [`VarScope`] enum. The [`VariableResolver`] merges all scopes into a flat
//! `HashMap<String, Value>` for template rendering.
//!
//! ## Precedence order (lowest → highest)
//!
//! ```text
//! RoleDefaults < Inventory < GroupVarsAll < GroupVars < HostVars
//!   < PlayVars < RoleVars < TaskVars < SetFact < ExtraVars
//! ```
//!
//! ## Usage
//!
//! ```rust,ignore
//! use ansiblers_vars::VariableResolver;
//!
//! // Attached to an ExecutionContext — picks up play vars, registered vars,
//! // host facts, and inventory vars automatically:
//! let resolver = VariableResolver::new(&ctx);
//! let flat = resolver.merged("web1.example.com");
//! ```
//!
//! ```rust
//! use ansiblers_vars::{StandaloneResolver, VarScope};
//! use serde_json::Value;
//!
//! let mut r = StandaloneResolver::new();
//! r.set(VarScope::PlayVars, "greeting", Value::String("hello".into()));
//! assert_eq!(r.get("greeting"), Some(Value::String("hello".into())));
//! ```
//!
//! [ap]: https://docs.ansible.com/ansible/latest/playbook_guide/playbooks_variables.html#understanding-variable-precedence

pub mod precedence;
pub mod resolver;

pub use precedence::VarScope;
pub use resolver::{StandaloneResolver, VariableResolver};
