//! `nix` module — manage packages and profiles via `nix` / `nix-env`, with
//! optional support for GNU Guix as a compatible fallback.
//!
//! Supports both legacy `nix-env` commands and the modern Nix flakes CLI
//! (`nix profile install`, `nix build`, etc.).
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Attribute path(s) or flake refs |
//! | `state` | `present` | `present`, `absent`, `latest`, `info`, `gc`, `search`, `build`, `develop`, `flake_update` |
//! | `profile` | — | Profile path or name |
//! | `channel` | — | Nix channel name (used with legacy nix-env: `<nixpkgs>`) |
//! | `attr` | — | Attribute path shorthand (e.g. `nixpkgs.git`) |
//! | `flake` | — | Flake reference (e.g. `nixpkgs#git`); enables flake mode |
//! | `impure` | `false` | Pass `--impure` in flake mode |
//! | `no_sandbox` | `false` | Pass `--no-sandbox` |
//! | `extra_args` | — | Extra flags forwarded verbatim |
//! | `nix_bin` | `nix` | Path to the nix binary |
//! | `nix_env_bin` | `nix-env` | Path to legacy nix-env binary |
//! | `use_guix` | `false` | Use `guix` CLI instead of `nix` for all operations |
//! | `guix_bin` | `guix` | Path to the guix binary |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct NixModule;

impl ModuleInvoker for NixModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        // Guix mode: delegate all operations to guix CLI.
        if bool_arg(args, "use_guix", false) {
            return guix_dispatch(args, host);
        }

        let state = args.get_str("state").unwrap_or("present");
        let flake_mode = args.get_str("flake").is_some();
        let packages = collect_packages(args);

        match state {
            "gc" => nix_gc(args, host),
            "search" => nix_search(args, host, ctx),
            "build" => nix_build(args, host),
            "develop" => nix_develop(args, host),
            "flake_update" => nix_flake_update(args, host),
            "info" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "nix: 'name' is required for state=info".to_string(),
                    ));
                }
                nix_info(&packages, args, host)
            }
            "absent" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "nix: 'name' is required for state=absent".to_string(),
                    ));
                }
                if flake_mode {
                    nix_profile_remove(&packages, args, host)
                } else {
                    nix_env_uninstall(&packages, args, host)
                }
            }
            "latest" => {
                if packages.is_empty() {
                    return nix_env_upgrade_all(args, host);
                }
                if flake_mode {
                    nix_profile_install(&packages, args, host)
                } else {
                    nix_env_install(&packages, args, host)
                }
            }
            _ => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "nix: 'name' or 'flake' is required for state=present".to_string(),
                    ));
                }
                if flake_mode {
                    nix_profile_install(&packages, args, host)
                } else {
                    nix_env_install(&packages, args, host)
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Modern Nix flakes profile commands
// ---------------------------------------------------------------------------

fn nix_bin(args: &ModuleArgs) -> String {
    args.get_str("nix_bin").unwrap_or("nix").to_string()
}

fn nix_env_bin(args: &ModuleArgs) -> String {
    args.get_str("nix_env_bin").unwrap_or("nix-env").to_string()
}

fn common_nix_flags(args: &ModuleArgs) -> Vec<String> {
    let mut v = Vec::new();
    if bool_arg(args, "impure", false) {
        v.push("--impure".to_string());
    }
    if bool_arg(args, "no_sandbox", false) {
        v.push("--no-sandbox".to_string());
    }
    if let Some(e) = args.get_str("extra_args") {
        v.extend(e.split_whitespace().map(|s| s.to_string()));
    }
    v
}

fn nix_profile_install(packages: &[String], args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let flake = args.get_str("flake").map(|s| s.to_string());
    let targets: Vec<String> = if let Some(f) = flake {
        vec![f]
    } else {
        packages
            .iter()
            .map(|p| {
                if p.contains('#') {
                    p.clone()
                } else {
                    format!("nixpkgs#{p}")
                }
            })
            .collect()
    };

    let mut cmd = Command::new(nix_bin(args));
    cmd.args(["profile", "install"]);
    cmd.args(common_nix_flags(args));
    if let Some(profile) = args.get_str("profile") {
        cmd.args(["--profile", profile]);
    }
    for t in &targets {
        cmd.arg(t);
    }

    let out = cmd.output().context("nix profile install")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = format!("nix profile installed: {}", targets.join(", "));
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("nix profile install failed: {stderr}"),
        ))
    }
}

