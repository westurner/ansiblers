//! `pkgng` module — manage packages on FreeBSD (and DragonFly BSD) via `pkg`.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Package name(s) or origin(s) (e.g. `editors/vim`) |
//! | `state` | `present` | `present`, `absent`, `latest`, `info`, `autoremove`, `audit` |
//! | `cached` | `false` | Use cached packages; do not fetch (`-U`) |
//! | `annotation` | — | Annotation key=value to add/modify on install |
//! | `rootdir` | — | Override root directory (`-r`) |
//! | `chroot` | — | Operate inside a chroot (`-c`) |
//! | `jail` | — | Operate inside a jail (`-j`) |
//! | `update_cache` | `false` | Run `pkg update` before operation |
//! | `yes` | `true` | Pass `-y` (non-interactive) |
//! | `force` | `false` | Pass `-f` (force reinstall) |
//! | `repo` | — | Restrict to a specific repository |
//! | `pkg_bin` | `pkg` | Path to the pkg binary |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct PkgngModule;

impl ModuleInvoker for PkgngModule {
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
            pkg_update(&bin, args, host)?;
        }

        match state {
            "autoremove" => pkg_autoremove(&bin, args, host),
            "audit" => pkg_audit(&bin, args, host),
            "absent" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pkgng: 'name' is required for state=absent".to_string(),
                    ));
                }
                pkg_delete(&bin, &packages, args, host)
            }
            "latest" => {
                if packages.is_empty() {
                    return pkg_upgrade_all(&bin, args, host);
                }
                pkg_install(&bin, &packages, args, true, host)
            }
            "info" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pkgng: 'name' is required for state=info".to_string(),
                    ));
                }
                pkg_info(&bin, &packages, host)
            }
            _ => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pkgng: 'name' is required for state=present".to_string(),
                    ));
                }
                pkg_install(&bin, &packages, args, false, host)
            }
        }
    }
}

fn scope_flags(args: &ModuleArgs) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(r) = args.get_str("rootdir") {
        v.push("-r".into());
        v.push(r.to_string());
    }
    if let Some(c) = args.get_str("chroot") {
        v.push("-c".into());
        v.push(c.to_string());
    }
    if let Some(j) = args.get_str("jail") {
        v.push("-j".into());
        v.push(j.to_string());
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

fn pkg_update(bin: &str, args: &ModuleArgs, host: &str) -> Result<()> {
    let mut cmd = Command::new(bin);
    cmd.args(scope_flags(args));
    cmd.arg("update");
    if bool_arg(args, "force", false) {
        cmd.arg("-f");
    }
    let s = cmd.status().context("pkg update")?;
    if !s.success() {
        anyhow::bail!("pkg update failed");
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

    let subcmd = if upgrade { "upgrade" } else { "install" };
    let mut cmd = Command::new(bin);
    cmd.args(scope_flags(args));
    cmd.arg(subcmd);
    if bool_arg(args, "yes", true) {
        cmd.arg("-y");
    }
    if bool_arg(args, "force", false) {
        cmd.arg("-f");
    }
    if bool_arg(args, "cached", false) {
        cmd.arg("-U");
    }
    if let Some(repo) = args.get_str("repo") {
        cmd.args(["--repository", repo]);
    }
    for p in &needs {
        cmd.arg(p.as_str());
    }

    let out = cmd.output().with_context(|| format!("pkg {subcmd}"))?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = format!(
            "{subcmd}ed: {}",
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
            format!("pkg {subcmd} failed: {stderr}"),
        ))
    }
}

fn pkg_delete(bin: &str, packages: &[String], args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let installed: Vec<&String> = packages.iter().filter(|p| is_installed(bin, p)).collect();
    if installed.is_empty() {
        return Ok(TaskResult::ok(host));
    }
    let mut cmd = Command::new(bin);
    cmd.args(scope_flags(args));
    cmd.arg("delete");
    if bool_arg(args, "yes", true) {
        cmd.arg("-y");
    }
    if bool_arg(args, "force", false) {
        cmd.arg("-f");
    }
    for p in &installed {
        cmd.arg(p.as_str());
    }
    let out = cmd.output().context("pkg delete")?;
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
            format!("pkg delete failed: {stderr}"),
        ))
    }
}

fn pkg_upgrade_all(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(scope_flags(args));
    cmd.arg("upgrade");
    if bool_arg(args, "yes", true) {
        cmd.arg("-y");
    }
    if bool_arg(args, "force", false) {
        cmd.arg("-f");
    }
    let out = cmd.output().context("pkg upgrade")?;
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
            "upgraded all packages".into()
        } else {
            "already up-to-date".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pkg upgrade failed: {stderr}"),
        ))
    }
}

fn pkg_autoremove(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(scope_flags(args));
    cmd.arg("autoremove");
    if bool_arg(args, "yes", true) {
        cmd.arg("-y");
    }
    let out = cmd.output().context("pkg autoremove")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = "autoremoved orphan packages".into();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pkg autoremove failed: {stderr}"),
        ))
    }
}

fn pkg_audit(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(scope_flags(args));
    cmd.args(["audit", "-F"]);
    let out = cmd.output().context("pkg audit")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    // pkg audit exits 1 when vulnerabilities found.
    let vuln = out.status.code() == Some(1);
    let mut r = TaskResult::ok(host);
    r.stdout = stdout.clone();
    r.vars.insert("audit".into(), Value::String(stdout));
    r.vars.insert("vulnerable".into(), Value::Bool(vuln));
    if vuln {
        r.msg = "vulnerabilities found".into();
    }
    Ok(r)
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
        let r = PkgngModule
            .invoke(&ModuleArgs::new(HashMap::new()), "h", &mut c)
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails() {
        let mut c = ctx();
        let r = PkgngModule
            .invoke(
                &args(&[("state", Value::String("absent".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_scope_flags_jail() {
        let a = args(&[("jail", Value::String("myjail".into()))]);
        let flags = scope_flags(&a);
        assert!(flags.contains(&"-j".to_string()));
        assert!(flags.contains(&"myjail".to_string()));
    }
    #[test]
    fn test_scope_flags_empty() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(scope_flags(&a).is_empty());
    }
    #[test]
    fn test_yes_default_true() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(bool_arg(&a, "yes", true));
    }
    #[test]
    fn test_collect_origin() {
        let a = args(&[("name", Value::String("editors/vim".into()))]);
        assert_eq!(collect_packages(&a), vec!["editors/vim"]);
    }
}
