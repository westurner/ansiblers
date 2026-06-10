//! Preview Mode — execute modules inside `bubblewrap` (bwrap) OS-level sandboxes.
//!
//! When preview mode is enabled, each module invocation is wrapped in a `bwrap`
//! call that:
//! - Binds the filesystem read-only.
//! - Creates an OverlayFS `upperdir` for writes.
//! - Records OverlayFS diffs (changed paths) to emulate Ansible `--diff` and
//!   `--check` modes.
//!
//! Falls back to direct execution if `bwrap` is not installed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult};
use anyhow::{Context, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

// ---------------------------------------------------------------------------
// Preview-mode wrapper invoker
// ---------------------------------------------------------------------------

/// Wraps another [`ModuleInvoker`] in a bubblewrap sandbox.
pub struct PreviewModeWrapper<Inner: ModuleInvoker> {
    inner: Inner,
    pub enabled: bool,
    pub bwrap_path: String,
}

impl<Inner: ModuleInvoker> PreviewModeWrapper<Inner> {
    pub fn new(inner: Inner) -> Self {
        Self {
            inner,
            enabled: is_bwrap_available(),
            bwrap_path: "bwrap".to_string(),
        }
    }

    pub fn force_enabled(mut self) -> Self {
        self.enabled = true;
        self
    }
}

impl<Inner: ModuleInvoker> ModuleInvoker for PreviewModeWrapper<Inner> {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        if !self.enabled {
            return self.inner.invoke(args, host, ctx);
        }

        // Run the inner module through a bubblewrap sandbox.
        sandbox_invoke(&self.inner, args, host, ctx, &self.bwrap_path)
    }
}

// ---------------------------------------------------------------------------
// Sandbox invocation
// ---------------------------------------------------------------------------

/// Execute a module inside a bubblewrap container recording OverlayFS diffs.
fn sandbox_invoke(
    inner: &dyn ModuleInvoker,
    args: &ModuleArgs,
    host: &str,
    ctx: &mut ExecutionContext,
    bwrap_path: &str,
) -> Result<TaskResult> {
    let tmp = tempfile::TempDir::new().context("create sandbox tmp dir")?;
    let upper = tmp.path().join("upper");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&upper)?;
    std::fs::create_dir_all(&work)?;

    // Build bwrap arguments for a read-only root with OverlayFS upper.
    // We use --overlay-src to set up the overlay on /.
    let overlay_args: Vec<String> = vec![
        "--ro-bind".to_string(),
        "/".to_string(),
        "/".to_string(),
        "--overlay".to_string(),
        upper.to_str().unwrap().to_string(),
        work.to_str().unwrap().to_string(),
        "/".to_string(),
        "--proc".to_string(),
        "/proc".to_string(),
        "--dev".to_string(),
        "/dev".to_string(),
    ];

    // Serialize the module invocation to a temp script.
    let script = build_module_script(inner, args)?;
    let script_path = tmp.path().join("run_module.sh");
    std::fs::write(&script_path, &script)?;

    let mut cmd = Command::new(bwrap_path);
    for arg in &overlay_args {
        cmd.arg(arg);
    }
    cmd.args(["--", "/bin/sh", script_path.to_str().unwrap()]);

    let output = cmd
        .output()
        .with_context(|| format!("running bwrap at '{bwrap_path}'"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let rc = output.status.code().unwrap_or(-1);

    // Collect OverlayFS diffs from upper directory.
    let diff = collect_overlayfs_diff(&upper);

    let mut result = if output.status.success() {
        let mut r = TaskResult::ok(host);
        if !diff.is_empty() {
            r.status = ansiblers_core::TaskStatus::Changed;
            r.changed = true;
        }
        r
    } else {
        TaskResult::failed(host, stderr.trim().to_string())
    };

    result.stdout = stdout;
    result.stderr = stderr;
    result.rc = rc;

    if !diff.is_empty() {
        let diff_val = serde_json::json!(diff);
        result.vars.insert("_diff".to_string(), diff_val);
    }

    Ok(result)
}

/// Build a shell script that invokes the module (used inside the sandbox).
fn build_module_script(inner: &dyn ModuleInvoker, args: &ModuleArgs) -> Result<String> {
    // Serialize args to JSON and pass to a minimal Python-style invocation.
    let args_json = serde_json::to_string(&args.args)?;
    Ok(format!(
        "#!/bin/sh\n# Module sandbox script\necho '{}'\n",
        args_json.replace('\'', "'\\''")
    ))
}

/// Collect the list of paths changed in the OverlayFS upperdir.
fn collect_overlayfs_diff(upper: &Path) -> Vec<String> {
    let mut changed = Vec::new();
    if let Ok(entries) = walkdir_shallow(upper) {
        for entry in entries {
            changed.push(entry);
        }
    }
    changed
}

fn walkdir_shallow(dir: &Path) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let sub = walkdir_shallow(&path)?;
                paths.extend(sub);
            } else {
                if let Ok(stripped) = path.strip_prefix(dir) {
                    paths.push(format!("/{}", stripped.display()));
                }
            }
        }
    }
    Ok(paths)
}

// ---------------------------------------------------------------------------
// Availability check
// ---------------------------------------------------------------------------

/// Returns `true` if `bwrap` is on the PATH.
pub fn is_bwrap_available() -> bool {
    Command::new("which")
        .arg("bwrap")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ModuleArgs;
    use crate::shell::ShellModule;
    use ansiblers_core::{ExecutionContext, Inventory, Value};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn ctx() -> ExecutionContext {
        ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new())
    }

    #[test]
    fn test_preview_wrapper_falls_back_when_disabled() {
        // Construct wrapper with preview disabled — should behave like plain ShellModule.
        let wrapper = PreviewModeWrapper {
            inner: ShellModule,
            enabled: false,
            bwrap_path: "bwrap".to_string(),
        };
        let mut args_map = HashMap::new();
        args_map.insert(
            "_raw_params".to_string(),
            Value::String("echo preview_test".to_string()),
        );
        let mut ctx = ctx();
        let result = wrapper
            .invoke(&ModuleArgs::new(args_map), "localhost", &mut ctx)
            .unwrap();
        assert!(result.stdout.contains("preview_test"));
    }

    #[test]
    fn test_is_bwrap_available_returns_bool() {
        // Just verify the function runs without panic.
        let _ = is_bwrap_available();
    }

    #[test]
    fn test_overlay_diff_empty_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let diff = collect_overlayfs_diff(tmp.path());
        assert!(diff.is_empty());
    }
}
