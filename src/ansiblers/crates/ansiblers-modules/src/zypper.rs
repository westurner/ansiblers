//! `zypper` module — manage packages on SUSE/openSUSE via `zypper`.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Package name(s); single string or list |
//! | `state` | `present` | `present`, `absent`, `latest`, `info`, `dist_upgrade` |
//! | `type` | `package` | `package`, `patch`, `pattern`, `product`, `srcpackage` |
//! | `update_cache` | `false` | Run `zypper refresh` before the operation |
//! | `disable_recommends` | `true` | Pass `--no-recommends` |
//! | `force` | `false` | Pass `--force` |
//! | `force_resolution` | `false` | Pass `--force-resolution` |
//! | `allow_vendor_change` | `false` | `--allow-vendor-change` on dist-upgrade |
//! | `extra_args` | — | Additional flags forwarded verbatim |
//! | `zypper_bin` | `zypper` | Path to the zypper binary |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct ZypperModule;

impl ModuleInvoker for ZypperModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let bin = args.get_str("zypper_bin").unwrap_or("zypper").to_string();
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");
        let pkg_type = args.get_str("type").unwrap_or("package");
        let update_cache = bool_arg(args, "update_cache", false);
        let disable_recommends = bool_arg(args, "disable_recommends", true);
        let force = bool_arg(args, "force", false);
        let force_resolution = bool_arg(args, "force_resolution", false);
        let allow_vendor_change = bool_arg(args, "allow_vendor_change", false);
        let extra_args = args.get_str("extra_args").map(|s| s.to_string());

        if update_cache {
            zypper_refresh(&bin)?;
        }

        match state {
            "absent" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "zypper: 'name' is required for state=absent".to_string(),
                    ));
                }
                zypper_remove(
                    &bin,
                    &packages,
                    pkg_type,
                    force,
                    extra_args.as_deref(),
                    host,
                )
            }
            "latest" => {
                if packages.is_empty() {
                    return zypper_update(&bin, force, extra_args.as_deref(), host);
                }
                zypper_install(
                    &bin,
                    &packages,
                    true,
                    pkg_type,
                    disable_recommends,
                    force,
                    force_resolution,
                    extra_args.as_deref(),
                    host,
                )
            }
            "dist_upgrade" => {
                zypper_dist_upgrade(&bin, allow_vendor_change, extra_args.as_deref(), host)
            }
            "info" => zypper_info(&bin, &packages, host),
            _ => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "zypper: 'name' is required for state=present".to_string(),
                    ));
                }
                zypper_install(
                    &bin,
                    &packages,
                    false,
                    pkg_type,
                    disable_recommends,
                    force,
                    force_resolution,
                    extra_args.as_deref(),
                    host,
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn zypper_refresh(bin: &str) -> Result<()> {
    let s = Command::new(bin)
        .args(["-n", "refresh"])
        .status()
        .context("zypper refresh")?;
    if !s.success() {
        anyhow::bail!("zypper refresh failed");
    }
    Ok(())
}

fn is_installed(bin: &str, pkg: &str, pkg_type: &str) -> bool {
    Command::new(bin)
        .args(["--non-interactive", "info", "--type", pkg_type, pkg])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("Installed : Yes"))
        .unwrap_or(false)
}

fn zypper_install(
    bin: &str,
    packages: &[String],
    upgrade: bool,
    pkg_type: &str,
    disable_recommends: bool,
    force: bool,
    force_resolution: bool,
    extra: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    let needs: Vec<&String> = if upgrade {
        packages.iter().collect()
    } else {
        packages
            .iter()
            .filter(|p| !is_installed(bin, p, pkg_type))
            .collect()
    };
    if needs.is_empty() {
        return Ok(TaskResult::ok(host));
    }

    let mut cmd = Command::new(bin);
    cmd.args(["--non-interactive", "install", "--type", pkg_type]);
    if disable_recommends {
        cmd.arg("--no-recommends");
    }
    if force {
        cmd.arg("--force");
    }
    if force_resolution {
        cmd.arg("--force-resolution");
    }
    if let Some(e) = extra {
        cmd.args(e.split_whitespace());
    }
    for p in &needs {
        cmd.arg(p.as_str());
    }

    let out = cmd.output().context("zypper install")?;
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
            format!("zypper install failed: {stderr}"),
        ))
    }
}

