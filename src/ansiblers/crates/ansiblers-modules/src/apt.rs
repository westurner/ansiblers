//! `apt` module — manage Debian/Ubuntu packages via `apt-get` / `dpkg`.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` / `package` | — | Package name(s); single string or list |
//! | `state` | `present` | `present`, `absent`, `latest`, `build-dep` |
//! | `update_cache` | `false` | Run `apt-get update` before installing |
//! | `cache_valid_time` | `0` | Skip update if cache is fresher than N seconds |
//! | `purge` | `false` | `--purge` on removal |
//! | `autoremove` | `false` | Run `apt-get autoremove` after the operation |
//! | `install_recommends` | `true` | Pass `--no-install-recommends` when `false` |
//! | `force_apt_get` | `false` | Prefer `apt-get` over `apt` CLI |
//! | `dpkg_options` | — | Extra options forwarded to dpkg |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct AptModule;

impl ModuleInvoker for AptModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        // Collect package names (single string or JSON array stored as Value::Sequence).
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");
        let update_cache = bool_arg(args, "update_cache", false);
        let cache_valid_time = args
            .args
            .get("cache_valid_time")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let purge = bool_arg(args, "purge", false);
        let autoremove = bool_arg(args, "autoremove", false);
        let install_recommends = bool_arg(args, "install_recommends", true);

        // ------------------------------------------------------------------
        // Optional cache update
        // ------------------------------------------------------------------
        if update_cache {
            if cache_valid_time > 0 {
                // Check mtime of apt lists directory.
                let lists_dir = std::path::Path::new("/var/lib/apt/lists");
                if let Ok(meta) = lists_dir.metadata() {
                    if let Ok(modified) = meta.modified() {
                        if let Ok(age) = modified.elapsed() {
                            if age.as_secs() < cache_valid_time {
                                tracing::debug!(
                                    "apt: cache valid ({} s < {cache_valid_time} s), skipping update",
                                    age.as_secs()
                                );
                                // Skip update — fall through to install.
                            } else {
                                apt_get_update()?;
                            }
                        } else {
                            apt_get_update()?;
                        }
                    } else {
                        apt_get_update()?;
                    }
                } else {
                    apt_get_update()?;
                }
            } else {
                apt_get_update()?;
            }
        }

        // ------------------------------------------------------------------
        // autoremove only (no packages required)
        // ------------------------------------------------------------------
        if autoremove && packages.is_empty() {
            return run_autoremove(host);
        }

        if packages.is_empty() && !autoremove {
            return Ok(TaskResult::failed(host, "apt: no package names provided".to_string()));
        }

        // ------------------------------------------------------------------
        // Per-state logic
        // ------------------------------------------------------------------
        let result = match state {
            "absent" | "purged" => {
                let actually_purge = purge || state == "purged";
                apt_remove(&packages, actually_purge, host)?
            }
            "latest" => apt_install(&packages, true, install_recommends, host)?,
            "build-dep" => apt_build_dep(&packages, host)?,
            _ => {
                // present
                apt_install(&packages, false, install_recommends, host)?
            }
        };

        // Post-install autoremove.
        if autoremove && !packages.is_empty() {
            run_autoremove(host)?;
        }

        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn collect_packages(args: &ModuleArgs) -> Vec<String> {
    let val = args
        .args
        .get("name")
        .or_else(|| args.args.get("pkg"))
        .or_else(|| args.args.get("package"));

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
    args.args.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

/// Check whether a package is currently installed via `dpkg-query`.
fn is_installed(pkg: &str) -> bool {
    Command::new("dpkg-query")
        .args(["-W", "-f=${Status}", pkg])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("install ok installed"))
        .unwrap_or(false)
}

/// Check whether a package is at its latest available version.
fn is_latest(pkg: &str) -> bool {
    // `apt-get --simulate upgrade <pkg>` returns 0 and prints nothing if already latest.
    Command::new("apt-get")
        .args(["-s", "upgrade", pkg])
        .output()
        .map(|o| {
            let out = String::from_utf8_lossy(&o.stdout);
            // If no new version available the simulated output won't contain "Inst".
            !out.contains(&format!("Inst {pkg}"))
        })
        .unwrap_or(false)
}

