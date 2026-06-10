//! `shell` module — executes commands through `/bin/sh`.

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult};
use anyhow::Result;

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct ShellModule;

impl ModuleInvoker for ShellModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let cmd = args
            .get_raw_params()
            .ok_or_else(|| anyhow::anyhow!("shell module requires a command"))?
            .to_string();

        let chdir = args.get_str("chdir");
        let creates = args.get_str("creates");
        let removes = args.get_str("removes");

        // `creates` skip logic.
        if let Some(path) = creates {
            if std::path::Path::new(path).exists() {
                let mut r = TaskResult::ok(host);
                r.msg = format!("skipped, since {path} exists");
                return Ok(r);
            }
        }

        // `removes` skip logic.
        if let Some(path) = removes {
            if !std::path::Path::new(path).exists() {
                let mut r = TaskResult::ok(host);
                r.msg = format!("skipped, since {path} does not exist");
                return Ok(r);
            }
        }

        let mut command = Command::new("/bin/sh");
        command.args(["-c", &cmd]);

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
    use ansiblers_core::{ExecutionContext, Inventory};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn ctx() -> ExecutionContext {
        ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new())
    }

    #[test]
    fn test_shell_echo() {
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "_raw_params".to_string(),
            ansiblers_core::Value::String("echo hello".to_string()),
        );
        let result = ShellModule
            .invoke(&ModuleArgs::new(args), "localhost", &mut ctx)
            .unwrap();
        assert_eq!(result.stdout.trim(), "hello");
        assert_eq!(result.rc, 0);
    }

    #[test]
    fn test_shell_failure() {
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "_raw_params".to_string(),
            ansiblers_core::Value::String("/bin/false".to_string()),
        );
        let result = ShellModule
            .invoke(&ModuleArgs::new(args), "localhost", &mut ctx)
            .unwrap();
        assert_ne!(result.rc, 0);
        assert!(result.status.is_failed());
    }

    #[test]
    fn test_shell_creates_skip() {
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "_raw_params".to_string(),
            ansiblers_core::Value::String("echo should_skip".to_string()),
        );
        // /etc/hostname always exists on Linux.
        args.insert(
            "creates".to_string(),
            ansiblers_core::Value::String("/etc/hostname".to_string()),
        );
        let result = ShellModule
            .invoke(&ModuleArgs::new(args), "localhost", &mut ctx)
            .unwrap();
        assert!(result.status.is_ok());
        assert!(result.msg.contains("exists"));
    }
}
