//! `uv` module — manage Python packages and projects via [`uv`](https://docs.astral.sh/uv/).
//!
//! `uv` is a fast Python package and project manager written in Rust.  It can
//! replace `pip`, `pip-tools`, `pipx`, `poetry`, `pyenv`, and `virtualenv`.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Package name(s), optionally with version specifiers |
//! | `state` | `present` | `present` (add/install), `absent` (remove), `latest` (upgrade), `info`, `sync`, `lock`, `run`, `tool_install`, `tool_uninstall`, `venv`, `python_install`, `python_list` |
//! | `version` | — | Exact version pin appended as `==<version>` |
//! | `requirements` | — | Path to a requirements file (`-r`) |
//! | `project` | — | Path to the project directory (sets `--directory`) |
//! | `virtualenv` / `venv` | — | Virtualenv path for `--python`/`--venv` |
//! | `python` | — | Python version or interpreter for `--python` |
//! | `extras` | — | List of extras to enable on install |
//! | `dev` | `false` | Install into the dev dependency group |
//! | `group` | — | Dependency group name (e.g. `dev`, `docs`) |
//! | `frozen` | `false` | `--frozen` — do not update the lockfile |
//! | `locked` | `false` | `--locked` — assert lockfile is up-to-date |
//! | `no_sync` | `false` | `--no-sync` — skip environment sync after changes |
//! | `system` | `false` | `--system` — install into the system Python |
//! | `break_system_packages` | `false` | `--break-system-packages` (PEP 668) |
//! | `index` | — | Extra index URL(s) |
//! | `index_strategy` | — | `first-index`, `unsafe-first-match`, `unsafe-best-match` |
//! | `command` | — | Shell command for `state=run` |
//! | `tool` | — | Tool name for `tool_install` / `tool_uninstall` |
//! | `uv_bin` | `uv` | Path to the uv binary |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct UvModule;

impl ModuleInvoker for UvModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let bin = args.get_str("uv_bin").unwrap_or("uv").to_string();
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");

        match state {
            // Project-level operations
            "sync" => uv_sync(&bin, args, host),
            "lock" => uv_lock(&bin, args, host),
            "run" => {
                let cmd = args
                    .get_str("command")
                    .ok_or_else(|| anyhow::anyhow!("uv: 'command' is required for state=run"))?;
                uv_run(&bin, cmd, args, host)
            }
            "venv" => uv_venv(&bin, args, host),
            "python_install" => {
                let py = args.get_str("python").ok_or_else(|| {
                    anyhow::anyhow!("uv: 'python' is required for state=python_install")
                })?;
                uv_python_install(&bin, py, host)
            }
            "python_list" => uv_python_list(&bin, host),
            // Tool operations
            "tool_install" => {
                let tool = args
                    .get_str("tool")
                    .or_else(|| args.get_str("name"))
                    .ok_or_else(|| {
                        anyhow::anyhow!("uv: 'tool' is required for state=tool_install")
                    })?;
                uv_tool_install(&bin, tool, args, host)
            }
            "tool_uninstall" => {
                let tool = args
                    .get_str("tool")
                    .or_else(|| args.get_str("name"))
                    .ok_or_else(|| {
                        anyhow::anyhow!("uv: 'tool' is required for state=tool_uninstall")
                    })?;
                uv_tool_uninstall(&bin, tool, host)
            }
            // Package operations
            "absent" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "uv: 'name' is required for state=absent".to_string(),
                    ));
                }
                uv_remove(&bin, &packages, args, host)
            }
            "latest" => {
                if packages.is_empty() && args.get_str("requirements").is_none() {
                    return Ok(TaskResult::failed(
                        host,
                        "uv: 'name' or 'requirements' required".to_string(),
                    ));
                }
                uv_add_or_pip(&bin, &packages, args, true, host)
            }
            "info" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "uv: 'name' is required for state=info".to_string(),
                    ));
                }
                uv_pip_show(&bin, &packages, args, host)
            }
            _ => {
                // present
                if packages.is_empty() && args.get_str("requirements").is_none() {
                    return Ok(TaskResult::failed(
                        host,
                        "uv: 'name' or 'requirements' required".to_string(),
                    ));
                }
                uv_add_or_pip(&bin, &packages, args, false, host)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn global_args(args: &ModuleArgs) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(d) = args.get_str("project") {
        v.push("--directory".to_string());
        v.push(d.to_string());
    }
    v
}

fn python_args(args: &ModuleArgs) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(py) = args.get_str("python") {
        v.push("--python".to_string());
        v.push(py.to_string());
    }
    v
}

fn index_args(args: &ModuleArgs) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(idx) = args.get_str("index") {
        v.push("--index".to_string());
        v.push(idx.to_string());
    }
    if let Some(strat) = args.get_str("index_strategy") {
        v.push("--index-strategy".to_string());
        v.push(strat.to_string());
    }
    v
}

