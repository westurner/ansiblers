//! `command` module — executes commands directly (no shell interpolation).

use std::process::Command;

use anyhow::Result;
use ansiblers_core::{ExecutionContext, TaskResult};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct CommandModule;

impl ModuleInvoker for CommandModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let raw = args
            .get_raw_params()
            .ok_or_else(|| anyhow::anyhow!("command module requires a command"))?
            .to_string();

        // Split into argv (simple whitespace split — no shell globbing).
        let argv: Vec<&str> = raw.split_whitespace().collect();
        if argv.is_empty() {
            return Ok(TaskResult::failed(host, "empty command"));
        }

        let chdir = args.get_str("chdir");
        let creates = args.get_str("creates");
        let removes = args.get_str("removes");

        if let Some(path) = creates {
            if std::path::Path::new(path).exists() {
                let mut r = TaskResult::ok(host);
                r.msg = format!("skipped, since {path} exists");
                return Ok(r);
            }
        }

        if let Some(path) = removes {
            if !std::path::Path::new(path).exists() {
                let mut r = TaskResult::ok(host);
                r.msg = format!("skipped, since {path} does not exist");
                return Ok(r);
            }
        }

        let mut command = Command::new(argv[0]);
        command.args(&argv[1..]);

        if let Some(dir) = chdir {
            command.current_dir(dir);
        }

        let output = command.output()?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let rc = output.status.code().unwrap_or(-1);

        let mut result = if output.status.success() {
            TaskResult::changed(host)
        } else {
            TaskResult::failed(host, stderr.trim().to_string())
        };
        result.stdout = stdout;
        result.stderr = stderr;
        result.rc = rc;
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
    fn test_command_true() {
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert("_raw_params".to_string(), Value::String("/bin/true".to_string()));
        let r = CommandModule
            .invoke(&ModuleArgs::new(args), "localhost", &mut ctx)
            .unwrap();
        assert_eq!(r.rc, 0);
    }

    #[test]
    fn test_command_echo() {
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "_raw_params".to_string(),
            Value::String("echo hello world".to_string()),
        );
        let r = CommandModule
            .invoke(&ModuleArgs::new(args), "localhost", &mut ctx)
            .unwrap();
        assert_eq!(r.stdout.trim(), "hello world");
    }
}
