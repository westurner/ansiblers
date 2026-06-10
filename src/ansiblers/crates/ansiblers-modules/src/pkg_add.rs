//! `pkg_add` module — manage packages on OpenBSD via `pkg_add` / `pkg_delete`.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Package name(s), optionally with flavour (`vim--python3`) |
//! | `state` | `present` | `present`, `absent`, `latest`, `info` |
//! | `update_cache` | `false` | Run `pkg_add -u` (update all) before install |
//! | `verbose` | `false` | Pass `-v` |
//! | `force` | `false` | Pass `-F` (force install even if already installed) |
//! | `replace` | `false` | Pass `-r` (replace existing package version) |
//! | `install_path` | — | Override `PKG_PATH` environment variable |
//! | `arch` | — | Override `PKG_ARCH` |
//! | `pkg_add_bin` | `pkg_add` | Path to `pkg_add` |
//! | `pkg_delete_bin` | `pkg_delete` | Path to `pkg_delete` |
//! | `pkg_info_bin` | `pkg_info` | Path to `pkg_info` |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct PkgAddModule;

impl ModuleInvoker for PkgAddModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");
        let verbose = bool_arg(args, "verbose", false);
        let force = bool_arg(args, "force", false);
        let replace = bool_arg(args, "replace", false);
        let install_path = args.get_str("install_path").map(|s| s.to_string());
        let arch = args.get_str("arch").map(|s| s.to_string());

        if bool_arg(args, "update_cache", false) {
            let mut cmd = Command::new(args.get_str("pkg_add_bin").unwrap_or("pkg_add"));
            cmd.arg("-u");
            if verbose {
                cmd.arg("-v");
            }
            apply_env(&mut cmd, install_path.as_deref(), arch.as_deref());
            let out = cmd.output().context("pkg_add -u")?;
            if !out.status.success() {
                return Ok(TaskResult::failed(
                    host,
                    format!(
                        "pkg_add -u failed: {}",
                        String::from_utf8_lossy(&out.stderr)
                    ),
                ));
            }
        }

        match state {
            "absent" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pkg_add: 'name' is required for state=absent".to_string(),
                    ));
                }
                pkg_delete(&packages, args, verbose, host)
            }
            "latest" => {
                if packages.is_empty() {
                    return pkg_add_update_all(
                        args,
                        verbose,
                        install_path.as_deref(),
                        arch.as_deref(),
                        host,
                    );
                }
                pkg_add_install(
                    &packages,
                    args,
                    true,
                    verbose,
                    force,
                    replace,
                    install_path.as_deref(),
                    arch.as_deref(),
                    host,
                )
            }
            "info" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pkg_add: 'name' is required for state=info".to_string(),
                    ));
                }
                pkg_info(&packages, args, host)
            }
            _ => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pkg_add: 'name' is required for state=present".to_string(),
                    ));
                }
                pkg_add_install(
                    &packages,
                    args,
                    false,
                    verbose,
                    force,
                    replace,
                    install_path.as_deref(),
                    arch.as_deref(),
                    host,
                )
            }
        }
    }
}

fn apply_env(cmd: &mut Command, install_path: Option<&str>, arch: Option<&str>) {
    if let Some(p) = install_path {
        cmd.env("PKG_PATH", p);
    }
    if let Some(a) = arch {
        cmd.env("PKG_ARCH", a);
    }
}