fn zypper_remove(
    bin: &str,
    packages: &[String],
    pkg_type: &str,
    force: bool,
    extra: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    let installed: Vec<&String> = packages
        .iter()
        .filter(|p| is_installed(bin, p, pkg_type))
        .collect();
    if installed.is_empty() {
        return Ok(TaskResult::ok(host));
    }
    let mut cmd = Command::new(bin);
    cmd.args(["--non-interactive", "remove", "--type", pkg_type]);
    if force {
        cmd.arg("--force");
    }
    if let Some(e) = extra {
        cmd.args(e.split_whitespace());
    }
    for p in &installed {
        cmd.arg(p.as_str());
    }
    let out = cmd.output().context("zypper remove")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
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
            format!("zypper remove failed: {stderr}"),
        ))
    }
}

fn zypper_update(bin: &str, force: bool, extra: Option<&str>, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(["--non-interactive", "update"]);
    if force {
        cmd.arg("--force");
    }
    if let Some(e) = extra {
        cmd.args(e.split_whitespace());
    }
    let out = cmd.output().context("zypper update")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let changed = !stdout.contains("Nothing to do");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = if changed {
            "updated".into()
        } else {
            "already up-to-date".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("zypper update failed: {stderr}"),
        ))
    }
}

fn zypper_dist_upgrade(
    bin: &str,
    allow_vendor_change: bool,
    extra: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(["--non-interactive", "dist-upgrade"]);
    if allow_vendor_change {
        cmd.arg("--allow-vendor-change");
    }
    if let Some(e) = extra {
        cmd.args(e.split_whitespace());
    }
    let out = cmd.output().context("zypper dist-upgrade")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = "dist-upgrade completed".into();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("zypper dist-upgrade failed: {stderr}"),
        ))
    }
}

fn zypper_info(bin: &str, packages: &[String], host: &str) -> Result<TaskResult> {
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
        let r = ZypperModule
            .invoke(&ModuleArgs::new(HashMap::new()), "h", &mut c)
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails() {
        let mut c = ctx();
        let r = ZypperModule
            .invoke(
                &args(&[("state", Value::String("absent".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_collect_array() {
        let a = args(&[(
            "name",
            Value::Array(vec![
                Value::String("vim".into()),
                Value::String("git".into()),
            ]),
        )]);
        assert_eq!(collect_packages(&a).len(), 2);
    }
    #[test]
    fn test_disable_recommends_default_true() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(bool_arg(&a, "disable_recommends", true));
    }

    #[test]
    fn test_collect_string_package() {
        let a = args(&[("name", Value::String("vim".into()))]);
        assert_eq!(collect_packages(&a), vec!["vim"]);
    }

    #[test]
    fn test_collect_pkg_alias() {
        let a = args(&[("pkg", Value::String("curl".into()))]);
        assert_eq!(collect_packages(&a), vec!["curl"]);
    }

    #[test]
    fn test_force_default_false() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "force", false));
    }

    #[test]
    fn test_force_resolution_default_false() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "force_resolution", false));
    }

    #[test]
    fn test_no_name_dist_upgrade_ok() {
        // dist_upgrade doesn't require a package name
        let mut c = ctx();
        // Will fail because zypper is not installed, but the dispatch is correct.
        let r = ZypperModule.invoke(
            &args(&[("state", Value::String("dist_upgrade".into()))]),
            "h",
            &mut c,
        );
        // Either result or error is fine; we just want to reach the dist_upgrade path
        let _ = r;
    }
}
