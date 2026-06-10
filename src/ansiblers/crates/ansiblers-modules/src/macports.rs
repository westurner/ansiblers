//! `macports` module — manage packages on macOS via [MacPorts](https://www.macports.org/).
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Port name(s), optionally with variants (`+ssl+python312`) |
//! | `state` | `present` | `present`, `absent`, `latest`, `info`, `selfupdate`, `reclaim` |
//! | `variants` | — | Port variants to activate (e.g. `+ssl +python312`) |
//! | `update_cache` | `false` | Run `port selfupdate` before operation |
//! | `upgrade_all` | `false` | Run `port upgrade outdated` |
//! | `source` | `false` | Build from source (`port -s install`) |
//! | `nosync` | `false` | Pass `--nosync` (skip tree sync for this op) |
//! | `verbose` | `false` | Pass `-v` |
//! | `debug` | `false` | Pass `-d` |
//! | `port_bin` | `port` | Path to the `port` binary |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct MacPortsModule;

impl ModuleInvoker for MacPortsModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let bin = args.get_str("port_bin").unwrap_or("port").to_string();
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");

        if bool_arg(args, "update_cache", false) {
            port_selfupdate(&bin, host)?;
        }

        if bool_arg(args, "upgrade_all", false) && packages.is_empty() {
            return port_upgrade_outdated(&bin, args, host);
        }

        match state {
            "selfupdate" => {
                port_selfupdate(&bin, host)?;
                let mut r = TaskResult::changed(host);
                r.msg = "port tree updated".into();
                Ok(r)
            }
            "reclaim" => port_reclaim(&bin, host),
            "absent" | "uninstall" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "macports: 'name' is required for state=absent".to_string(),
                    ));
                }
                port_uninstall(&bin, &packages, args, host)
            }
            "latest" | "upgrade" => {
                if packages.is_empty() {
                    return port_upgrade_outdated(&bin, args, host);
                }
                port_upgrade(&bin, &packages, args, host)
            }
            "info" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "macports: 'name' is required for state=info".to_string(),
                    ));
                }
                port_info(&bin, &packages, host)
            }
            _ => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "macports: 'name' is required for state=present".to_string(),
                    ));
                }
                port_install(&bin, &packages, args, host)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn global_flags(args: &ModuleArgs) -> Vec<String> {
    let mut v = Vec::new();
    if bool_arg(args, "verbose", false) {
        v.push("-v".into());
    }
    if bool_arg(args, "debug", false) {
        v.push("-d".into());
    }
    if bool_arg(args, "source", false) {
        v.push("-s".into());
    }
    if bool_arg(args, "nosync", false) {
        v.push("--nosync".into());
    }
    v
}

fn is_installed(bin: &str, pkg: &str) -> bool {
    // Strip variants for the presence check.
    let bare = pkg.split('+').next().unwrap_or(pkg).trim();
    Command::new(bin)
        .args(["installed", bare])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("(active)"))
        .unwrap_or(false)
}

fn variant_args(args: &ModuleArgs) -> Vec<String> {
    if let Some(v) = args.get_str("variants") {
        v.split_whitespace().map(|s| s.to_string()).collect()
    } else {
        vec![]
    }
}

fn port_selfupdate(bin: &str, host: &str) -> Result<()> {
    let s = Command::new(bin)
        .arg("selfupdate")
        .status()
        .context("port selfupdate")?;
    if !s.success() {
        anyhow::bail!("port selfupdate failed");
    }
    Ok(())
}

fn port_install(
    bin: &str,
    packages: &[String],
    args: &ModuleArgs,
    host: &str,
) -> Result<TaskResult> {
    let needs: Vec<&String> = packages.iter().filter(|p| !is_installed(bin, p)).collect();
    if needs.is_empty() {
        return Ok(TaskResult::ok(host));
    }

    let mut cmd = Command::new(bin);
    cmd.args(global_flags(args));
    cmd.arg("install");
    for p in &needs {
        cmd.arg(p.as_str());
        cmd.args(variant_args(args));
    }
    let out = cmd.output().context("port install")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
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
            format!("port install failed: {stderr}"),
        ))
    }
}