fn apt_get_update() -> Result<()> {
    let status = Command::new("apt-get")
        .args(["-q", "update"])
        .status()
        .context("failed to run apt-get update")?;
    if !status.success() {
        anyhow::bail!("apt-get update failed with {status}");
    }
    Ok(())
}

fn apt_install(pkgs: &[String], upgrade: bool, recommends: bool, host: &str) -> Result<TaskResult> {
    // Check which packages actually need to be installed/upgraded.
    let needs_action: Vec<&String> = if upgrade {
        pkgs.iter().filter(|p| !is_latest(p)).collect()
    } else {
        pkgs.iter().filter(|p| !is_installed(p)).collect()
    };

    if needs_action.is_empty() {
        return Ok(TaskResult::ok(host));
    }

    let mut cmd = Command::new("apt-get");
    cmd.args(["-y", "-q"]);
    if !recommends {
        cmd.arg("--no-install-recommends");
    }
    cmd.arg(if upgrade { "install" } else { "install" });
    for pkg in &needs_action {
        cmd.arg(pkg.as_str());
    }

    let status = cmd.status().context("failed to run apt-get install")?;
    if status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = format!("installed: {}", needs_action.iter().map(|p| p.as_str()).collect::<Vec<_>>().join(", "));
        Ok(r)
    } else {
        Ok(TaskResult::failed(host, format!("apt-get install failed with {status}")))
    }
}

fn apt_remove(pkgs: &[String], purge: bool, host: &str) -> Result<TaskResult> {
    let installed: Vec<&String> = pkgs.iter().filter(|p| is_installed(p)).collect();
    if installed.is_empty() {
        return Ok(TaskResult::ok(host));
    }

    let mut cmd = Command::new("apt-get");
    cmd.args(["-y", "-q"]);
    cmd.arg(if purge { "purge" } else { "remove" });
    for pkg in &installed {
        cmd.arg(pkg.as_str());
    }

    let status = cmd.status().context("failed to run apt-get remove")?;
    if status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = format!("removed: {}", installed.iter().map(|p| p.as_str()).collect::<Vec<_>>().join(", "));
        Ok(r)
    } else {
        Ok(TaskResult::failed(host, format!("apt-get remove failed with {status}")))
    }
}

fn apt_build_dep(pkgs: &[String], host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new("apt-get");
    cmd.args(["-y", "-q", "build-dep"]);
    for pkg in pkgs {
        cmd.arg(pkg.as_str());
    }
    let status = cmd.status().context("failed to run apt-get build-dep")?;
    if status.success() {
        Ok(TaskResult::changed(host))
    } else {
        Ok(TaskResult::failed(host, format!("apt-get build-dep failed with {status}")))
    }
}

fn run_autoremove(host: &str) -> Result<TaskResult> {
    let status = Command::new("apt-get")
        .args(["-y", "-q", "autoremove"])
        .status()
        .context("failed to run apt-get autoremove")?;
    if status.success() {
        Ok(TaskResult::changed(host))
    } else {
        Ok(TaskResult::failed(host, format!("apt-get autoremove failed with {status}")))
    }
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
    fn test_collect_packages_single_string() {
        let args = make_args(&[("name", Value::String("curl".into()))]);
        assert_eq!(collect_packages(&args), vec!["curl"]);
    }

    #[test]
    fn test_collect_packages_sequence() {
        let args = make_args(&[(
            "name",
            Value::Array(vec![
                Value::String("curl".into()),
                Value::String("wget".into()),
            ]),
        )]);
        assert_eq!(collect_packages(&args), vec!["curl", "wget"]);
    }

    #[test]
    fn test_collect_packages_empty() {
        let args = ModuleArgs::new(HashMap::new());
        assert!(collect_packages(&args).is_empty());
    }

    #[test]
    fn test_bool_arg_default() {
        let args = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&args, "update_cache", false));
        assert!(bool_arg(&args, "install_recommends", true));
    }

    #[test]
    fn test_bool_arg_override() {
        let args = make_args(&[("update_cache", Value::Bool(true))]);
        assert!(bool_arg(&args, "update_cache", false));
    }

    #[test]
    fn test_collect_packages_pkg_alias() {
        let args = make_args(&[("pkg", Value::String("git".into()))]);
        assert_eq!(collect_packages(&args), vec!["git"]);
    }
}
