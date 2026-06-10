//! `pacman` module — manage packages on Arch Linux and derivatives via `pacman`.
//!
//! Supports `yay` / `paru` / `aur` AUR helpers as drop-in replacements via
//! the `aur_helper` parameter.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Package name(s); single string or list |
//! | `state` | `present` | `present`, `absent`, `latest`, `info` |
//! | `update_cache` | `false` | Run `pacman -Sy` before installing |
//! | `upgrade` | `false` | Run a full system upgrade (`-Su`) before installing |
//! | `force` | `false` | Add `--noconfirm --needed` to every operation |
//! | `reason` | — | `explicit` or `dependency` — sets install reason |
//! | `aur_helper` | — | Use this AUR helper instead of pacman (e.g. `yay`, `paru`) |
//! | `extra_args` | — | Extra flags forwarded verbatim |
//! | `pacman_bin` | `pacman` | Path to the pacman (or AUR-helper) binary |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct PacmanModule;

impl ModuleInvoker for PacmanModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        // Prefer an AUR helper when specified.
        let bin = args
            .get_str("aur_helper")
            .or_else(|| args.get_str("pacman_bin"))
            .unwrap_or("pacman")
            .to_string();

        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");
        let update_cache = bool_arg(args, "update_cache", false);
        let upgrade = bool_arg(args, "upgrade", false);
        let force = bool_arg(args, "force", true); // --noconfirm is practical default
        let reason = args.get_str("reason");
        let extra_args = args.get_str("extra_args").map(|s| s.to_string());

        if update_cache {
            pacman_sync_db(&bin, force)?;
        }

        if upgrade && packages.is_empty() {
            return pacman_sysupgrade(&bin, force, extra_args.as_deref(), host);
        }

        match state {
            "absent" | "removed" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pacman: 'name' is required for state=absent".to_string(),
                    ));
                }
                pacman_remove(&bin, &packages, force, extra_args.as_deref(), host)
            }
            "latest" => {
                if packages.is_empty() {
                    return pacman_sysupgrade(&bin, force, extra_args.as_deref(), host);
                }
                pacman_install(
                    &bin,
                    &packages,
                    upgrade,
                    force,
                    reason,
                    extra_args.as_deref(),
                    host,
                )
            }
            "info" => pacman_info(&bin, &packages, host),
            _ => {
                // present
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pacman: 'name' is required for state=present".to_string(),
                    ));
                }
                pacman_install(
                    &bin,
                    &packages,
                    upgrade,
                    force,
                    reason,
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

fn is_installed(bin: &str, pkg: &str) -> bool {
    Command::new(bin)
        .args(["-Q", pkg])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn pacman_sync_db(bin: &str, force: bool) -> Result<()> {
    let mut args = vec!["-Sy"];
    if force {
        args.push("--noconfirm");
    }
    let s = Command::new(bin)
        .args(&args)
        .status()
        .context("pacman -Sy")?;
    if !s.success() {
        anyhow::bail!("pacman -Sy failed");
    }
    Ok(())
}

fn pacman_sysupgrade(
    bin: &str,
    force: bool,
    extra: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("-Su");
    if force {
        cmd.arg("--noconfirm");
    }
    if let Some(e) = extra {
        cmd.args(e.split_whitespace());
    }
    let out = cmd.output().context("pacman -Su")?;
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
        r.msg = if changed {
            "system upgraded".into()
        } else {
            "already up-to-date".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pacman -Su failed: {stderr}"),
        ))
    }
}

fn pacman_install(
    bin: &str,
    packages: &[String],
    upgrade: bool,
    force: bool,
    reason: Option<&str>,
    extra: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    let needs: Vec<&String> = packages.iter().filter(|p| !is_installed(bin, p)).collect();
    if needs.is_empty() && !upgrade {
        return Ok(TaskResult::ok(host));
    }

    let mut cmd = Command::new(bin);
    let flag = if upgrade { "-Syu" } else { "-S" };
    cmd.args([flag, "--needed"]);
    if force {
        cmd.arg("--noconfirm");
    }
    if let Some(r) = reason {
        cmd.args(["--asdeps"]);
        let _ = r; // --asdeps / --asexplicit handled below
        if r == "dependency" {
            cmd.arg("--asdeps");
        } else if r == "explicit" {
            cmd.arg("--asexplicit");
        }
    }
    if let Some(e) = extra {
        cmd.args(e.split_whitespace());
    }
    for p in &needs {
        cmd.arg(p.as_str());
    }

    let out = cmd.output().context("pacman -S")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.stdout = stdout;
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
            format!("pacman install failed: {stderr}"),
        ))
    }
}

fn pacman_remove(
    bin: &str,
    packages: &[String],
    force: bool,
    extra: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    let installed: Vec<&String> = packages.iter().filter(|p| is_installed(bin, p)).collect();
    if installed.is_empty() {
        return Ok(TaskResult::ok(host));
    }
    let mut cmd = Command::new(bin);
    cmd.arg("-R");
    if force {
        cmd.arg("--noconfirm");
    }
    if let Some(e) = extra {
        cmd.args(e.split_whitespace());
    }
    for p in &installed {
        cmd.arg(p.as_str());
    }
    let out = cmd.output().context("pacman -R")?;
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
            format!("pacman -R failed: {stderr}"),
        ))
    }
}

fn pacman_info(bin: &str, packages: &[String], host: &str) -> Result<TaskResult> {
    let mut out_all = String::new();
    for pkg in packages {
        if let Ok(out) = Command::new(bin).args(["-Qi", pkg]).output() {
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
    fn test_collect_single() {
        let a = args(&[("name", Value::String("git".into()))]);
        assert_eq!(collect_packages(&a), vec!["git"]);
    }
    #[test]
    fn test_collect_array() {
        let a = args(&[(
            "name",
            Value::Array(vec![
                Value::String("git".into()),
                Value::String("vim".into()),
            ]),
        )]);
        assert_eq!(collect_packages(&a).len(), 2);
    }
    #[test]
    fn test_collect_space_separated() {
        let a = args(&[("name", Value::String("git vim".into()))]);
        assert_eq!(collect_packages(&a), vec!["git", "vim"]);
    }
    #[test]
    fn test_no_name_present_fails() {
        let mut c = ctx();
        let a = args(&[("state", Value::String("present".into()))]);
        let r = PacmanModule.invoke(&a, "h", &mut c).unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails() {
        let mut c = ctx();
        let a = args(&[("state", Value::String("absent".into()))]);
        let r = PacmanModule.invoke(&a, "h", &mut c).unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_bool_arg_default() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "update_cache", false));
    }
}
