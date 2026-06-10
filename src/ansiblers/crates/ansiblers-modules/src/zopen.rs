//! `zopen` module — manage packages on IBM z/OS via the
//! [z/OS Open Tools](https://zopen.community/) package manager (`zopen`).
//!
//! z/OS Open Tools provides open-source software ports for z/OS UNIX System
//! Services (USS).  The `zopen` CLI manages the installation, updating, and
//! removal of these packages in a designated `$ZOPEN_ROOTFS` directory.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Package name(s) (e.g. `git`, `vim`, `bash`) |
//! | `state` | `present` | `present`, `absent`, `latest`, `info`, `list`, `upgrade_all`, `init` |
//! | `rootfs` | — | Override `$ZOPEN_ROOTFS` environment variable |
//! | `yes` | `true` | Pass `--yes` (non-interactive) |
//! | `update_sources` | `false` | Run `zopen update` (refresh package lists) before install |
//! | `no_deps` | `false` | Pass `--no-deps` |
//! | `zopen_bin` | `zopen` | Path to the zopen binary |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct ZopenModule;

impl ModuleInvoker for ZopenModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let bin = args.get_str("zopen_bin").unwrap_or("zopen").to_string();
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");
        let rootfs = args.get_str("rootfs").map(|s| s.to_string());

        if bool_arg(args, "update_sources", false) {
            zopen_run(&bin, &["update"], rootfs.as_deref(), host)?;
        }

        match state {
            "init" => {
                let out = build_cmd(&bin, &["init"], rootfs.as_deref())
                    .output()
                    .context("zopen init")?;
                if out.status.success() {
                    let mut r = TaskResult::changed(host);
                    r.msg = "zopen environment initialized".into();
                    Ok(r)
                } else {
                    Ok(TaskResult::failed(
                        host,
                        format!(
                            "zopen init failed: {}",
                            String::from_utf8_lossy(&out.stderr)
                        ),
                    ))
                }
            }
            "upgrade_all" | "latest" if packages.is_empty() => {
                let out = build_cmd(&bin, &["upgrade"], rootfs.as_deref())
                    .output()
                    .context("zopen upgrade")?;
                let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
                if out.status.success() {
                    let changed = !stdout.contains("already up to date")
                        && !stdout.contains("nothing to upgrade");
                    let mut r = if changed {
                        TaskResult::changed(host)
                    } else {
                        TaskResult::ok(host)
                    };
                    r.stdout = stdout;
                    Ok(r)
                } else {
                    Ok(TaskResult::failed(
                        host,
                        format!("zopen upgrade failed: {stderr}"),
                    ))
                }
            }
            "list" => {
                let out = build_cmd(&bin, &["list", "--installed"], rootfs.as_deref())
                    .output()
                    .context("zopen list")?;
                let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
                let pkgs: Vec<Value> = stdout
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| Value::String(l.to_string()))
                    .collect();
                let mut r = TaskResult::ok(host);
                r.stdout = stdout;
                r.vars.insert("packages".into(), Value::Array(pkgs));
                Ok(r)
            }
            "info" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "zopen: 'name' is required for state=info".to_string(),
                    ));
                }
                let mut out_all = String::new();
                for pkg in &packages {
                    if let Ok(out) = build_cmd(&bin, &["query", pkg], rootfs.as_deref()).output() {
                        out_all.push_str(&String::from_utf8_lossy(&out.stdout));
                    }
                }
                let mut r = TaskResult::ok(host);
                r.stdout = out_all.clone();
                r.vars.insert("info".into(), Value::String(out_all));
                Ok(r)
            }
            "absent" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "zopen: 'name' is required for state=absent".to_string(),
                    ));
                }
                zopen_remove(&bin, &packages, args, rootfs.as_deref(), host)
            }
            _ => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "zopen: 'name' is required for state=present/latest".to_string(),
                    ));
                }
                zopen_install(
                    &bin,
                    &packages,
                    args,
                    state == "latest",
                    rootfs.as_deref(),
                    host,
                )
            }
        }
    }
}

fn build_cmd(bin: &str, sub_args: &[&str], rootfs: Option<&str>) -> Command {
    let mut cmd = Command::new(bin);
    if let Some(r) = rootfs {
        cmd.env("ZOPEN_ROOTFS", r);
    }
    cmd.args(sub_args);
    cmd
}

fn zopen_run(bin: &str, sub_args: &[&str], rootfs: Option<&str>, host: &str) -> Result<()> {
    let out = build_cmd(bin, sub_args, rootfs).output().context("zopen")?;
    if !out.status.success() {
        anyhow::bail!("zopen {} failed", sub_args.join(" "));
    }
    Ok(())
}

fn zopen_install(
    bin: &str,
    packages: &[String],
    args: &ModuleArgs,
    upgrade: bool,
    rootfs: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    let subcmd = if upgrade { "upgrade" } else { "install" };
    let mut cmd = build_cmd(bin, &[subcmd], rootfs);
    if bool_arg(args, "yes", true) {
        cmd.arg("--yes");
    }
    if bool_arg(args, "no_deps", false) {
        cmd.arg("--no-deps");
    }
    for p in packages {
        cmd.arg(p);
    }

    let out = cmd.output().with_context(|| format!("zopen {subcmd}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let changed = !stdout.contains("already installed") && !stdout.contains("nothing to do");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = format!("{subcmd}ed: {}", packages.join(", "));
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("zopen {subcmd} failed: {stderr}"),
        ))
    }
}

fn zopen_remove(
    bin: &str,
    packages: &[String],
    args: &ModuleArgs,
    rootfs: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, &["remove"], rootfs);
    if bool_arg(args, "yes", true) {
        cmd.arg("--yes");
    }
    for p in packages {
        cmd.arg(p);
    }

    let out = cmd.output().context("zopen remove")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = format!("removed: {}", packages.join(", "));
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("zopen remove failed: {stderr}"),
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
        let r = ZopenModule
            .invoke(&ModuleArgs::new(HashMap::new()), "h", &mut c)
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails() {
        let mut c = ctx();
        let r = ZopenModule
            .invoke(
                &args(&[("state", Value::String("absent".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_collect_packages() {
        let a = args(&[("name", Value::String("git vim".into()))]);
        assert_eq!(collect_packages(&a), vec!["git", "vim"]);
    }
    #[test]
    fn test_rootfs_env_set() {
        let mut cmd = build_cmd("zopen", &["install"], Some("/usr/zopen"));
        // Just check it builds without panic and has the right program.
        assert_eq!(cmd.get_program(), "zopen");
    }
    #[test]
    fn test_yes_default_true() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(bool_arg(&a, "yes", true));
    }
}
