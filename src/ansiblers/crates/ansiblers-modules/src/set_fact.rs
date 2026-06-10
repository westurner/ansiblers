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
}
