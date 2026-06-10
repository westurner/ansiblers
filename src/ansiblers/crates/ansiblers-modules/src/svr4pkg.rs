//! `svr4pkg` module — manage SVR4 packages on Oracle Solaris 10 and earlier
//! (and compatible systems) via `pkgadd` / `pkgrm` / `pkginfo` / `pkgchk`.
//!
//! SVR4 packages use a `VENDOR.PKGNAME` naming convention (e.g. `SUNWcsr`).
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Package name(s) (e.g. `SUNWcsr`) |
//! | `state` | `present` | `present`, `absent`, `info`, `check` |
//! | `spool` | — | Spool directory or device path for `pkgadd` |
//! | `category` | — | Filter by package category |
//! | `admin_file` | `/tmp/ansiblers-pkgadd.admin` | Admin file for non-interactive install |
//! | `response_file` | — | Response file for interactive packages |
//! | `root_path` | — | Alternate root directory (`-R`) |
//! | `force` | `false` | Pass `-f` to pkgrm |
//! | `pkgadd_bin` | `pkgadd` | Path to pkgadd |
//! | `pkgrm_bin` | `pkgrm` | Path to pkgrm |
//! | `pkginfo_bin` | `pkginfo` | Path to pkginfo |
//! | `pkgchk_bin` | `pkgchk` | Path to pkgchk |

use std::path::Path;
use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

/// Non-interactive admin file content — suitable for automation.
const DEFAULT_ADMIN: &str = "\
mail=\n\
instance=overwrite\n\
partial=nocheck\n\
runlevel=nocheck\n\
idepend=nocheck\n\
rdepend=nocheck\n\
space=ask\n\
setuid=nocheck\n\
conflict=nocheck\n\
action=nocheck\n\
networktimeout=60\n\
networkretries=3\n\
authentication=quit\n\
keystore=/var/sadm/security\n\
proxy=\n\
basedir=default\n";

pub struct Svr4PkgModule;

impl ModuleInvoker for Svr4PkgModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");

        match state {
            "absent" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "svr4pkg: 'name' is required for state=absent".to_string(),
                    ));
                }
                svr4_remove(&packages, args, host)
            }
            "info" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "svr4pkg: 'name' is required for state=info".to_string(),
                    ));
                }
                svr4_info(&packages, args, host)
            }
            "check" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "svr4pkg: 'name' is required for state=check".to_string(),
                    ));
                }
                svr4_check(&packages, args, host)
            }
            _ => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "svr4pkg: 'name' is required for state=present".to_string(),
                    ));
                }
                svr4_install(&packages, args, host)
            }
        }
    }
}

