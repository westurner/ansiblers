//! `ansiblers-executor` — task and playbook execution engine.
//!
//! Ties together parsing, inventory, variable resolution, template rendering,
//! and module invocation into a full playbook run.
//!
//! ## Key types
//!
//! | Type | Role |
//! |------|------|
//! | [`PlayExecutor`] | Runs a [`Playbook`] against an [`ExecutionContext`] |
//! | [`TaskExecutor`] | Runs a single [`Task`] (when/loop/register/changed_when) |
//! | [`Strategy`] | `Linear` (task-by-task, all hosts) or `Free` (thread-per-host) |
//!
//! ## Execution flow
//!
//! ```text
//! PlayExecutor::run_playbook
//!   └── for each Play:
//!         merge play vars → resolve hosts → choose Strategy
//!         └── TaskExecutor::run (per host × per task)
//!               ├── resolve vars (VariableResolver)
//!               ├── evaluate when: condition
//!               ├── expand loop: items
//!               ├── render module args (AnsibleTemplateEngine)
//!               └── ModuleRegistry::invoke
//! ```
//!
//! ## Trust level
//!
//! The executor propagates a [`TrustLevel`] from `PlayExecutor` → `TaskExecutor`
//! → `AnsibleTemplateEngine`. Set `TrustLevel::Untrusted` when playbook content
//! originates from an untrusted source to activate the sandboxed template engine.
//!
//! ```rust,no_run
//! use ansiblers_executor::PlayExecutor;
//! use ansiblers_modules::ModuleRegistry;
//! use ansiblers_templates::TrustLevel;
//!
//! let executor = PlayExecutor::with_trust(ModuleRegistry::with_defaults(), TrustLevel::PartiallyTrusted);
//! ```

pub mod play;
pub mod strategy;
pub mod task;

pub use play::{PlayExecutor, PlayResult, PlaybookResult};
pub use strategy::Strategy;
pub use task::TaskExecutor;
