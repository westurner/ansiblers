//! `pip` module — manage Python packages via `pip` / `pip3`.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Package name(s), optionally with version specs (`requests>=2.28`) |
//! | `state` | `present` | `present`, `absent`, `latest`, `info` |
//! | `version` | — | Exact version (appended as `==<version>`) |
//! | `requirements` | — | Path to a `requirements.txt` file |
//! | `virtualenv` | — | Path to a virtualenv to activate |
//! | `virtualenv_command` | `python3 -m venv` | Command to create virtualenv |
//! | `virtualenv_python` | — | Python interpreter for the new virtualenv |
//! | `extra_args` | — | Extra flags forwarded to pip |
//! | `editable` | `false` | Install in editable mode (`-e`) |
//! | `break_system_packages` | `false` | Pass `--break-system-packages` (PEP 668) |
//! | `executable` | — | Explicit path to pip executable |
//! | `chdir` | — | Change directory before running pip |
//! | `pip_bin` | `pip3` | Default pip binary when `executable` is not set |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct PipModule;

impl ModuleInvoker for PipModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        // Resolve the pip binary.
        let bin = resolve_bin(args);
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");
        let requirements = args.get_str("requirements").map(|s| s.to_string());
        let extra_args = args.get_str("extra_args").map(|s| s.to_string());
        let editable = bool_arg(args, "editable", false);
        let break_system = bool_arg(args, "break_system_packages", false);
        let chdir = args.get_str("chdir").map(|s| s.to_string());

        // Ensure virtualenv exists if requested.
        if let Some(venv) = args.get_str("virtualenv") {
            ensure_virtualenv(venv, args)?;
        }

        match state {
            "absent" => {
                let mut targets = packages.clone();
                if targets.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pip: 'name' is required for state=absent".to_string(),
                    ));
                }
                pip_uninstall(
                    &bin,
                    &targets,
                    break_system,
                    extra_args.as_deref(),
                    chdir.as_deref(),
                    host,
                )
            }
            "latest" => {
                let mut targets = packages.clone();
                if targets.is_empty() && requirements.is_none() {
                    return Ok(TaskResult::failed(
                        host,
                        "pip: 'name' or 'requirements' is required".to_string(),
                    ));
                }
                pip_install(
                    &bin,
                    &targets,
                    requirements.as_deref(),
                    true,
                    editable,
                    break_system,
                    extra_args.as_deref(),
                    chdir.as_deref(),
                    host,
                )
            }
            "info" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pip: 'name' is required for state=info".to_string(),
                    ));
                }
                pip_show(&bin, &packages, host)
            }
            _ => {
                if packages.is_empty() && requirements.is_none() {
                    return Ok(TaskResult::failed(
                        host,
                        "pip: 'name' or 'requirements' is required".to_string(),
                    ));
                }
                pip_install(
                    &bin,
                    &packages,
                    requirements.as_deref(),
                    false,
                    editable,
                    break_system,
                    extra_args.as_deref(),
                    chdir.as_deref(),
                    host,
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_bin(args: &ModuleArgs) -> String {
    if let Some(exe) = args.get_str("executable") {
        return exe.to_string();
    }
    // When a virtualenv is given, use its pip.
    if let Some(venv) = args.get_str("virtualenv") {
        return format!("{venv}/bin/pip");
    }
    args.get_str("pip_bin").unwrap_or("pip3").to_string()
}

fn ensure_virtualenv(venv_path: &str, args: &ModuleArgs) -> Result<()> {
    let venv_dir = std::path::Path::new(venv_path);
    if venv_dir.join("bin/pip").exists() || venv_dir.join("Scripts/pip.exe").exists() {
        return Ok(());
    }
    let create_cmd = args
        .get_str("virtualenv_command")
        .unwrap_or("python3 -m venv");
    let python = args.get_str("virtualenv_python").map(|s| s.to_string());

    let mut parts: Vec<&str> = create_cmd.split_whitespace().collect();
    let mut cmd = Command::new(parts.remove(0));
    cmd.args(&parts);
    if let Some(py) = &python {
        // Override interpreter: `python3 -m venv --python=<py>`
        cmd.arg(format!("--python={py}"));
    }
    cmd.arg(venv_path);
    let s = cmd.status().context("virtualenv creation")?;
    if !s.success() {
        anyhow::bail!("failed to create virtualenv at '{venv_path}'");
    }
    Ok(())
}

fn is_installed(bin: &str, pkg: &str) -> bool {
    // Strip version specifier for the check.
    let bare = pkg.split(['=', '<', '>', '!']).next().unwrap_or(pkg).trim();
    Command::new(bin)
        .args(["show", bare])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn versioned_name(pkg: &str, version: Option<&str>) -> String {
    if let Some(v) = version {
        if pkg.contains(['=', '<', '>', '!']) {
            pkg.to_string()
        } else {
            format!("{pkg}=={v}")
        }
    } else {
        pkg.to_string()
    }
}

fn pip_install(
    bin: &str,
    packages: &[String],
    requirements: Option<&str>,
    upgrade: bool,
    editable: bool,
    break_system: bool,
    extra: Option<&str>,
    chdir: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    // For idempotency with upgrade=false, skip already-installed packages.
    let targets: Vec<&String> = if upgrade {
        packages.iter().collect()
    } else {
        packages.iter().filter(|p| !is_installed(bin, p)).collect()
    };

    if targets.is_empty() && requirements.is_none() {
        return Ok(TaskResult::ok(host));
    }

    let mut cmd = Command::new(bin);
    cmd.arg("install");
    if upgrade {
        cmd.arg("--upgrade");
    }
    if editable {
        cmd.arg("-e");
    }
    if break_system {
        cmd.arg("--break-system-packages");
    }
    if let Some(r) = requirements {
        cmd.args(["-r", r]);
    }
    if let Some(e) = extra {
        cmd.args(e.split_whitespace());
    }
    for p in &targets {
        cmd.arg(p.as_str());
    }
    if let Some(d) = chdir {
        cmd.current_dir(d);
    }

    let out = cmd.output().context("pip install")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let changed = stdout.contains("Successfully installed") || stdout.contains("Collecting");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = format!(
            "installed: {}",
            targets
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pip install failed: {stderr}"),
        ))
    }
}

fn pip_uninstall(
    bin: &str,
    packages: &[String],
    break_system: bool,
    extra: Option<&str>,
    chdir: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    let installed: Vec<&String> = packages.iter().filter(|p| is_installed(bin, p)).collect();
    if installed.is_empty() {
        return Ok(TaskResult::ok(host));
    }

    let mut cmd = Command::new(bin);
    cmd.args(["uninstall", "-y"]);
    if break_system {
        cmd.arg("--break-system-packages");
    }
    if let Some(e) = extra {
        cmd.args(e.split_whitespace());
    }
    for p in &installed {
        cmd.arg(p.as_str());
    }
    if let Some(d) = chdir {
        cmd.current_dir(d);
    }

    let out = cmd.output().context("pip uninstall")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = format!(
            "uninstalled: {}",
            installed
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pip uninstall failed: {stderr}"),
        ))
    }
}

