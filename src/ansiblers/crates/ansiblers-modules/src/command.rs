//! `command` module — executes commands directly (no shell interpolation).

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult};
use anyhow::Result;

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
    use tempfile::TempDir;

    fn ctx() -> ExecutionContext {
        ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new())
    }

    fn args(pairs: &[(&str, &str)]) -> ModuleArgs {
        let mut m = HashMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), Value::String(v.to_string()));
        }
        ModuleArgs::new(m)
    }

    #[test]
    fn test_command_true() {
        let mut ctx = ctx();
        let r = CommandModule
            .invoke(
                &args(&[("_raw_params", "/bin/true")]),
                "localhost",
                &mut ctx,
            )
            .unwrap();
        assert!(r.status.is_ok());
        assert_eq!(r.rc, 0);
    }

    #[test]
    fn test_command_echo() {
        let mut ctx = ctx();
        let r = CommandModule
            .invoke(
                &args(&[("_raw_params", "echo hello")]),
                "localhost",
                &mut ctx,
            )
            .unwrap();
        assert!(r.status.is_ok());
        assert!(r.stdout.contains("hello"));
    }

    #[test]
    fn test_command_failure() {
        let mut ctx = ctx();
        let r = CommandModule
            .invoke(
                &args(&[("_raw_params", "/bin/false")]),
                "localhost",
                &mut ctx,
            )
            .unwrap();
        assert!(r.status.is_failed());
        assert_ne!(r.rc, 0);
    }

    #[test]
    fn test_command_missing_raw_params_fails() {
        let mut ctx = ctx();
        let r = CommandModule.invoke(&ModuleArgs::new(HashMap::new()), "localhost", &mut ctx);
        assert!(r.is_err());
    }

    #[test]
    fn test_command_creates_skip() {
        let tmp = TempDir::new().unwrap();
        let existing = tmp.path().join("exists.txt");
        std::fs::write(&existing, "x").unwrap();
        let mut m = HashMap::new();
        m.insert(
            "_raw_params".into(),
            Value::String("echo should_not_run".into()),
        );
        m.insert(
            "creates".into(),
            Value::String(existing.to_str().unwrap().to_string()),
        );
        let mut ctx = ctx();
        let r = CommandModule
            .invoke(&ModuleArgs::new(m), "localhost", &mut ctx)
            .unwrap();
        // Skipped because creates file exists → ok, not changed
        assert!(r.status.is_ok());
    }

    #[test]
    fn test_command_removes_skip() {
        let mut m = HashMap::new();
        m.insert(
            "_raw_params".into(),
            Value::String("echo should_not_run".into()),
        );
        m.insert(
            "removes".into(),
            Value::String("/nonexistent/path/xyz".into()),
        );
        let mut ctx = ctx();
        let r = CommandModule
            .invoke(&ModuleArgs::new(m), "localhost", &mut ctx)
            .unwrap();
        assert!(r.status.is_ok());
    }

    #[test]
    fn test_command_chdir() {
        let tmp = TempDir::new().unwrap();
        let mut m = HashMap::new();
        m.insert("_raw_params".into(), Value::String("pwd".into()));
        m.insert(
            "chdir".into(),
            Value::String(tmp.path().to_str().unwrap().to_string()),
        );
        let mut ctx = ctx();
        let r = CommandModule
            .invoke(&ModuleArgs::new(m), "localhost", &mut ctx)
            .unwrap();
        assert!(r.status.is_ok());
        assert!(r
            .stdout
            .contains(tmp.path().to_str().unwrap().trim_end_matches('/')));
    }

    #[test]
    fn test_command_rc_captured() {
        let mut ctx = ctx();
        // `false` always exits 1 without a shell.
        let r = CommandModule
            .invoke(
                &args(&[("_raw_params", "/bin/false")]),
                "localhost",
                &mut ctx,
            )
            .unwrap();
        assert_ne!(r.rc, 0);
        assert!(r.status.is_failed());
    }
}