/// For projects (pixi.toml/pyproject): `uv add`; standalone: `uv pip install`.
fn uv_add_or_pip(
    bin: &str,
    packages: &[String],
    args: &ModuleArgs,
    upgrade: bool,
    host: &str,
) -> Result<TaskResult> {
    let is_project = args.get_str("project").is_some();
    let requirements = args.get_str("requirements").map(|s| s.to_string());

    if is_project && requirements.is_none() {
        // `uv add`
        let mut cmd = Command::new(bin);
        cmd.args(global_args(args));
        cmd.arg("add");
        if upgrade {
            cmd.arg("--upgrade");
        }
        if bool_arg(args, "dev", false) {
            cmd.arg("--dev");
        }
        if let Some(g) = args.get_str("group") {
            cmd.args(["--group", g]);
        }
        if bool_arg(args, "frozen", false) {
            cmd.arg("--frozen");
        }
        if bool_arg(args, "no_sync", false) {
            cmd.arg("--no-sync");
        }
        cmd.args(python_args(args));
        cmd.args(index_args(args));
        if let Some(extras) = collect_list(args, "extras") {
            for e in extras {
                cmd.args(["--extra", &e]);
            }
        }
        for p in packages {
            cmd.arg(p);
        }
        return run_change(cmd, host, &format!("added: {}", packages.join(", ")));
    }

    // `uv pip install`
    let mut cmd = Command::new(bin);
    cmd.arg("pip");
    cmd.arg("install");
    if upgrade {
        cmd.arg("--upgrade");
    }
    if bool_arg(args, "system", false) {
        cmd.arg("--system");
    }
    if bool_arg(args, "break_system_packages", false) {
        cmd.arg("--break-system-packages");
    }
    if let Some(venv) = args.get_str("virtualenv").or_else(|| args.get_str("venv")) {
        cmd.args(["--python", &format!("{venv}/bin/python")]);
    }
    cmd.args(python_args(args));
    cmd.args(index_args(args));
    if let Some(r) = requirements.as_deref() {
        cmd.args(["-r", r]);
    }
    if let Some(e) = args.get_str("extra_args") {
        cmd.args(e.split_whitespace());
    }
    for p in packages {
        cmd.arg(p);
    }

    let out = cmd.output().context("uv pip install")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let changed = stdout.contains("Installed") || stdout.contains("Resolved");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = format!("installed: {}", packages.join(", "));
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("uv pip install failed: {stderr}"),
        ))
    }
}

fn uv_remove(bin: &str, packages: &[String], args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let is_project = args.get_str("project").is_some();
    let mut cmd = Command::new(bin);
    cmd.args(global_args(args));

    if is_project {
        cmd.arg("remove");
        if bool_arg(args, "dev", false) {
            cmd.arg("--dev");
        }
        if let Some(g) = args.get_str("group") {
            cmd.args(["--group", g]);
        }
        if bool_arg(args, "frozen", false) {
            cmd.arg("--frozen");
        }
    } else {
        cmd.args(["pip", "uninstall", "-y"]);
        if bool_arg(args, "system", false) {
            cmd.arg("--system");
        }
    }
    for p in packages {
        cmd.arg(p);
    }
    run_change(cmd, host, &format!("removed: {}", packages.join(", ")))
}

fn uv_sync(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(global_args(args));
    cmd.arg("sync");
    if bool_arg(args, "frozen", false) {
        cmd.arg("--frozen");
    }
    if bool_arg(args, "locked", false) {
        cmd.arg("--locked");
    }
    if bool_arg(args, "dev", false) {
        cmd.arg("--dev");
    } else {
        cmd.arg("--no-dev");
    }
    if let Some(g) = args.get_str("group") {
        cmd.args(["--group", g]);
    }
    cmd.args(python_args(args));
    run_change(cmd, host, "synced")
}

fn uv_lock(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(global_args(args));
    cmd.arg("lock");
    if bool_arg(args, "frozen", false) {
        cmd.arg("--frozen");
    }
    cmd.args(index_args(args));
    run_change(cmd, host, "lockfile updated")
}

fn uv_run(bin: &str, command: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(global_args(args));
    cmd.arg("run");
    if bool_arg(args, "frozen", false) {
        cmd.arg("--frozen");
    }
    if bool_arg(args, "locked", false) {
        cmd.arg("--locked");
    }
    cmd.args(python_args(args));
    if let Some(e) = args.get_str("extra_args") {
        cmd.args(e.split_whitespace());
    }
    cmd.args(command.split_whitespace());
    let out = cmd.output().context("uv run")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let rc = out.status.code().unwrap_or(-1);
    if out.status.success() {
        let mut r = TaskResult::ok(host);
        r.stdout = stdout;
        r.msg = format!("ran: {command}");
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("uv run '{command}' failed (rc={rc}): {stderr}"),
        ))
    }
}

fn uv_venv(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let path = args
        .get_str("virtualenv")
        .or_else(|| args.get_str("venv"))
        .unwrap_or(".venv");
    let mut cmd = Command::new(bin);
    cmd.arg("venv");
    cmd.args(python_args(args));
    cmd.arg(path);
    run_change(cmd, host, &format!("venv created at '{path}'"))
}

fn uv_python_install(bin: &str, version: &str, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(["python", "install", version]);
    run_change(cmd, host, &format!("python {version} installed"))
}

