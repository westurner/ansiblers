//! `pkgsrc` module — manage packages via `pkgin` (binary) or `pkg_add` on
//! NetBSD pkgsrc and compatible systems (MINIX 3, illumos, macOS via pkgsrc).
//!
//! The module prefers `pkgin` (the binary package manager front-end) when
//! available; falls back to the lower-level `pkg_add`/`pkg_delete` pkgsrc
//! tools controlled by the `tool` parameter.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Package name(s) |
//! | `state` | `present` | `present`, `absent`, `latest`, `info`, `upgrade_all`, `clean` |
//! | `tool` | auto | `pkgin` or `pkg_add` |
//! | `update_cache` | `false` | Run `pkgin update` / `pkg_admin fetch-pkg-vulnerabilities` |
//! | `full` | `false` | `pkgin full-upgrade` instead of `pkgin upgrade` |
//! | `yes` | `true` | Pass `-y` to pkgin (non-interactive) |
//! | `pkg_path` | — | Override `PKG_PATH` env var (for `pkg_add` tool) |
//! | `pkgin_bin` | `pkgin` | Path to pkgin |
//! | `pkg_add_bin` | `pkg_add` | Path to `pkg_add` (pkgsrc) |
//! | `pkg_delete_bin` | `pkg_delete` | Path to `pkg_delete` (pkgsrc) |
//! | `pkg_info_bin` | `pkg_info` | Path to `pkg_info` |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct PkgsrcModule;

impl ModuleInvoker for PkgsrcModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");
        let tool = detect_tool(args);

        match tool.as_str() {
            "pkg_add" => pkgsrc_lowlevel_dispatch(state, &packages, args, host),
            _ => pkgin_dispatch(state, &packages, args, host),
        }
    }
}

fn detect_tool(args: &ModuleArgs) -> String {
    if let Some(t) = args.get_str("tool") {
        return t.to_string();
    }
    if which(args.get_str("pkgin_bin").unwrap_or("pkgin")) {
        return "pkgin".into();
    }
    "pkg_add".into()
}

fn which(binary: &str) -> bool {
    Command::new("which")
        .arg(binary)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// pkgin path
// ---------------------------------------------------------------------------

fn pkgin_dispatch(
    state: &str,
    packages: &[String],
    args: &ModuleArgs,
    host: &str,
) -> Result<TaskResult> {
    let bin = args.get_str("pkgin_bin").unwrap_or("pkgin").to_string();
    let yes = bool_arg(args, "yes", true);

    if bool_arg(args, "update_cache", false) {
        let mut cmd = Command::new(&bin);
        if yes {
            cmd.arg("-y");
        }
        cmd.arg("update");
        let s = cmd.status().context("pkgin update")?;
        if !s.success() {
            anyhow::bail!("pkgin update failed");
        }
    }

    match state {
        "absent" => {
            if packages.is_empty() {
                return Ok(TaskResult::failed(
                    host,
                    "pkgsrc: 'name' is required for state=absent".to_string(),
                ));
            }
            pkgin_run(&bin, "remove", packages, yes, host)
        }
        "upgrade_all" => {
            let subcmd = if bool_arg(args, "full", false) {
                "full-upgrade"
            } else {
                "upgrade"
            };
            pkgin_run(&bin, subcmd, &[], yes, host)
        }
        "clean" => pkgin_run(&bin, "clean-cache", &[], yes, host),
        "info" => {
            if packages.is_empty() {
                return Ok(TaskResult::failed(
                    host,
                    "pkgsrc: 'name' is required for state=info".to_string(),
                ));
            }
            pkgin_info(&bin, packages, host)
        }
        "latest" => {
            if packages.is_empty() {
                return pkgin_run(&bin, "upgrade", &[], yes, host);
            }
            pkgin_run(&bin, "install", packages, yes, host)
        }
        _ => {
            if packages.is_empty() {
                return Ok(TaskResult::failed(
                    host,
                    "pkgsrc: 'name' is required for state=present".to_string(),
                ));
            }
            pkgin_run(&bin, "install", packages, yes, host)
        }
    }
}

fn pkgin_run(
    bin: &str,
    subcmd: &str,
    packages: &[String],
    yes: bool,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    if yes {
        cmd.arg("-y");
    }
    cmd.arg(subcmd);
    for p in packages {
        cmd.arg(p);
    }
    let out = cmd.output().with_context(|| format!("pkgin {subcmd}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let changed = !stdout.contains("nothing to do") && !stdout.contains("up to date");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = format!("pkgin {subcmd}");
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pkgin {subcmd} failed: {stderr}"),
        ))
    }
}

