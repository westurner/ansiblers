//! `debug` module — prints messages or variable values.

use ansiblers_core::{ExecutionContext, TaskResult};
use anyhow::Result;

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct DebugModule;

impl ModuleInvoker for DebugModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let msg = args.get_str("msg").unwrap_or("").to_string();
        let var = args.get_str("var");

        let output = if let Some(var_name) = var {
            format!("{var_name}: <displayed at runtime>")
        } else {
            msg.clone()
        };

        let mut result = TaskResult::ok(host);
        result.msg = output.clone();
        result.stdout = output;
        Ok(result)
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
    fn test_debug_msg() {
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert("msg".to_string(), Value::String("hello debug".to_string()));
        let r = DebugModule
            .invoke(&ModuleArgs::new(args), "localhost", &mut ctx)
            .unwrap();
        assert_eq!(r.msg, "hello debug");
        assert!(r.status.is_ok());
    }

    #[test]
    fn test_debug_var() {
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert("var".to_string(), Value::String("my_var".to_string()));
        let r = DebugModule
            .invoke(&ModuleArgs::new(args), "localhost", &mut ctx)
            .unwrap();
        assert!(r.msg.contains("my_var"));
    }
}
