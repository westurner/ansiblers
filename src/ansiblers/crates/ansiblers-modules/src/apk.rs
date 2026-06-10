//! `apk` module — manage packages on Alpine Linux via `apk`.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Package name(s); single string or list |
//! | `state` | `present` | `present`, `absent`, `latest`, `info` |
//! | `update_cache` | `false` | Run `apk update` first |
//! | `upgrade` | `false` | Run `apk upgrade` (full system upgrade) |
//! | `no_cache` | `false` | Pass `--no-cache` |
//! | `repository` | — | Extra repository URL to use for this operation |
//! | `world_file` | — | Override the world file path |
//! | `apk_bin` | `apk` | Path to the apk binary |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct ApkModule;

impl ModuleInvoker for ApkModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let bin = args.get_str("apk_bin").unwrap_or("apk").to_string();
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");
        let update_cache = bool_arg(args, "update_cache", false);
        let upgrade = bool_arg(args, "upgrade", false);
        let no_cache = bool_arg(args, "no_cache", false);
        let repository = args.get_str("repository").map(|s| s.to_string());

        if update_cache {
            apk_update(&bin, no_cache, repository.as_deref())?;
        }

        if upgrade && packages.is_empty() {
            return apk_upgrade(&bin, no_cache, repository.as_deref(), host);
        }

        match state {
            "absent" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "apk: 'name' required for state=absent".to_string(),
                    ));
                }
                apk_del(&bin, &packages, no_cache, host)
            }
            "latest" => {
                if packages.is_empty() {
                    return apk_upgrade(&bin, no_cache, repository.as_deref(), host);
                }
                apk_add(
                    &bin,
                    &packages,
                    upgrade,
                    no_cache,
                    repository.as_deref(),
                    host,
                )
            }
            "info" => apk_info(&bin, &packages, host),
            _ => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "apk: 'name' required for state=present".to_string(),
                    ));
                }
                apk_add(
                    &bin,
                    &packages,
                    upgrade,
                    no_cache,
                    repository.as_deref(),
                    host,
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn base_args<'a>(no_cache: bool, extra: Option<&'a str>) -> Vec<&'a str> {
    let mut v: Vec<&str> = vec![];
    if no_cache {
        v.push("--no-cache");
    }
    if let Some(e) = extra {
        v.push(e);
    }
    v
}

fn is_installed(bin: &str, pkg: &str) -> bool {
    Command::new(bin)
        .args(["info", "-e", pkg])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn apk_update(bin: &str, no_cache: bool, repo: Option<&str>) -> Result<()> {
    let mut cmd = Command::new(bin);
    cmd.arg("update");
    if no_cache {
        cmd.arg("--no-cache");
    }
    if let Some(r) = repo {
        cmd.args(["--repository", r]);
    }
    let s = cmd.status().context("apk update")?;
    if !s.success() {
        anyhow::bail!("apk update failed");
    }
    Ok(())
}

fn apk_upgrade(bin: &str, no_cache: bool, repo: Option<&str>, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("upgrade");
    if no_cache {
        cmd.arg("--no-cache");
    }
    if let Some(r) = repo {
        cmd.args(["--repository", r]);
    }
    let out = cmd.output().context("apk upgrade")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let changed = stdout.contains("Upgrading") || stdout.contains("Installing");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = if changed {
            "upgraded".into()
        } else {
            "already up-to-date".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("apk upgrade failed: {stderr}"),
        ))
    }
}

fn apk_add(
    bin: &str,
    packages: &[String],
    upgrade: bool,
    no_cache: bool,
    repo: Option<&str>,
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

    let mut cmd = Command::new(bin);
    cmd.arg("add");
    if no_cache {
        cmd.arg("--no-cache");
    }
    if let Some(r) = repo {
        cmd.args(["--repository", r]);
    }
    for p in &needs {
        cmd.arg(p.as_str());
    }

    let out = cmd.output().context("apk add")?;
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
            format!("apk add failed: {stderr}"),
        ))
    }
}

fn apk_del(bin: &str, packages: &[String], no_cache: bool, host: &str) -> Result<TaskResult> {
    let installed: Vec<&String> = packages.iter().filter(|p| is_installed(bin, p)).collect();
    if installed.is_empty() {
        return Ok(TaskResult::ok(host));
    }
    let mut cmd = Command::new(bin);
    cmd.arg("del");
    if no_cache {
        cmd.arg("--no-cache");
    }
    for p in &installed {
        cmd.arg(p.as_str());
    }
    let out = cmd.output().context("apk del")?;
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
            format!("apk del failed: {stderr}"),
        ))
    }
}

fn apk_info(bin: &str, packages: &[String], host: &str) -> Result<TaskResult> {
    let mut out_all = String::new();
    for pkg in packages {
        if let Ok(out) = Command::new(bin).args(["info", "-v", pkg]).output() {
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
    fn test_collect_packages_string() {
        let a = args(&[("name", Value::String("curl".into()))]);
        assert_eq!(collect_packages(&a), vec!["curl"]);
    }
    #[test]
    fn test_collect_packages_array() {
        let a = args(&[(
            "name",
            Value::Array(vec![
                Value::String("curl".into()),
                Value::String("wget".into()),
            ]),
        )]);
        assert_eq!(collect_packages(&a).len(), 2);
    }
    #[test]
    fn test_no_name_absent_fails() {
        let mut c = ctx();
        let a = args(&[("state", Value::String("absent".into()))]);
        let r = ApkModule.invoke(&a, "h", &mut c).unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_present_fails() {
        let mut c = ctx();
        let a = ModuleArgs::new(HashMap::new());
        let r = ApkModule.invoke(&a, "h", &mut c).unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_base_args_no_cache() {
        let v = base_args(true, None);
        assert!(v.contains(&"--no-cache"));
    }
}
