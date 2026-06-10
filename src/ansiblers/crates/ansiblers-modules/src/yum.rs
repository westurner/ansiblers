//! `yum` / `dnf` module — manage RPM packages on RHEL/Fedora/CentOS.
//!
//! Supports both the legacy `yum` binary and the newer `dnf` binary; selects
//! automatically based on availability (dnf preferred when present).
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Package name(s); single string or list |
//! | `state` | `present` | `present`, `absent`, `latest`, `installed`, `removed` |
//! | `enablerepo` | — | Additional repository to enable for this operation |
//! | `disablerepo` | — | Repository to disable for this operation |
//! | `update_cache` | `false` | Invalidate the yum/dnf cache |
//! | `security` | `false` | Only apply security-relevant updates (`update` only) |
//! | `skip_broken` | `false` | Skip packages that have broken dependencies |
//! | `autoremove` | `false` | Remove unneeded dependencies after removal |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

/// Yum / DNF module.  Registered under both `"yum"` and `"dnf"`.
pub struct YumModule;

impl ModuleInvoker for YumModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");
        let update_cache = bool_arg(args, "update_cache", false);
        let skip_broken = bool_arg(args, "skip_broken", false);
        let autoremove = bool_arg(args, "autoremove", false);
        let security = bool_arg(args, "security", false);
        let enablerepo = args.get_str("enablerepo");
        let disablerepo = args.get_str("disablerepo");

        if update_cache {
            yum_makecache()?;
        }

        if packages.is_empty() && !autoremove {
            return Ok(TaskResult::failed(
                host,
                "yum: no package names provided".to_string(),
            ));
        }

        let result = match state {
            "absent" | "removed" => yum_remove(
                &packages,
                autoremove,
                skip_broken,
                enablerepo,
                disablerepo,
                host,
            )?,
            "latest" => yum_install(
                &packages,
                true,
                security,
                skip_broken,
                enablerepo,
                disablerepo,
                host,
            )?,
            _ => {
                // present / installed
                yum_install(
                    &packages,
                    false,
                    false,
                    skip_broken,
                    enablerepo,
                    disablerepo,
                    host,
                )?
            }
        };

        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn collect_packages(args: &ModuleArgs) -> Vec<String> {
    let val = args.args.get("name").or_else(|| args.args.get("pkg"));
    match val {
        None => vec![],
        Some(ansiblers_core::Value::String(s)) => {
            s.split_whitespace().map(|s| s.to_string()).collect()
        }
        Some(ansiblers_core::Value::Array(seq)) => seq
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

/// Detect the preferred package manager (dnf > yum).
fn pkg_manager() -> &'static str {
    if which("dnf") {
        "dnf"
    } else {
        "yum"
    }
}

fn which(binary: &str) -> bool {
    Command::new("which")
        .arg(binary)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check whether an RPM package is installed.
fn is_installed(pkg: &str) -> bool {
    Command::new("rpm")
        .args(["-q", "--quiet", pkg])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check whether a package is at its latest version via dnf/yum check-update.
fn is_latest(pkg: &str) -> bool {
    // check-update exits 0 when nothing to update, 100 when updates exist.
    let status = Command::new(pkg_manager())
        .args(["check-update", pkg])
        .status()
        .unwrap_or_else(|_| std::process::ExitStatus::default());
    status.code() == Some(0)
}

fn yum_makecache() -> Result<()> {
    let status = Command::new(pkg_manager())
        .args(["makecache"])
        .status()
        .context("failed to run yum makecache")?;
    if !status.success() {
        anyhow::bail!("yum makecache failed with {status}");
    }
    Ok(())
}

fn build_base_cmd(
    subcmd: &str,
    skip_broken: bool,
    enablerepo: Option<&str>,
    disablerepo: Option<&str>,
) -> Command {
    let mut cmd = Command::new(pkg_manager());
    cmd.args(["-y", "-q", subcmd]);
    if skip_broken {
        cmd.arg("--skip-broken");
    }
    if let Some(repo) = enablerepo {
        cmd.arg(format!("--enablerepo={repo}"));
    }
    if let Some(repo) = disablerepo {
        cmd.arg(format!("--disablerepo={repo}"));
    }
    cmd
}

fn yum_install(
    pkgs: &[String],
    upgrade: bool,
    security: bool,
    skip_broken: bool,
    enablerepo: Option<&str>,
    disablerepo: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    let needs_action: Vec<&String> = if upgrade {
        pkgs.iter().filter(|p| !is_latest(p)).collect()
    } else {
        pkgs.iter().filter(|p| !is_installed(p)).collect()
    };

    if needs_action.is_empty() {
        return Ok(TaskResult::ok(host));
    }

    let subcmd = if upgrade { "upgrade" } else { "install" };
    let mut cmd = build_base_cmd(subcmd, skip_broken, enablerepo, disablerepo);
    if security && upgrade {
        cmd.arg("--security");
    }
    for pkg in &needs_action {
        cmd.arg(pkg.as_str());
    }

    let status = cmd.status().context("failed to run yum install")?;
    if status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = format!(
            "{subcmd}ed: {}",
            needs_action
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("yum {subcmd} failed with {status}"),
        ))
    }
}

fn yum_remove(
    pkgs: &[String],
    autoremove: bool,
    skip_broken: bool,
    enablerepo: Option<&str>,
    disablerepo: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    let installed: Vec<&String> = pkgs.iter().filter(|p| is_installed(p)).collect();
    if installed.is_empty() && !autoremove {
        return Ok(TaskResult::ok(host));
    }

    if !installed.is_empty() {
        let mut cmd = build_base_cmd("remove", skip_broken, enablerepo, disablerepo);
        for pkg in &installed {
            cmd.arg(pkg.as_str());
        }
        let status = cmd.status().context("failed to run yum remove")?;
        if !status.success() {
            return Ok(TaskResult::failed(
                host,
                format!("yum remove failed with {status}"),
            ));
        }
    }

    if autoremove {
        let status = Command::new(pkg_manager())
            .args(["-y", "-q", "autoremove"])
            .status()
            .context("failed to run yum autoremove")?;
        if !status.success() {
            return Ok(TaskResult::failed(
                host,
                format!("yum autoremove failed with {status}"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use ansiblers_core::Value;
    use std::collections::HashMap;

    fn make_args(pairs: &[(&str, Value)]) -> ModuleArgs {
        let mut m = HashMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.clone());
        }
        ModuleArgs::new(m)
    }

    #[test]
    fn test_collect_packages_single() {
        let args = make_args(&[("name", Value::String("bash".into()))]);
        assert_eq!(collect_packages(&args), vec!["bash"]);
    }

    #[test]
    fn test_collect_packages_list() {
        let args = make_args(&[(
            "name",
            Value::Array(vec![
                Value::String("curl".into()),
                Value::String("wget".into()),
            ]),
        )]);
        let pkgs = collect_packages(&args);
        assert!(pkgs.contains(&"curl".to_string()));
        assert!(pkgs.contains(&"wget".to_string()));
    }

    #[test]
    fn test_bool_arg_defaults() {
        let args = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&args, "update_cache", false));
        assert!(!bool_arg(&args, "skip_broken", false));
    }

    #[test]
    fn test_empty_packages_fails() {
        let mut ctx = ansiblers_core::ExecutionContext::new(
            std::sync::Arc::new(ansiblers_core::Inventory::default()),
            std::collections::HashMap::new(),
        );
        let args = ModuleArgs::new(HashMap::new());
        let result = YumModule.invoke(&args, "localhost", &mut ctx).unwrap();
        assert!(result.status.is_failed());
    }

    #[test]
    fn test_collect_packages_pkg_alias() {
        let args = make_args(&[("pkg", Value::String("bash".into()))]);
        assert_eq!(collect_packages(&args), vec!["bash"]);
    }

    #[test]
    fn test_bool_arg_override() {
        let args = make_args(&[("skip_broken", Value::Bool(true))]);
        assert!(bool_arg(&args, "skip_broken", false));
    }

    #[test]
    fn test_collect_packages_space_separated() {
        let args = make_args(&[("name", Value::String("curl wget git".into()))]);
        assert_eq!(collect_packages(&args), vec!["curl", "wget", "git"]);
    }

    #[test]
    fn test_security_default_false() {
        let args = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&args, "security", false));
    }

    #[test]
    fn test_autoremove_default_false() {
        let args = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&args, "autoremove", false));
    }
}