fn pip_show(bin: &str, packages: &[String], host: &str) -> Result<TaskResult> {
    let mut out_all = String::new();
    for pkg in packages {
        let bare = pkg.split(['=', '<', '>', '!']).next().unwrap_or(pkg).trim();
        if let Ok(out) = Command::new(bin).args(["show", bare]).output() {
            out_all.push_str(&String::from_utf8_lossy(&out.stdout));
        }
    }
    let mut r = TaskResult::ok(host);
    r.stdout = out_all.clone();
    r.vars.insert("info".into(), Value::String(out_all));
    Ok(r)
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
        .map(|p| versioned_name(&p, version))
        .collect()
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
    fn test_no_name_or_requirements_present_fails() {
        let mut c = ctx();
        let r = PipModule
            .invoke(&ModuleArgs::new(HashMap::new()), "h", &mut c)
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails() {
        let mut c = ctx();
        let r = PipModule
            .invoke(
                &args(&[("state", Value::String("absent".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_versioned_name_appends_pin() {
        assert_eq!(
            versioned_name("requests", Some("2.28.0")),
            "requests==2.28.0"
        );
    }
    #[test]
    fn test_versioned_name_no_version() {
        assert_eq!(versioned_name("requests", None), "requests");
    }
    #[test]
    fn test_versioned_name_already_specifier() {
        assert_eq!(
            versioned_name("requests>=2.0", Some("2.28.0")),
            "requests>=2.0"
        );
    }
    #[test]
    fn test_resolve_bin_default() {
        let a = ModuleArgs::new(HashMap::new());
        assert_eq!(resolve_bin(&a), "pip3");
    }
    #[test]
    fn test_resolve_bin_executable() {
        let a = args(&[("executable", Value::String("/venv/bin/pip".into()))]);
        assert_eq!(resolve_bin(&a), "/venv/bin/pip");
    }
    #[test]
    fn test_resolve_bin_virtualenv() {
        let a = args(&[("virtualenv", Value::String("/opt/venv".into()))]);
        assert_eq!(resolve_bin(&a), "/opt/venv/bin/pip");
    }
    #[test]
    fn test_collect_packages_with_version() {
        let a = args(&[
            ("name", Value::String("requests".into())),
            ("version", Value::String("2.28.0".into())),
        ]);
        assert_eq!(collect_packages(&a), vec!["requests==2.28.0"]);
    }
    #[test]
    fn test_break_system_default_false() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "break_system_packages", false));
    }
}
