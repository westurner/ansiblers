//! `set_fact` module — stores per-host facts in the execution context.

use ansiblers_core::{ExecutionContext, TaskResult};
use anyhow::Result;

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct SetFactModule;

impl ModuleInvoker for SetFactModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        if args.args.is_empty() {
            return Ok(TaskResult::failed(
                host,
                "set_fact requires at least one key=value pair",
            ));
        }

        for (key, val) in &args.args {
            if key == "_raw_params" {
                continue;
            }
            ctx.set_fact(host, key.clone(), val.clone());
        }

        let mut result = TaskResult::ok(host);
        result.changed = true;
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
    fn test_set_fact() {
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert("my_fact".to_string(), Value::String("42".to_string()));
        SetFactModule
            .invoke(&ModuleArgs::new(args), "localhost", &mut ctx)
            .unwrap();
        assert_eq!(
            ctx.get_fact("localhost", "my_fact"),
            Some(&Value::String("42".to_string()))
        );
    }

    #[test]
    fn test_set_fact_multiple_keys() {
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert("key_a".to_string(), Value::String("val_a".to_string()));
        args.insert("key_b".to_string(), Value::Bool(true));
        SetFactModule
            .invoke(&ModuleArgs::new(args), "host1", &mut ctx)
            .unwrap();
        assert_eq!(
            ctx.get_fact("host1", "key_a"),
            Some(&Value::String("val_a".to_string()))
        );
        assert_eq!(ctx.get_fact("host1", "key_b"), Some(&Value::Bool(true)));
    }

    #[test]
    fn test_set_fact_number_value() {
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert("count".to_string(), Value::Number(7.into()));
        SetFactModule
            .invoke(&ModuleArgs::new(args), "h", &mut ctx)
            .unwrap();
        assert!(ctx.get_fact("h", "count").is_some());
    }

    #[test]
    fn test_set_fact_returns_ok() {
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert("x".to_string(), Value::Null);
        let r = SetFactModule
            .invoke(&ModuleArgs::new(args), "h", &mut ctx)
            .unwrap();
        assert!(r.status.is_ok());
    }

    #[test]
    fn test_set_fact_per_host_isolation() {
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert("env".to_string(), Value::String("prod".to_string()));
        SetFactModule
            .invoke(&ModuleArgs::new(args.clone()), "host_a", &mut ctx)
            .unwrap();
        // host_b should not have the fact
        assert!(ctx.get_fact("host_b", "env").is_none());
        assert!(ctx.get_fact("host_a", "env").is_some());
    }
}
