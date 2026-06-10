//! Module registry — maps module names to their implementations.

use std::collections::HashMap;
use std::sync::Arc;

use ansiblers_core::{ExecutionContext, TaskResult};
use anyhow::Result;

use crate::apt::AptModule;
use crate::command::CommandModule;
use crate::copy::CopyModule;
use crate::debug::DebugModule;
use crate::fail::FailModule;
use crate::file::FileModule;
use crate::find::FindModule;
use crate::git::GitModule;
use crate::lineinfile::LineinfileModule;
use crate::set_fact::SetFactModule;
use crate::setup::SetupModule;
use crate::shell::ShellModule;
use crate::stat::StatModule;
use crate::template::TemplateModule;
use crate::yum::YumModule;

/// Arguments passed to a module at invocation time.
///
/// `args` holds the rendered key/value parameters from the task YAML.
/// `task_name` is populated from `task.name` for use in error messages.
#[derive(Debug, Clone)]
pub struct ModuleArgs {
    pub args: HashMap<String, ansiblers_core::Value>,
    pub task_name: Option<String>,
}

impl ModuleArgs {
    pub fn new(args: HashMap<String, ansiblers_core::Value>) -> Self {
        Self {
            args,
            task_name: None,
        }
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.args.get(key).and_then(|v| v.as_str())
    }

    pub fn get_raw_params(&self) -> Option<&str> {
        self.get_str("_raw_params")
    }
}

/// Trait implemented by every module.
///
/// Each implementation corresponds to one Ansible module name (e.g. `"shell"`,
/// `"copy"`, `"apt"`).  Modules receive rendered arguments, the target
/// hostname, and mutable access to the [`ExecutionContext`] (to write facts
/// via `set_fact` or read registered variables).
///
/// All built-in modules are synchronous and blocking.  Async support is planned
/// for Phase 6 (WebRTC transport).
pub trait ModuleInvoker: Send + Sync {
    /// Execute the module synchronously for a single host.
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult>;
}

/// Registry that maps module names → [`ModuleInvoker`] implementations.
///
/// Use [`ModuleRegistry::with_defaults`] to get a registry pre-populated with
/// all Phase 1+2 built-in Rust modules.  Additional modules (Python wrappers,
/// custom Rust modules) can be registered at runtime via [`register`](Self::register).
///
/// # Fallback strategy
///
/// Unregistered modules return a failed [`TaskResult`] with `"module not found"`
/// rather than panicking.  To support arbitrary Python modules transparently,
/// wrap the registry with a [`ConfigurablePythonInvoker`](crate::ConfigurablePythonInvoker)
/// as the default invoker.
pub struct ModuleRegistry {
    modules: HashMap<String, Arc<dyn ModuleInvoker>>,
}

impl ModuleRegistry {
    /// Create a registry pre-populated with Phase 1+2 built-in modules.
    pub fn with_defaults() -> Self {
        let mut r = Self {
            modules: HashMap::new(),
        };
        // Phase 1
        r.register("shell", Arc::new(ShellModule));
        r.register("command", Arc::new(CommandModule));
        r.register("debug", Arc::new(DebugModule));
        r.register("set_fact", Arc::new(SetFactModule));
        r.register("fail", Arc::new(FailModule));
        // Phase 2
        r.register("file", Arc::new(FileModule));
        r.register("copy", Arc::new(CopyModule));
        r.register("stat", Arc::new(StatModule));
        // Phase 5
        r.register("apt", Arc::new(AptModule));
        r.register("yum", Arc::new(YumModule));
        r.register("dnf", Arc::new(YumModule));
        r.register("find", Arc::new(FindModule));
        r.register("template", Arc::new(TemplateModule));
        r.register("lineinfile", Arc::new(LineinfileModule));
        r.register("setup", Arc::new(SetupModule));
        r.register("gather_facts", Arc::new(SetupModule));
        r.register("git", Arc::new(GitModule));
        r
    }

    pub fn register(&mut self, name: &str, invoker: Arc<dyn ModuleInvoker>) {
        self.modules.insert(name.to_string(), invoker);
    }

    /// Clone the registry by re-creating it with defaults.
    /// Used by the free-strategy multi-thread executor.
    pub fn clone_defaults() -> Self {
        Self::with_defaults()
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn ModuleInvoker>> {
        self.modules.get(name).cloned()
    }

    pub fn invoke(
        &self,
        module: &str,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        match self.modules.get(module) {
            Some(invoker) => invoker.invoke(args, host, ctx),
            None => Ok(TaskResult::failed(
                host,
                format!("module not found: {module}"),
            )),
        }
    }
}
