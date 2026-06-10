//! Module registry — maps module names to their implementations.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use ansiblers_core::{ExecutionContext, TaskResult};

use crate::command::CommandModule;
use crate::debug::DebugModule;
use crate::fail::FailModule;
use crate::set_fact::SetFactModule;
use crate::shell::ShellModule;

/// Arguments passed to a module at invocation time.
#[derive(Debug, Clone)]
pub struct ModuleArgs {
    pub args: HashMap<String, ansiblers_core::Value>,
    pub task_name: Option<String>,
}

impl ModuleArgs {
    pub fn new(args: HashMap<String, ansiblers_core::Value>) -> Self {
        Self { args, task_name: None }
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.args.get(key).and_then(|v| v.as_str())
    }

    pub fn get_raw_params(&self) -> Option<&str> {
        self.get_str("_raw_params")
    }
}

/// Trait implemented by every module.
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
pub struct ModuleRegistry {
    modules: HashMap<String, Arc<dyn ModuleInvoker>>,
}

impl ModuleRegistry {
    /// Create a registry pre-populated with Phase 1 built-in modules.
    pub fn with_defaults() -> Self {
        let mut r = Self {
            modules: HashMap::new(),
        };
        r.register("shell", Arc::new(ShellModule));
        r.register("command", Arc::new(CommandModule));
        r.register("debug", Arc::new(DebugModule));
        r.register("set_fact", Arc::new(SetFactModule));
        r.register("fail", Arc::new(FailModule));
        r
    }

    pub fn register(&mut self, name: &str, invoker: Arc<dyn ModuleInvoker>) {
        self.modules.insert(name.to_string(), invoker);
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
