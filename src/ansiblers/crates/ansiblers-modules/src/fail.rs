//! `fail` module — immediately fails the task with a custom message.

use ansiblers_core::{ExecutionContext, TaskResult};
use anyhow::Result;

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct FailModule;

impl ModuleInvoker for FailModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let msg = args
            .get_str("msg")
            .unwrap_or("Failed as requested")
            .to_string();
        Ok(TaskResult::failed(host, msg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ModuleArgs;
    use ansiblers_core::{ExecutionContext, Inventory, Value};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn ctx() -> ExecutionContext {
        ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new())
    }

    #[test]
    fn test_fail_with_msg() {
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert("msg".to_string(), Value::String("intentional".to_string()));
        let r = FailModule
            .invoke(&ModuleArgs::new(args), "localhost", &mut ctx)
            .unwrap();
        assert!(r.status.is_failed());
        assert_eq!(r.msg, "intentional");
    }

    #[test]
    fn test_fail_default_msg() {
        let mut ctx = ctx();
        let r = FailModule
            .invoke(&ModuleArgs::new(HashMap::new()), "localhost", &mut ctx)
            .unwrap();
        assert!(r.status.is_failed());
    }
}