fn port_upgrade(
    bin: &str,
    packages: &[String],
    args: &ModuleArgs,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(global_flags(args));
    cmd.arg("upgrade");
    for p in packages {
        cmd.arg(p.as_str());
        cmd.args(variant_args(args));
    }
    let out = cmd.output().context("port upgrade")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let changed = !stdout.contains("already the latest");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = if changed {
            format!("upgraded: {}", packages.join(", "))
        } else {
            "already up-to-date".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("port upgrade failed: {stderr}"),
        ))
    }
}

fn port_upgrade_outdated(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(global_flags(args));
    cmd.args(["upgrade", "outdated"]);
    let out = cmd.output().context("port upgrade outdated")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let changed = !stdout.contains("Nothing to upgrade");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = if changed {
            "upgraded outdated ports".into()
        } else {
            "all ports up-to-date".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("port upgrade outdated failed: {stderr}"),
        ))
    }
}

fn port_uninstall(
    bin: &str,
    packages: &[String],
    args: &ModuleArgs,
    host: &str,
) -> Result<TaskResult> {
    let installed: Vec<&String> = packages.iter().filter(|p| is_installed(bin, p)).collect();
    if installed.is_empty() {
        return Ok(TaskResult::ok(host));
    }
    let mut cmd = Command::new(bin);
    cmd.args(global_flags(args));
    cmd.arg("uninstall");
    for p in &installed {
        cmd.arg(p.as_str());
    }
    let out = cmd.output().context("port uninstall")?;
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
            format!("port uninstall failed: {stderr}"),
        ))
    }
}

fn port_reclaim(bin: &str, host: &str) -> Result<TaskResult> {
    let out = Command::new(bin)
        .arg("reclaim")
        .output()
        .context("port reclaim")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = "reclaimed disk space".into();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("port reclaim failed: {stderr}"),
        ))
    }
}

fn port_info(bin: &str, packages: &[String], host: &str) -> Result<TaskResult> {
    let mut out_all = String::new();
    for pkg in packages {
        if let Ok(out) = Command::new(bin).args(["info", pkg]).output() {
            out_all.push_str(&String::from_utf8_lossy(&out.stdout));
        }
    }
    let mut r = TaskResult::ok(host);
    r.stdout = out_all.clone();
    r.vars.insert("info".into(), Value::String(out_all));
    Ok(r)
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
        let r = MacPortsModule
            .invoke(&ModuleArgs::new(HashMap::new()), "h", &mut c)
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails() {
        let mut c = ctx();
        let r = MacPortsModule
            .invoke(
                &args(&[("state", Value::String("absent".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_variant_strip_for_presence_check() {
        let bare = "vim+python312"
            .split('+')
            .next()
            .unwrap_or("vim+python312")
            .trim();
        assert_eq!(bare, "vim");
    }
    #[test]
    fn test_global_flags_source() {
        let a = args(&[("source", Value::Bool(true))]);
        assert!(global_flags(&a).contains(&"-s".to_string()));
    }
    #[test]
    fn test_global_flags_nosync() {
        let a = args(&[("nosync", Value::Bool(true))]);
        assert!(global_flags(&a).contains(&"--nosync".to_string()));
    }
    #[test]
    fn test_variant_args_split() {
        let a = args(&[("variants", Value::String("+ssl +python312".into()))]);
        assert_eq!(variant_args(&a), vec!["+ssl", "+python312"]);
    }
    #[test]
    fn test_collect_packages() {
        let a = args(&[("name", Value::String("vim".into()))]);
        assert_eq!(collect_packages(&a), vec!["vim"]);
    }
    #[test]
    fn test_verbose_default_false() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "verbose", false));
    }
}