fn uv_python_list(bin: &str, host: &str) -> Result<TaskResult> {
    let out = Command::new(bin)
        .args(["python", "list"])
        .output()
        .context("uv python list")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let versions: Vec<Value> = stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| Value::String(l.to_string()))
        .collect();
    let mut r = TaskResult::ok(host);
    r.stdout = stdout;
    r.vars
        .insert("python_versions".into(), Value::Array(versions));
    Ok(r)
}

fn uv_tool_install(bin: &str, tool: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(["tool", "install"]);
    if bool_arg(args, "upgrade", false) {
        cmd.arg("--upgrade");
    }
    cmd.args(python_args(args));
    cmd.args(index_args(args));
    cmd.arg(tool);
    run_change(cmd, host, &format!("tool '{tool}' installed"))
}

fn uv_tool_uninstall(bin: &str, tool: &str, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(["tool", "uninstall", tool]);
    run_change(cmd, host, &format!("tool '{tool}' uninstalled"))
}

fn uv_pip_show(
    bin: &str,
    packages: &[String],
    args: &ModuleArgs,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(["pip", "show"]);
    if bool_arg(args, "system", false) {
        cmd.arg("--system");
    }
    for p in packages {
        cmd.arg(p);
    }
    let out = cmd.output().context("uv pip show")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut r = TaskResult::ok(host);
    r.stdout = stdout.clone();
    r.vars.insert("info".into(), Value::String(stdout));
    Ok(r)
}

fn run_change(mut cmd: Command, host: &str, msg: &str) -> Result<TaskResult> {
    let out = cmd.output().context("uv command")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let rc = out.status.code().unwrap_or(-1);
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        r.msg = msg.to_string();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("uv failed (rc={rc}): {stderr}"),
        ))
    }
}

fn collect_packages(args: &ModuleArgs) -> Vec<String> {
    let version = args.get_str("version");
    let val = args.args.get("name").or_else(|| args.args.get("pkg"));
    let pkgs: Vec<String> = match val {
        None => vec![],
        Some(Value::String(s)) => s.split_whitespace().map(|s| s.to_string()).collect(),
        Some(Value::Array(seq)) => seq
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => vec![],
    };
    pkgs.into_iter()
        .map(|p| {
            if let Some(v) = version {
                if p.contains(['=', '<', '>', '!']) {
                    p
                } else {
                    format!("{p}=={v}")
                }
            } else {
                p
            }
        })
        .collect()
}

fn collect_list(args: &ModuleArgs, key: &str) -> Option<Vec<String>> {
    match args.args.get(key)? {
        Value::String(s) => Some(s.split_whitespace().map(|s| s.to_string()).collect()),
        Value::Array(seq) => Some(
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
        ),
        _ => None,
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
    use ansiblers_core::{ExecutionContext, Inventory, Value};
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
    fn test_no_name_present_fails() {
        let mut c = ctx();
        let r = UvModule
            .invoke(&ModuleArgs::new(HashMap::new()), "h", &mut c)
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails() {
        let mut c = ctx();
        let r = UvModule
            .invoke(
                &args(&[("state", Value::String("absent".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_run_missing_command_errors() {
        let mut c = ctx();
        let r = UvModule.invoke(
            &args(&[("state", Value::String("run".into()))]),
            "h",
            &mut c,
        );
        assert!(r.is_err());
    }
    #[test]
    fn test_tool_install_missing_tool_errors() {
        let mut c = ctx();
        let r = UvModule.invoke(
            &args(&[("state", Value::String("tool_install".into()))]),
            "h",
            &mut c,
        );
        assert!(r.is_err());
    }
    #[test]
    fn test_python_install_missing_version_errors() {
        let mut c = ctx();
        let r = UvModule.invoke(
            &args(&[("state", Value::String("python_install".into()))]),
            "h",
            &mut c,
        );
        assert!(r.is_err());
    }
    #[test]
    fn test_global_args_project() {
        let a = args(&[("project", Value::String("/srv/myproject".into()))]);
        assert_eq!(global_args(&a), vec!["--directory", "/srv/myproject"]);
    }
    #[test]
    fn test_global_args_empty() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(global_args(&a).is_empty());
    }
    #[test]
    fn test_python_args_version() {
        let a = args(&[("python", Value::String("3.12".into()))]);
        assert_eq!(python_args(&a), vec!["--python", "3.12"]);
    }
    #[test]
    fn test_index_args() {
        let a = args(&[("index", Value::String("https://pypi.org/simple".into()))]);
        let v = index_args(&a);
        assert!(v.contains(&"--index".to_string()));
    }
    #[test]
    fn test_collect_packages_with_version() {
        let a = args(&[
            ("name", Value::String("fastapi".into())),
            ("version", Value::String("0.110.0".into())),
        ]);
        assert_eq!(collect_packages(&a), vec!["fastapi==0.110.0"]);
    }
    #[test]
    fn test_collect_packages_specifier_preserved() {
        let a = args(&[("name", Value::String("fastapi>=0.100".into()))]);
        assert_eq!(collect_packages(&a), vec!["fastapi>=0.100"]);
    }
    #[test]
    fn test_frozen_default_false() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "frozen", false));
    }
}
