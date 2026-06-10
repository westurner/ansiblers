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

    #[test]
    fn test_shell_removes_skip_when_absent() {
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "_raw_params".to_string(),
            ansiblers_core::Value::String("echo run".to_string()),
        );
        // This file does not exist → removes condition not met → skip.
        args.insert(
            "removes".to_string(),
            ansiblers_core::Value::String("/nonexistent/xyz12345".to_string()),
        );
        let r = ShellModule
            .invoke(&ModuleArgs::new(args), "localhost", &mut ctx)
            .unwrap();
        assert!(r.status.is_ok());
    }

    #[test]
    fn test_shell_removes_runs_when_file_exists() {
        use tempfile::NamedTempFile;
        let tmp = NamedTempFile::new().unwrap();
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "_raw_params".to_string(),
            ansiblers_core::Value::String("echo did_run".to_string()),
        );
        args.insert(
            "removes".to_string(),
            ansiblers_core::Value::String(tmp.path().to_str().unwrap().to_string()),
        );
        let r = ShellModule
            .invoke(&ModuleArgs::new(args), "localhost", &mut ctx)
            .unwrap();
        assert!(r.status.is_ok());
        assert!(r.stdout.contains("did_run"));
    }

    #[test]
    fn test_shell_missing_command_errors() {
        let mut ctx = ctx();
        let r = ShellModule.invoke(&ModuleArgs::new(HashMap::new()), "localhost", &mut ctx);
        assert!(r.is_err());
    }

    #[test]
    fn test_shell_chdir() {
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "_raw_params".to_string(),
            ansiblers_core::Value::String("pwd".to_string()),
        );
        args.insert(
            "chdir".to_string(),
            ansiblers_core::Value::String(tmp.path().to_str().unwrap().to_string()),
        );
        let r = ShellModule
            .invoke(&ModuleArgs::new(args), "localhost", &mut ctx)
            .unwrap();
        assert!(r.status.is_ok());
    }

    #[test]
    fn test_shell_rc_captured() {
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "_raw_params".to_string(),
            ansiblers_core::Value::String("exit 5".to_string()),
        );
        let r = ShellModule
            .invoke(&ModuleArgs::new(args), "localhost", &mut ctx)
            .unwrap();
        assert_eq!(r.rc, 5);
    }
}