fn is_installed(bin: &str, pkg: &str) -> bool {
    Command::new(bin)
        .args(["-q", pkg])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn ensure_admin_file(args: &ModuleArgs) -> Result<String> {
    let path = args
        .get_str("admin_file")
        .unwrap_or("/tmp/ansiblers-pkgadd.admin");
    if !Path::new(path).exists() {
        std::fs::write(path, DEFAULT_ADMIN)?;
    }
    Ok(path.to_string())
}

fn svr4_install(packages: &[String], args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let pkginfo_bin = args.get_str("pkginfo_bin").unwrap_or("pkginfo").to_string();
    let needs: Vec<&String> = packages
        .iter()
        .filter(|p| !is_installed(&pkginfo_bin, p))
        .collect();
    if needs.is_empty() {
        return Ok(TaskResult::ok(host));
    }

    let admin = ensure_admin_file(args)?;
    let pkgadd_bin = args.get_str("pkgadd_bin").unwrap_or("pkgadd").to_string();

    for pkg in &needs {
        let mut cmd = Command::new(&pkgadd_bin);
        cmd.args(["-a", &admin, "-n"]);
        if let Some(r) = args.get_str("root_path") {
            cmd.args(["-R", r]);
        }
        if let Some(resp) = args.get_str("response_file") {
            cmd.args(["-r", resp]);
        }
        if let Some(d) = args.get_str("spool") {
            cmd.args(["-d", d]);
        } else {
            cmd.arg("-d");
            cmd.arg("ask");
        }
        cmd.arg("all");
        cmd.arg(pkg.as_str());

        let out = cmd.output().context("pkgadd")?;
        if !out.status.success() {
            return Ok(TaskResult::failed(
                host,
                format!(
                    "pkgadd '{}' failed: {}",
                    pkg,
                    String::from_utf8_lossy(&out.stderr)
                ),
            ));
        }
    }
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
}

fn svr4_remove(packages: &[String], args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let pkginfo_bin = args.get_str("pkginfo_bin").unwrap_or("pkginfo").to_string();
    let installed: Vec<&String> = packages
        .iter()
        .filter(|p| is_installed(&pkginfo_bin, p))
        .collect();
    if installed.is_empty() {
        return Ok(TaskResult::ok(host));
    }

    let pkgrm_bin = args.get_str("pkgrm_bin").unwrap_or("pkgrm").to_string();
    for pkg in &installed {
        let mut cmd = Command::new(&pkgrm_bin);
        cmd.arg("-n");
        if bool_arg(args, "force", false) {
            cmd.arg("-f");
        }
        if let Some(r) = args.get_str("root_path") {
            cmd.args(["-R", r]);
        }
        cmd.arg(pkg.as_str());
        let out = cmd.output().context("pkgrm")?;
        if !out.status.success() {
            return Ok(TaskResult::failed(
                host,
                format!(
                    "pkgrm '{}' failed: {}",
                    pkg,
                    String::from_utf8_lossy(&out.stderr)
                ),
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

fn svr4_info(packages: &[String], args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let bin = args.get_str("pkginfo_bin").unwrap_or("pkginfo").to_string();
    let mut out_all = String::new();
    for pkg in packages {
        if let Ok(out) = Command::new(&bin).args(["-l", pkg]).output() {
            out_all.push_str(&String::from_utf8_lossy(&out.stdout));
        }
    }
    let mut r = TaskResult::ok(host);
    r.stdout = out_all.clone();
    r.vars.insert("info".into(), Value::String(out_all));
    Ok(r)
}

fn svr4_check(packages: &[String], args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let bin = args.get_str("pkgchk_bin").unwrap_or("pkgchk").to_string();
    let mut cmd = Command::new(&bin);
    if let Some(r) = args.get_str("root_path") {
        cmd.args(["-R", r]);
    }
    for p in packages {
        cmd.arg(p);
    }
    let out = cmd.output().context("pkgchk")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let errors = !out.status.success();
    let mut r = if errors {
        TaskResult::failed(host, format!("pkgchk found errors: {stdout}"))
    } else {
        TaskResult::ok(host)
    };
    r.stdout = stdout;
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
        let r = Svr4PkgModule
            .invoke(&ModuleArgs::new(HashMap::new()), "h", &mut c)
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails() {
        let mut c = ctx();
        let r = Svr4PkgModule
            .invoke(
                &args(&[("state", Value::String("absent".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_collect_sunw_package() {
        let a = args(&[("name", Value::String("SUNWcsr".into()))]);
        assert_eq!(collect_packages(&a), vec!["SUNWcsr"]);
    }
    #[test]
    fn test_default_admin_content() {
        assert!(DEFAULT_ADMIN.contains("instance=overwrite"));
        assert!(DEFAULT_ADMIN.contains("partial=nocheck"));
    }
    #[test]
    fn test_force_default_false() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "force", false));
    }

    #[test]
    fn test_collect_packages_array() {
        let a = args(&[(
            "name",
            Value::Array(vec![
                Value::String("SUNWcsr".into()),
                Value::String("SUNWcsu".into()),
            ]),
        )]);
        assert_eq!(collect_packages(&a).len(), 2);
    }

    #[test]
    fn test_collect_packages_pkg_alias() {
        let a = args(&[("pkg", Value::String("SUNWcsr".into()))]);
        assert_eq!(collect_packages(&a), vec!["SUNWcsr"]);
    }

    #[test]
    fn test_no_name_info_fails() {
        let mut c = ctx();
        let r = Svr4PkgModule
            .invoke(
                &args(&[("state", Value::String("info".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }

    #[test]
    fn test_no_name_check_fails() {
        let mut c = ctx();
        let r = Svr4PkgModule
            .invoke(
                &args(&[("state", Value::String("check".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }
}
