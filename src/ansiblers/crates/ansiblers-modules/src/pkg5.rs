//! `pkg5` module — manage packages on OpenIndiana and Solaris 11+ via the
//! Image Packaging System (IPS) `pkg` command (pkg(5) / pkg(1M)).
//!
//! OpenIndiana and Oracle Solaris 11 both use IPS as their primary package
//! manager.  This module wraps the `pkg` command for common operations.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Package FMRI(s) (e.g. `editor/vim`, `pkg:/entire`) |
//! | `state` | `present` | `present`, `absent`, `latest`, `info`, `freeze`, `unfreeze`, `verify`, `fix`, `revert`, `purge_history` |
//! | `publisher` | — | Preferred publisher/repository to use |
//! | `be_name` | — | Boot environment name for staged updates |
//! | `accept` | `false` | Accept all license agreements (`--accept`) |
//! | `backup_be` | `false` | Create a backup boot environment |
//! | `reject` | — | Package FMRI(s) to reject during update |
//! | `update_cache` | `false` | Run `pkg refresh` before operation |
//! | `pkg_bin` | `pkg` | Path to the pkg binary |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct Pkg5Module;

impl ModuleInvoker for Pkg5Module {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let bin = args.get_str("pkg_bin").unwrap_or("pkg").to_string();
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");

        if bool_arg(args, "update_cache", false) {
            pkg_refresh(&bin, args)?;
        }

        match state {
            "verify" => pkg_verify(&bin, &packages, args, host),
            "fix" => pkg_fix(&bin, &packages, args, host),
            "revert" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pkg5: 'name' is required for state=revert".to_string(),
                    ));
                }
                pkg_revert(&bin, &packages, host)
            }
            "freeze" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pkg5: 'name' is required for state=freeze".to_string(),
                    ));
                }
                pkg_freeze(&bin, &packages, host)
            }
            "unfreeze" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pkg5: 'name' is required for state=unfreeze".to_string(),
                    ));
                }
                pkg_unfreeze(&bin, &packages, host)
            }
            "purge_history" => pkg_purge_history(&bin, host),
            "info" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pkg5: 'name' is required for state=info".to_string(),
                    ));
                }
                pkg_info(&bin, &packages, host)
            }
            "absent" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pkg5: 'name' is required for state=absent".to_string(),
                    ));
                }
                pkg_uninstall(&bin, &packages, args, host)
            }
            "latest" => {
                if packages.is_empty() {
                    return pkg_update_all(&bin, args, host);
                }
                pkg_install(&bin, &packages, args, true, host)
            }
            _ => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pkg5: 'name' is required for state=present".to_string(),
                    ));
                }
                pkg_install(&bin, &packages, args, false, host)
            }
        }
    }
}

fn common_flags(args: &ModuleArgs) -> Vec<String> {
    let mut v = Vec::new();
    if bool_arg(args, "accept", false) {
        v.push("--accept".into());
    }
    if bool_arg(args, "backup_be", false) {
        v.push("--backup-be".into());
    }
    if let Some(be) = args.get_str("be_name") {
        v.push("--be-name".into());
        v.push(be.to_string());
    }
    if let Some(pub_) = args.get_str("publisher") {
        v.push(format!("--preferred-publisher={pub_}"));
    }
    for rej in collect_list(args, "reject") {
        v.push("--reject".into());
        v.push(rej);
    }
    v
}

