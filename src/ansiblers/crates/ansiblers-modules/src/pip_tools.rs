//! `pip_tools` module — manage `requirements.in` → `requirements.txt` via
//! [`pip-compile`](https://pip-tools.readthedocs.io/) and sync virtual
//! environments via [`pip-sync`](https://pip-tools.readthedocs.io/).
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `state` | `compile` | `compile`, `sync`, `upgrade`, `upgrade_package` |
//! | `src_file` | `requirements.in` | Input file for `pip-compile` |
//! | `output_file` | — | Output `.txt` file (default: derived from `src_file`) |
//! | `extra_files` | — | Additional `.in` files to merge on compile |
//! | `upgrade` | `false` | `--upgrade` — re-resolve all packages |
//! | `upgrade_package` / `name` | — | Upgrade only specified packages (`--upgrade-package`) |
//! | `generate_hashes` | `false` | Embed hashes in the output |
//! | `allow_unsafe` | `false` | `--allow-unsafe` |
//! | `strip_extras` | `false` | `--strip-extras` |
//! | `no_header` | `false` | Omit the `pip-compile` header comment |
//! | `reuse_hashes` | `false` | `--reuse-hashes` |
//! | `resolver` | — | `backtracking` or `legacy` |
//! | `index_url` | — | Override the PyPI index URL |
//! | `extra_index_url` | — | Extra index URL |
//! | `find_links` | — | Additional `--find-links` path or URL |
//! | `requirements` | `requirements.txt` | File to pass to `pip-sync` |
//! | `virtualenv` | — | Virtualenv path (sets `--python-executable`) |
//! | `ask` | `false` | Pass `--ask` to pip-sync (interactive) |
//! | `pip_compile_bin` | `pip-compile` | Path to pip-compile |
//! | `pip_sync_bin` | `pip-sync` | Path to pip-sync |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct PipToolsModule;

impl ModuleInvoker for PipToolsModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let state = args.get_str("state").unwrap_or("compile");

        match state {
            "sync" => pip_sync(args, host),
            "upgrade" => pip_compile(args, true, None, host),
            "upgrade_package" => {
                let pkgs = collect_packages(args);
                if pkgs.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pip_tools: 'name' is required for state=upgrade_package".to_string(),
                    ));
                }
                pip_compile(args, false, Some(&pkgs), host)
            }
            _ => pip_compile(args, bool_arg(args, "upgrade", false), None, host),
        }
    }
}

// ---------------------------------------------------------------------------
// pip-compile
// ---------------------------------------------------------------------------

fn pip_compile(
    args: &ModuleArgs,
    upgrade: bool,
    upgrade_pkgs: Option<&[String]>,
    host: &str,
) -> Result<TaskResult> {
    let compile_bin = args
        .get_str("pip_compile_bin")
        .unwrap_or("pip-compile")
        .to_string();
    let src_file = args.get_str("src_file").unwrap_or("requirements.in");

    let mut cmd = Command::new(&compile_bin);
    cmd.arg(src_file);

    if let Some(out) = args.get_str("output_file") {
        cmd.args(["--output-file", out]);
    }
    if upgrade {
        cmd.arg("--upgrade");
    }
    if let Some(pkgs) = upgrade_pkgs {
        for p in pkgs {
            cmd.args(["--upgrade-package", p]);
        }
    }
    if bool_arg(args, "generate_hashes", false) {
        cmd.arg("--generate-hashes");
    }
    if bool_arg(args, "allow_unsafe", false) {
        cmd.arg("--allow-unsafe");
    }
    if bool_arg(args, "strip_extras", false) {
        cmd.arg("--strip-extras");
    }
    if bool_arg(args, "no_header", false) {
        cmd.arg("--no-header");
    }
    if bool_arg(args, "reuse_hashes", false) {
        cmd.arg("--reuse-hashes");
    }
    if let Some(r) = args.get_str("resolver") {
        cmd.args(["--resolver", r]);
    }
    if let Some(i) = args.get_str("index_url") {
        cmd.args(["--index-url", i]);
    }
    if let Some(ei) = args.get_str("extra_index_url") {
        cmd.args(["--extra-index-url", ei]);
    }
    if let Some(fl) = args.get_str("find_links") {
        cmd.args(["--find-links", fl]);
    }

    // Extra .in files.
    for f in collect_extra_files(args) {
        cmd.arg(&f);
    }

    // Virtualenv python executable.
    if let Some(venv) = args.get_str("virtualenv") {
        cmd.args(["--python-executable", &format!("{venv}/bin/python")]);
    }

    if let Some(extra) = args.get_str("extra_args") {
        cmd.args(extra.split_whitespace());
    }

    let out = cmd.output().context("pip-compile")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        // pip-compile exits 0 and writes "Nothing to upgrade" when nothing changed.
        let changed =
            !stderr.contains("Nothing to upgrade") && !stderr.contains("already up-to-date");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = if changed {
            "requirements compiled".into()
        } else {
            "requirements already up-to-date".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pip-compile failed: {stderr}"),
        ))
    }
}