fn nix_profile_remove(packages: &[String], args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(nix_bin(args));
    cmd.args(["profile", "remove"]);
    if let Some(profile) = args.get_str("profile") {
        cmd.args(["--profile", profile]);
    }
    for p in packages {
        cmd.arg(p);
    }
    let out = cmd.output().context("nix profile remove")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = format!("nix profile removed: {}", packages.join(", "));
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("nix profile remove failed: {stderr}"),
        ))
    }
}

fn nix_build(args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let flake = args.get_str("flake").unwrap_or(".");
    let mut cmd = Command::new(nix_bin(args));
    cmd.arg("build");
    cmd.args(common_nix_flags(args));
    cmd.arg(flake);
    let out = cmd.output().context("nix build")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = format!("nix build '{flake}' completed");
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("nix build failed: {stderr}"),
        ))
    }
}

fn nix_develop(args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let flake = args.get_str("flake").unwrap_or(".");
    let cmd_arg = args.get_str("command").map(|s| s.to_string());
    let mut cmd = Command::new(nix_bin(args));
    cmd.arg("develop");
    cmd.args(common_nix_flags(args));
    cmd.arg(flake);
    if let Some(c) = &cmd_arg {
        cmd.args(["--command", c]);
    }
    let out = cmd.output().context("nix develop")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::ok(host);
        r.stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("nix develop failed: {stderr}"),
        ))
    }
}

fn nix_flake_update(args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let flake_dir = args.get_str("flake").unwrap_or(".");
    let mut cmd = Command::new(nix_bin(args));
    cmd.args(["flake", "update", flake_dir]);
    let out = cmd.output().context("nix flake update")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = "flake inputs updated".into();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("nix flake update failed: {stderr}"),
        ))
    }
}

fn nix_gc(args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(nix_bin(args));
    cmd.args(["store", "gc"]);
    cmd.args(common_nix_flags(args));
    let out = cmd.output().context("nix store gc")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = "nix store GC completed".into();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("nix store gc failed: {stderr}"),
        ))
    }
}

fn nix_search(args: &ModuleArgs, host: &str, ctx: &mut ExecutionContext) -> Result<TaskResult> {
    let query = args
        .get_str("name")
        .or_else(|| args.get_str("pkg"))
        .unwrap_or("");
    let mut cmd = Command::new(nix_bin(args));
    cmd.args(["search", "nixpkgs", "--json", query]);
    let out = cmd.output().context("nix search")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let results: Value = serde_json::from_str(&stdout).unwrap_or(Value::String(stdout.clone()));
    ctx.set_fact(host, "nix_search_results".into(), results.clone());
    let mut r = TaskResult::ok(host);
    r.stdout = stdout;
    r.vars.insert("results".into(), results);
    Ok(r)
}

fn nix_info(packages: &[String], args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut out_all = String::new();
    for pkg in packages {
        if let Ok(out) = Command::new(nix_env_bin(args))
            .args(["-qaA", pkg, "--description"])
            .output()
        {
            out_all.push_str(&String::from_utf8_lossy(&out.stdout));
        }
    }
    let mut r = TaskResult::ok(host);
    r.stdout = out_all.clone();
    r.vars.insert("info".into(), Value::String(out_all));
    Ok(r)
}

// ---------------------------------------------------------------------------
// Legacy nix-env commands
// ---------------------------------------------------------------------------

