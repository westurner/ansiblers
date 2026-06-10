//! `flatpak` module — manage Flatpak applications and runtimes.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Application ID(s) (e.g. `org.mozilla.firefox`) |
//! | `state` | `present` | `present`, `absent`, `latest`, `info` |
//! | `remote` | `flathub` | Remote name to install from |
//! | `method` | `system` | `system` or `user` installation |
//! | `no_auto_pin` | `false` | Pass `--no-auto-pin` |
//! | `no_deps` | `false` | Pass `--no-deps` |
//! | `remote_add` | — | Name of a remote to add (when set, `remote_url` is required) |
//! | `remote_url` | — | URL for the new remote |
//! | `remote_if_not_exists` | `true` | Only add remote when it doesn't already exist |
//! | `flatpak_bin` | `flatpak` | Path to the flatpak binary |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct FlatpakModule;

impl ModuleInvoker for FlatpakModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let bin = args.get_str("flatpak_bin").unwrap_or("flatpak").to_string();
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");
        let remote = args.get_str("remote").unwrap_or("flathub").to_string();
        let user = args.get_str("method").map_or(false, |m| m == "user");
        let no_auto_pin = bool_arg(args, "no_auto_pin", false);
        let no_deps = bool_arg(args, "no_deps", false);

        // Handle remote management first.
        if let Some(remote_name) = args.get_str("remote_add") {
            let remote_url = args.get_str("remote_url").ok_or_else(|| {
                anyhow::anyhow!("flatpak: 'remote_url' is required when 'remote_add' is set")
            })?;
            return flatpak_remote_add(
                &bin,
                remote_name,
                remote_url,
                user,
                bool_arg(args, "remote_if_not_exists", true),
                host,
            );
        }

        match state {
            "absent" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "flatpak: 'name' is required for state=absent".to_string(),
                    ));
                }
                flatpak_uninstall(&bin, &packages, user, host)
            }
            "latest" => {
                if packages.is_empty() {
                    return flatpak_update_all(&bin, user, host);
                }
                flatpak_install(
                    &bin,
                    &packages,
                    &remote,
                    user,
                    no_auto_pin,
                    no_deps,
                    true,
                    host,
                )
            }
            "info" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "flatpak: 'name' is required for state=info".to_string(),
                    ));
                }
                flatpak_info(&bin, &packages, host)
            }
            _ => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "flatpak: 'name' is required for state=present".to_string(),
                    ));
                }
                flatpak_install(
                    &bin,
                    &packages,
                    &remote,
                    user,
                    no_auto_pin,
                    no_deps,
                    false,
                    host,
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn scope_arg(user: bool) -> &'static str {
    if user {
        "--user"
    } else {
        "--system"
    }
}

fn is_installed(bin: &str, app_id: &str, user: bool) -> bool {
    Command::new(bin)
        .args(["info", scope_arg(user), app_id])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn flatpak_install(
    bin: &str,
    packages: &[String],
    remote: &str,
    user: bool,
    no_auto_pin: bool,
    no_deps: bool,
    upgrade: bool,
    host: &str,
) -> Result<TaskResult> {
    let needs: Vec<&String> = if upgrade {
        packages.iter().collect()
    } else {
        packages
            .iter()
            .filter(|p| !is_installed(bin, p, user))
            .collect()
    };
    if needs.is_empty() {
        return Ok(TaskResult::ok(host));
    }

    let mut cmd = Command::new(bin);
    cmd.args(["install", "-y", "--noninteractive", scope_arg(user), remote]);
    if no_auto_pin {
        cmd.arg("--no-auto-pin");
    }
    if no_deps {
        cmd.arg("--no-deps");
    }
    for p in &needs {
        cmd.arg(p.as_str());
    }

    let out = cmd.output().context("flatpak install")?;
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
            format!("flatpak install failed: {stderr}"),
        ))
    }
}

fn flatpak_uninstall(bin: &str, packages: &[String], user: bool, host: &str) -> Result<TaskResult> {
    let installed: Vec<&String> = packages
        .iter()
        .filter(|p| is_installed(bin, p, user))
        .collect();
    if installed.is_empty() {
        return Ok(TaskResult::ok(host));
    }
    let mut cmd = Command::new(bin);
    cmd.args(["uninstall", "-y", "--noninteractive", scope_arg(user)]);
    for p in &installed {
        cmd.arg(p.as_str());
    }
    let out = cmd.output().context("flatpak uninstall")?;
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
            format!("flatpak uninstall failed: {stderr}"),
        ))
    }
}

fn flatpak_update_all(bin: &str, user: bool, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(["update", "-y", "--noninteractive", scope_arg(user)]);
    let out = cmd.output().context("flatpak update")?;
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
            "updated all Flatpaks".into()
        } else {
            "all Flatpaks up-to-date".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("flatpak update failed: {stderr}"),
        ))
    }
}

fn flatpak_info(bin: &str, packages: &[String], host: &str) -> Result<TaskResult> {
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

fn flatpak_remote_add(
    bin: &str,
    name: &str,
    url: &str,
    user: bool,
    if_not_exists: bool,
    host: &str,
) -> Result<TaskResult> {
    // Check if remote exists.
    let exists = Command::new(bin)
        .args(["remotes", scope_arg(user)])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(name))
        .unwrap_or(false);
    if exists && if_not_exists {
        return Ok(TaskResult::ok(host));
    }

    let mut cmd = Command::new(bin);
    cmd.args(["remote-add", "--if-not-exists", scope_arg(user), name, url]);
    let out = cmd.output().context("flatpak remote-add")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = format!("remote '{name}' added");
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("flatpak remote-add failed: {stderr}"),
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
        let r = FlatpakModule
            .invoke(&ModuleArgs::new(HashMap::new()), "h", &mut c)
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails() {
        let mut c = ctx();
        let r = FlatpakModule
            .invoke(
                &args(&[("state", Value::String("absent".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_remote_add_missing_url_errors() {
        let mut c = ctx();
        let r = FlatpakModule.invoke(
            &args(&[("remote_add", Value::String("flathub".into()))]),
            "h",
            &mut c,
        );
        assert!(r.is_err());
    }
    #[test]
    fn test_scope_arg_system() {
        assert_eq!(scope_arg(false), "--system");
    }
    #[test]
    fn test_scope_arg_user() {
        assert_eq!(scope_arg(true), "--user");
    }
}
