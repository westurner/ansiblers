//! `snap` module — manage Snap packages via `snap` / `snapd`.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Snap name(s) |
//! | `state` | `present` | `present`, `absent`, `latest`, `info`, `refresh_all` |
//! | `classic` | `false` | Install in classic confinement (`--classic`) |
//! | `dangerous` | `false` | Allow unsigned snaps (`--dangerous`) |
//! | `devmode` | `false` | Install in developer mode |
//! | `jailmode` | `false` | Force strict confinement |
//! | `channel` | `stable` | Channel to install from (`stable`, `edge`, `beta`, `candidate`) |
//! | `revision` | — | Specific revision to install |
//! | `snap_bin` | `snap` | Path to the snap binary |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct SnapModule;

impl ModuleInvoker for SnapModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let bin = args.get_str("snap_bin").unwrap_or("snap").to_string();
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");

        match state {
            "refresh_all" => snap_refresh_all(&bin, host),
            "absent" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "snap: 'name' is required for state=absent".to_string(),
                    ));
                }
                snap_remove(&bin, &packages, host)
            }
            "latest" => {
                if packages.is_empty() {
                    return snap_refresh_all(&bin, host);
                }
                snap_install(&bin, &packages, args, true, host)
            }
            "info" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "snap: 'name' is required for state=info".to_string(),
                    ));
                }
                snap_info(&bin, &packages, host)
            }
            _ => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "snap: 'name' is required for state=present".to_string(),
                    ));
                }
                snap_install(&bin, &packages, args, false, host)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_installed(bin: &str, pkg: &str) -> bool {
    Command::new(bin)
        .args(["list", pkg])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn snap_install(
    bin: &str,
    packages: &[String],
    args: &ModuleArgs,
    refresh: bool,
    host: &str,
) -> Result<TaskResult> {
    let needs: Vec<&String> = if refresh {
        packages.iter().collect()
    } else {
        packages.iter().filter(|p| !is_installed(bin, p)).collect()
    };
    if needs.is_empty() {
        return Ok(TaskResult::ok(host));
    }

    let classic = bool_arg(args, "classic", false);
    let dangerous = bool_arg(args, "dangerous", false);
    let devmode = bool_arg(args, "devmode", false);
    let jailmode = bool_arg(args, "jailmode", false);
    let channel = args.get_str("channel").unwrap_or("stable");
    let revision = args.get_str("revision").map(|s| s.to_string());

    let subcommand = if refresh { "refresh" } else { "install" };

    for pkg in &needs {
        let mut cmd = Command::new(bin);
        cmd.arg(subcommand);
        if classic {
            cmd.arg("--classic");
        }
        if dangerous {
            cmd.arg("--dangerous");
        }
        if devmode {
            cmd.arg("--devmode");
        }
        if jailmode {
            cmd.arg("--jailmode");
        }
        cmd.arg(format!("--channel={channel}"));
        if let Some(rev) = &revision {
            cmd.args(["--revision", rev]);
        }
        cmd.arg(pkg.as_str());
        let out = cmd.output().context("snap install")?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            return Ok(TaskResult::failed(
                host,
                format!("snap {subcommand} '{pkg}' failed: {stderr}"),
            ));
        }
    }
    let mut r = TaskResult::changed(host);
    r.msg = format!(
        "{subcommand}ed: {}",
        needs
            .iter()
            .map(|p| p.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(r)
}

fn snap_remove(bin: &str, packages: &[String], host: &str) -> Result<TaskResult> {
    let installed: Vec<&String> = packages.iter().filter(|p| is_installed(bin, p)).collect();
    if installed.is_empty() {
        return Ok(TaskResult::ok(host));
    }
    for pkg in &installed {
        let out = Command::new(bin)
            .args(["remove", pkg.as_str()])
            .output()
            .context("snap remove")?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            return Ok(TaskResult::failed(
                host,
                format!("snap remove '{pkg}' failed: {stderr}"),
            ));
        }
    }
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
}

fn snap_refresh_all(bin: &str, host: &str) -> Result<TaskResult> {
    let out = Command::new(bin)
        .arg("refresh")
        .output()
        .context("snap refresh")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let changed = !stdout.contains("All snaps up to date");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = if changed {
            "snaps refreshed".into()
        } else {
            "all snaps up-to-date".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("snap refresh failed: {stderr}"),
        ))
    }
}

fn snap_info(bin: &str, packages: &[String], host: &str) -> Result<TaskResult> {
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
        let r = SnapModule
            .invoke(&ModuleArgs::new(HashMap::new()), "h", &mut c)
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails() {
        let mut c = ctx();
        let r = SnapModule
            .invoke(
                &args(&[("state", Value::String("absent".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_collect_packages_array() {
        let a = args(&[(
            "name",
            Value::Array(vec![
                Value::String("vlc".into()),
                Value::String("htop".into()),
            ]),
        )]);
        assert_eq!(collect_packages(&a).len(), 2);
    }
    #[test]
    fn test_classic_default_false() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "classic", false));
    }

    #[test]
    fn test_no_name_info_fails() {
        let mut c = ctx();
        let r = SnapModule
            .invoke(
                &args(&[("state", Value::String("info".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }

    #[test]
    fn test_collect_packages_single() {
        let a = args(&[("name", Value::String("vlc".into()))]);
        assert_eq!(collect_packages(&a), vec!["vlc"]);
    }

    #[test]
    fn test_collect_packages_space_separated() {
        let a = args(&[("name", Value::String("vlc htop".into()))]);
        assert_eq!(collect_packages(&a), vec!["vlc", "htop"]);
    }

    #[test]
    fn test_devmode_default_false() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "devmode", false));
    }

    #[test]
    fn test_jailmode_default_false() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "jailmode", false));
    }

    #[test]
    fn test_dangerous_default_false() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "dangerous", false));
    }
}