fn nix_env_install(packages: &[String], args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let attr = args.get_str("attr").map(|s| s.to_string());
    let mut cmd = Command::new(nix_env_bin(args));
    if attr.is_some() {
        cmd.arg("-iA");
        for p in packages {
            cmd.arg(p);
        }
    } else {
        cmd.arg("-i");
        for p in packages {
            cmd.arg(p);
        }
    }
    if let Some(e) = args.get_str("extra_args") {
        cmd.args(e.split_whitespace());
    }
    let out = cmd.output().context("nix-env -i")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = format!("nix-env installed: {}", packages.join(", "));
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("nix-env install failed: {stderr}"),
        ))
    }
}

fn nix_env_uninstall(packages: &[String], args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(nix_env_bin(args));
    cmd.arg("-e");
    for p in packages {
        cmd.arg(p);
    }
    let out = cmd.output().context("nix-env -e")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = format!("nix-env uninstalled: {}", packages.join(", "));
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("nix-env uninstall failed: {stderr}"),
        ))
    }
}

fn nix_env_upgrade_all(args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(nix_env_bin(args));
    cmd.arg("-u");
    let out = cmd.output().context("nix-env -u")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = "nix-env upgraded all packages".into();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("nix-env upgrade failed: {stderr}"),
        ))
    }
}

// ---------------------------------------------------------------------------
// GNU Guix mode
// ---------------------------------------------------------------------------

fn guix_dispatch(args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let guix_bin = args.get_str("guix_bin").unwrap_or("guix").to_string();
    let packages = collect_packages(args);
    let state = args.get_str("state").unwrap_or("present");

    match state {
        "gc" => {
            let out = Command::new(&guix_bin)
                .args(["gc"])
                .output()
                .context("guix gc")?;
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            if out.status.success() {
                let mut r = TaskResult::changed(host);
                r.msg = "guix gc completed".into();
                Ok(r)
            } else {
                Ok(TaskResult::failed(
                    host,
                    format!("guix gc failed: {stderr}"),
                ))
            }
        }
        "absent" => {
            if packages.is_empty() {
                return Ok(TaskResult::failed(
                    host,
                    "guix: 'name' required for state=absent".to_string(),
                ));
            }
            let mut cmd = Command::new(&guix_bin);
            cmd.arg("remove");
            for p in &packages {
                cmd.arg(p);
            }
            let out = cmd.output().context("guix remove")?;
            if out.status.success() {
                let mut r = TaskResult::changed(host);
                r.msg = format!("guix removed: {}", packages.join(", "));
                Ok(r)
            } else {
                Ok(TaskResult::failed(host, "guix remove failed".to_string()))
            }
        }
        _ => {
            if packages.is_empty() {
                return Ok(TaskResult::failed(
                    host,
                    "guix: 'name' required".to_string(),
                ));
            }
            let mut cmd = Command::new(&guix_bin);
            cmd.arg("install");
            for p in &packages {
                cmd.arg(p);
            }
            let out = cmd.output().context("guix install")?;
            if out.status.success() {
                let mut r = TaskResult::changed(host);
                r.msg = format!("guix installed: {}", packages.join(", "));
                Ok(r)
            } else {
                Ok(TaskResult::failed(host, "guix install failed".to_string()))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Common helpers
// ---------------------------------------------------------------------------

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
        let r = NixModule
            .invoke(&ModuleArgs::new(HashMap::new()), "h", &mut c)
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails() {
        let mut c = ctx();
        let r = NixModule
            .invoke(
                &args(&[("state", Value::String("absent".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_common_nix_flags_impure() {
        let a = args(&[("impure", Value::Bool(true))]);
        assert!(common_nix_flags(&a).contains(&"--impure".to_string()));
    }
    #[test]
    fn test_flake_mode_detected() {
        let a = args(&[("flake", Value::String("nixpkgs#git".into()))]);
        assert!(a.get_str("flake").is_some());
    }
    #[test]
    fn test_guix_no_name_fails() {
        let mut c = ctx();
        let a = args(&[("use_guix", Value::Bool(true))]);
        let r = NixModule.invoke(&a, "h", &mut c).unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_nix_env_bin_default() {
        let a = ModuleArgs::new(HashMap::new());
        assert_eq!(nix_env_bin(&a), "nix-env");
    }
}