fn is_installed(bin: &str, pkg: &str) -> bool {
    Command::new(bin)
        .args(["list", "-q", pkg])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn pkg_refresh(bin: &str, args: &ModuleArgs) -> Result<()> {
    let mut cmd = Command::new(bin);
    cmd.arg("refresh");
    if let Some(pub_) = args.get_str("publisher") {
        cmd.arg(pub_);
    }
    let s = cmd.status().context("pkg refresh")?;
    if !s.success() {
        anyhow::bail!("pkg refresh failed");
    }
    Ok(())
}

fn pkg_install(
    bin: &str,
    packages: &[String],
    args: &ModuleArgs,
    upgrade: bool,
    host: &str,
) -> Result<TaskResult> {
    let needs: Vec<&String> = if upgrade {
        packages.iter().collect()
    } else {
        packages.iter().filter(|p| !is_installed(bin, p)).collect()
    };
    if needs.is_empty() {
        return Ok(TaskResult::ok(host));
    }

    let subcmd = if upgrade { "update" } else { "install" };
    let mut cmd = Command::new(bin);
    cmd.arg(subcmd);
    cmd.args(common_flags(args));
    for p in &needs {
        cmd.arg(p.as_str());
    }

    run_pkg(
        cmd,
        host,
        &format!(
            "{subcmd}ed: {}",
            needs
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    )
}

fn pkg_uninstall(
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
    cmd.arg("uninstall");
    cmd.args(common_flags(args));
    for p in &installed {
        cmd.arg(p.as_str());
    }
    run_pkg(
        cmd,
        host,
        &format!(
            "uninstalled: {}",
            installed
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    )
}

fn pkg_update_all(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("update");
    cmd.args(common_flags(args));
    let out = cmd.output().context("pkg update")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() || out.status.code() == Some(4)
    /* nothing to update */
    {
        let changed = out.status.code() != Some(4);
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = if changed {
            "system updated".into()
        } else {
            "already up-to-date".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pkg update failed: {stderr}"),
        ))
    }
}

fn pkg_verify(bin: &str, packages: &[String], args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("verify");
    for p in packages {
        cmd.arg(p);
    }
    let out = cmd.output().context("pkg verify")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let has_errors = !out.status.success();
    let mut r = if has_errors {
        TaskResult::failed(host, format!("pkg verify found errors: {stdout}"))
    } else {
        TaskResult::ok(host)
    };
    r.stdout = stdout;
    Ok(r)
}

fn pkg_fix(bin: &str, packages: &[String], args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("fix");
    cmd.args(common_flags(args));
    for p in packages {
        cmd.arg(p);
    }
    run_pkg(cmd, host, "packages fixed")
}

fn pkg_revert(bin: &str, packages: &[String], host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("revert");
    for p in packages {
        cmd.arg(p);
    }
    run_pkg(cmd, host, "files reverted")
}

fn pkg_freeze(bin: &str, packages: &[String], host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("freeze");
    for p in packages {
        cmd.arg(p);
    }
    run_pkg(cmd, host, &format!("frozen: {}", packages.join(", ")))
}

fn pkg_unfreeze(bin: &str, packages: &[String], host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(["freeze", "-n"]);
    for p in packages {
        cmd.arg(p);
    }
    run_pkg(cmd, host, &format!("unfrozen: {}", packages.join(", ")))
}

fn pkg_purge_history(bin: &str, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("purge-history");
    run_pkg(cmd, host, "history purged")
}

fn pkg_info(bin: &str, packages: &[String], host: &str) -> Result<TaskResult> {
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

fn run_pkg(cmd: Command, host: &str, msg: &str) -> Result<TaskResult> {
    let mut cmd = cmd;
    let out = cmd.output().context("pkg")?;
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
            format!("pkg failed (rc={rc}): {stderr}"),
        ))
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

fn collect_list(args: &ModuleArgs, key: &str) -> Vec<String> {
    match args.args.get(key) {
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
        let r = Pkg5Module
            .invoke(&ModuleArgs::new(HashMap::new()), "h", &mut c)
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails() {
        let mut c = ctx();
        let r = Pkg5Module
            .invoke(
                &args(&[("state", Value::String("absent".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_freeze_fails() {
        let mut c = ctx();
        let r = Pkg5Module
            .invoke(
                &args(&[("state", Value::String("freeze".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_collect_fmri() {
        let a = args(&[("name", Value::String("pkg:/editor/vim".into()))]);
        assert_eq!(collect_packages(&a), vec!["pkg:/editor/vim"]);
    }
    #[test]
    fn test_accept_default_false() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "accept", false));
    }
    #[test]
    fn test_collect_list_reject() {
        let a = args(&[("reject", Value::String("pkg:/entire".into()))]);
        assert_eq!(collect_list(&a, "reject"), vec!["pkg:/entire"]);
    }
}