fn pkgin_info(bin: &str, packages: &[String], host: &str) -> Result<TaskResult> {
    let mut out_all = String::new();
    for pkg in packages {
        if let Ok(out) = Command::new(bin).args(["show", pkg]).output() {
            out_all.push_str(&String::from_utf8_lossy(&out.stdout));
        }
    }
    let mut r = TaskResult::ok(host);
    r.stdout = out_all.clone();
    r.vars.insert("info".into(), Value::String(out_all));
    Ok(r)
}

// ---------------------------------------------------------------------------
// Low-level pkg_add/pkg_delete path (NetBSD without pkgin)
// ---------------------------------------------------------------------------

fn pkgsrc_lowlevel_dispatch(
    state: &str,
    packages: &[String],
    args: &ModuleArgs,
    host: &str,
) -> Result<TaskResult> {
    let info_bin = args
        .get_str("pkg_info_bin")
        .unwrap_or("pkg_info")
        .to_string();

    let is_installed = |pkg: &str| -> bool {
        Command::new(&info_bin)
            .args(["-e", pkg])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };

    match state {
        "absent" => {
            if packages.is_empty() {
                return Ok(TaskResult::failed(
                    host,
                    "pkgsrc: 'name' required for state=absent".to_string(),
                ));
            }
            let bin = args
                .get_str("pkg_delete_bin")
                .unwrap_or("pkg_delete")
                .to_string();
            let installed: Vec<&String> = packages.iter().filter(|p| is_installed(p)).collect();
            if installed.is_empty() {
                return Ok(TaskResult::ok(host));
            }
            let mut cmd = Command::new(&bin);
            for p in &installed {
                cmd.arg(p.as_str());
            }
            let out = cmd.output().context("pkg_delete")?;
            if out.status.success() {
                let mut r = TaskResult::changed(host);
                r.msg = format!(
                    "removed: {}",
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
                    format!(
                        "pkg_delete failed: {}",
                        String::from_utf8_lossy(&out.stderr)
                    ),
                ))
            }
        }
        _ => {
            if packages.is_empty() {
                return Ok(TaskResult::failed(
                    host,
                    "pkgsrc: 'name' required for state=present".to_string(),
                ));
            }
            let bin = args.get_str("pkg_add_bin").unwrap_or("pkg_add").to_string();
            let needs: Vec<&String> = packages.iter().filter(|p| !is_installed(p)).collect();
            if needs.is_empty() {
                return Ok(TaskResult::ok(host));
            }
            let mut cmd = Command::new(&bin);
            if let Some(path) = args.get_str("pkg_path") {
                cmd.env("PKG_PATH", path);
            }
            for p in &needs {
                cmd.arg(p.as_str());
            }
            let out = cmd.output().context("pkg_add")?;
            if out.status.success() {
                let mut r = TaskResult::changed(host);
                r.msg = format!(
                    "installed: {}",
                    needs
                        .iter()
                        .map(|p| p.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                Ok(r)
            } else {
                Ok(TaskResult::failed(
                    host,
                    format!("pkg_add failed: {}", String::from_utf8_lossy(&out.stderr)),
                ))
            }
        }
    }
}

fn collect_packages(args: &ModuleArgs) -> Vec<String> {
    let val = args.args.get("name").or_else(|| args.args.get("pkg"));
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
    fn test_no_name_present_fails() {
        let mut c = ctx();
        // Force pkgin path with non-existent binary so we get routing, not exec error.
        let a = args(&[("tool", Value::String("pkg_add".into()))]);
        let r = PkgsrcModule.invoke(&a, "h", &mut c).unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails_pkgin() {
        let r = pkgin_dispatch("absent", &[], &ModuleArgs::new(HashMap::new()), "h").unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails_lowlevel() {
        let r =
            pkgsrc_lowlevel_dispatch("absent", &[], &ModuleArgs::new(HashMap::new()), "h").unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_collect_packages() {
        let a = args(&[("name", Value::String("vim".into()))]);
        assert_eq!(collect_packages(&a), vec!["vim"]);
    }
    #[test]
    fn test_yes_default_true() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(bool_arg(&a, "yes", true));
    }
    #[test]
    fn test_full_default_false() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "full", false));
    }
}