fn is_installed(bin: &str, pkg: &str) -> bool {
    // Strip flavour suffix for the check: vim--python3 → vim
    let bare = pkg.split("--").next().unwrap_or(pkg);
    Command::new(bin)
        .args(["-e", bare])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn pkg_add_install(
    packages: &[String],
    args: &ModuleArgs,
    upgrade: bool,
    verbose: bool,
    force: bool,
    replace: bool,
    install_path: Option<&str>,
    arch: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    let bin = args.get_str("pkg_add_bin").unwrap_or("pkg_add").to_string();
    let info_bin = args
        .get_str("pkg_info_bin")
        .unwrap_or("pkg_info")
        .to_string();

    let needs: Vec<&String> = if upgrade {
        packages.iter().collect()
    } else {
        packages
            .iter()
            .filter(|p| !is_installed(&info_bin, p))
            .collect()
    };
    if needs.is_empty() {
        return Ok(TaskResult::ok(host));
    }

    let mut cmd = Command::new(&bin);
    if verbose {
        cmd.arg("-v");
    }
    if force {
        cmd.arg("-F");
    }
    if replace {
        cmd.arg("-r");
    }
    if upgrade {
        cmd.arg("-u");
    }
    apply_env(&mut cmd, install_path, arch);
    for p in &needs {
        cmd.arg(p.as_str());
    }

    let out = cmd.output().context("pkg_add")?;
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
            format!("pkg_add failed: {stderr}"),
        ))
    }
}

fn pkg_add_update_all(
    args: &ModuleArgs,
    verbose: bool,
    install_path: Option<&str>,
    arch: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    let bin = args.get_str("pkg_add_bin").unwrap_or("pkg_add").to_string();
    let mut cmd = Command::new(&bin);
    cmd.arg("-u");
    if verbose {
        cmd.arg("-v");
    }
    apply_env(&mut cmd, install_path, arch);
    let out = cmd.output().context("pkg_add -u")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = "all packages updated".into();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pkg_add -u failed: {stderr}"),
        ))
    }
}

fn pkg_delete(
    packages: &[String],
    args: &ModuleArgs,
    verbose: bool,
    host: &str,
) -> Result<TaskResult> {
    let bin = args
        .get_str("pkg_delete_bin")
        .unwrap_or("pkg_delete")
        .to_string();
    let info_bin = args
        .get_str("pkg_info_bin")
        .unwrap_or("pkg_info")
        .to_string();
    let installed: Vec<&String> = packages
        .iter()
        .filter(|p| is_installed(&info_bin, p))
        .collect();
    if installed.is_empty() {
        return Ok(TaskResult::ok(host));
    }

    let mut cmd = Command::new(&bin);
    if verbose {
        cmd.arg("-v");
    }
    if bool_arg(args, "force", false) {
        cmd.arg("-F");
    }
    for p in &installed {
        cmd.arg(p.as_str());
    }
    let out = cmd.output().context("pkg_delete")?;
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
            format!("pkg_delete failed: {stderr}"),
        ))
    }
}

fn pkg_info(packages: &[String], args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let bin = args
        .get_str("pkg_info_bin")
        .unwrap_or("pkg_info")
        .to_string();
    let mut out_all = String::new();
    for pkg in packages {
        if let Ok(out) = Command::new(&bin).arg(pkg).output() {
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
        let r = PkgAddModule
            .invoke(&ModuleArgs::new(HashMap::new()), "h", &mut c)
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails() {
        let mut c = ctx();
        let r = PkgAddModule
            .invoke(
                &args(&[("state", Value::String("absent".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_flavour_strip_for_check() {
        // vim--python3 → bare name is "vim"
        let bare = "vim--python3".split("--").next().unwrap_or("vim--python3");
        assert_eq!(bare, "vim");
    }
    #[test]
    fn test_collect_packages_string() {
        let a = args(&[("name", Value::String("vim--python3".into()))]);
        assert_eq!(collect_packages(&a), vec!["vim--python3"]);
    }
    #[test]
    fn test_verbose_default_false() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "verbose", false));
    }

    #[test]
    fn test_replace_default_false() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "replace", false));
    }

    #[test]
    fn test_collect_packages_array() {
        let a = args(&[(
            "name",
            Value::Array(vec![
                Value::String("vim--python3".into()),
                Value::String("curl".into()),
            ]),
        )]);
        assert_eq!(collect_packages(&a).len(), 2);
    }

    #[test]
    fn test_no_name_info_fails() {
        let mut c = ctx();
        let r = PkgAddModule
            .invoke(
                &args(&[("state", Value::String("info".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }

    #[test]
    fn test_pkg_alias_accepted() {
        let a = args(&[("pkg", Value::String("curl".into()))]);
        assert_eq!(collect_packages(&a), vec!["curl"]);
    }
}