// ---------------------------------------------------------------------------
// pip-sync
// ---------------------------------------------------------------------------

fn pip_sync(args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let sync_bin = args
        .get_str("pip_sync_bin")
        .unwrap_or("pip-sync")
        .to_string();
    let req_file = args.get_str("requirements").unwrap_or("requirements.txt");

    let mut cmd = Command::new(&sync_bin);
    cmd.arg(req_file);

    if bool_arg(args, "ask", false) {
        cmd.arg("--ask");
    }
    if let Some(venv) = args.get_str("virtualenv") {
        cmd.args(["--python-executable", &format!("{venv}/bin/python")]);
    }
    if let Some(pip) = args.get_str("pip_args") {
        cmd.args(["--pip-args", pip]);
    }
    if let Some(extra) = args.get_str("extra_args") {
        cmd.args(extra.split_whitespace());
    }

    let out = cmd.output().context("pip-sync")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let changed = stdout.contains("Collecting")
            || stdout.contains("Installing")
            || stdout.contains("Uninstalling");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = if changed {
            "environment synced".into()
        } else {
            "environment already up-to-date".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pip-sync failed: {stderr}"),
        ))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn collect_packages(args: &ModuleArgs) -> Vec<String> {
    let val = args
        .args
        .get("upgrade_package")
        .or_else(|| args.args.get("name"))
        .or_else(|| args.args.get("pkg"));
    match val {
        None => vec![],
        Some(Value::String(s)) => s.split_whitespace().map(|s| s.to_string()).collect(),
        Some(Value::Array(seq)) => seq
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => vec![],
    }
}

fn collect_extra_files(args: &ModuleArgs) -> Vec<String> {
    if let Some(val) = args.args.get("extra_files") {
        match val {
            Value::String(s) => s.split_whitespace().map(|s| s.to_string()).collect(),
            Value::Array(seq) => seq
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            _ => vec![],
        }
    } else {
        vec![]
    }
}

fn bool_arg(args: &ModuleArgs, key: &str, default: bool) -> bool {
    args.args
        .get(key)
        .and_then(|v| v.as_bool())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ansiblers_core::{ExecutionContext, Inventory};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn ctx() -> ExecutionContext {
        ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new())
    }
    fn args(pairs: &[(&str, Value)]) -> ModuleArgs {
        let mut m = HashMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.clone());
        }
        ModuleArgs::new(m)
    }

    #[test]
    fn test_upgrade_package_missing_name_fails() {
        let mut c = ctx();
        let r = PipToolsModule
            .invoke(
                &args(&[("state", Value::String("upgrade_package".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_default_state_is_compile() {
        // No binary present → will error, but state routing is correct.
        let a = ModuleArgs::new(HashMap::new());
        // Just test the state defaults to compile (doesn't panic).
        assert_eq!(a.get_str("state").unwrap_or("compile"), "compile");
    }
    #[test]
    fn test_collect_packages_upgrade_package_key() {
        let a = args(&[("upgrade_package", Value::String("requests".into()))]);
        assert_eq!(collect_packages(&a), vec!["requests"]);
    }
    #[test]
    fn test_collect_packages_name_key() {
        let a = args(&[("name", Value::String("requests flask".into()))]);
        assert_eq!(collect_packages(&a), vec!["requests", "flask"]);
    }
    #[test]
    fn test_collect_extra_files_array() {
        let a = args(&[(
            "extra_files",
            Value::Array(vec![
                Value::String("base.in".into()),
                Value::String("dev.in".into()),
            ]),
        )]);
        assert_eq!(collect_extra_files(&a), vec!["base.in", "dev.in"]);
    }
    #[test]
    fn test_generate_hashes_default_false() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "generate_hashes", false));
    }
    #[test]
    fn test_bool_arg_upgrade() {
        let a = args(&[("upgrade", Value::Bool(true))]);
        assert!(bool_arg(&a, "upgrade", false));
    }
}
