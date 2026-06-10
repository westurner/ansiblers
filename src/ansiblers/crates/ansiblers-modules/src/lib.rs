//! `ansiblers-modules` — module registry and built-in module implementations.
//!
//! Modules are the units of work in Ansible playbooks.  This crate provides:
//!
//! ## Built-in Rust modules
//!
//! | Module | Phase | Description |
//! |--------|-------|-------------|
//! | `shell` | 1 | Execute commands via `/bin/sh` |
//! | `command` | 1 | Execute commands without shell interpolation |
//! | `debug` | 1 | Print messages or variable values |
//! | `set_fact` | 1 | Store per-host facts in the execution context |
//! | `fail` | 1 | Immediately fail with a custom message |
//! | `file` | 2 | Manage file/directory/symlink state |
//! | `copy` | 2 | Copy files or write string content to a destination |
//! | `stat` | 2 | Gather file/directory metadata |
//!
//! ## Python module invocation
//!
//! [`ConfigurablePythonInvoker`] supports three backends selected via
//! [`PythonInvokeMode`]:
//!
//! | Mode | Mechanism | Parallelism |
//! |------|-----------|-------------|
//! | `Subprocess` (default) | `python3 <module> <args_file>` | Full (separate process) |
//! | `Native` | PyO3 in-process `exec()` | Concurrent (I/O yields GIL) |
//! | `SubInterpreter` | Python 3.12+ `_interpreters` | Full (own GIL per interpreter) |
//!
//! ## Preview Mode sandbox
//!
//! [`PreviewModeWrapper`] wraps any `ModuleInvoker` in a bubblewrap container
//! with OverlayFS diff capture, emulating Ansible `--diff` / `--check` modes.
//!
//! ## Registry
//!
//! ```rust,no_run
//! use ansiblers_modules::ModuleRegistry;
//!
//! // All built-in modules pre-registered:
//! let mut registry = ModuleRegistry::with_defaults();
//!
//! // Fallback to Python for unknown modules:
//! use ansiblers_modules::{ConfigurablePythonInvoker, PythonModuleConfig};
//! use std::sync::Arc;
//! let python = Arc::new(ConfigurablePythonInvoker::default_subprocess());
//! registry.register("my_custom_module", python);
//! ```

pub mod command;
pub mod copy;
pub mod debug;
pub mod fail;
pub mod file;
pub mod preview;
pub mod python_wrapper;
pub mod registry;
pub mod set_fact;
pub mod shell;
pub mod stat;

pub use preview::{is_bwrap_available, PreviewModeWrapper};
pub use python_wrapper::{
    discover_ansible_library_paths, AnsiblePythonModuleInvoker, ConfigurablePythonInvoker,
    PythonInvokeMode, PythonModuleConfig, PythonModuleWrapper, SubprocessInvoker,
};
#[cfg(feature = "native-python")]
pub use python_wrapper::{NativePythonInvoker, SubInterpreterInvoker};
pub use registry::{ModuleArgs, ModuleInvoker, ModuleRegistry};
