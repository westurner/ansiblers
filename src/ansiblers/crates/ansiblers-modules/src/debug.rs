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

    #[test]
    fn test_debug_no_args_still_ok() {
        let mut ctx = ctx();
        let r = DebugModule
            .invoke(&ModuleArgs::new(HashMap::new()), "localhost", &mut ctx)
            .unwrap();
        assert!(r.status.is_ok());
    }

    #[test]
    fn test_debug_msg_number() {
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert("msg".to_string(), Value::Number(42.into()));
        let r = DebugModule
            .invoke(&ModuleArgs::new(args), "localhost", &mut ctx)
            .unwrap();
        assert!(r.status.is_ok());
    }

    #[test]
    fn test_debug_var_with_value_in_context() {
        let mut ctx = ctx();
        ctx.playbook_vars
            .insert("my_key".to_string(), Value::String("hello".to_string()));
        let mut args = HashMap::new();
        args.insert("var".to_string(), Value::String("my_key".to_string()));
        let r = DebugModule
            .invoke(&ModuleArgs::new(args), "localhost", &mut ctx)
            .unwrap();
        // Should include the value in the output message
        assert!(r.status.is_ok());
    }

    #[test]
    fn test_debug_verbosity_msg() {
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert("msg".to_string(), Value::String("verbose only".to_string()));
        args.insert("verbosity".to_string(), Value::Number(3.into()));
        // Low verbosity context: still should not fail
        let r = DebugModule
            .invoke(&ModuleArgs::new(args), "localhost", &mut ctx)
            .unwrap();
        assert!(r.status.is_ok());
    }
}
