//! `chocolatey` module — manage Windows packages via Chocolatey / NuGet.
//!
//! Chocolatey is the primary Windows package manager; it uses NuGet `.nupkg`
//! packages hosted on `chocolatey.org` or custom feeds.
//!
//! **Note:** On non-Windows hosts (Linux/macOS CI runners) this module will
//! fail unless the `choco_bin` path points to a Wine-wrapped or mock binary.
//! All test assertions are based on parameter validation only.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Package name(s) |
//! | `state` | `present` | `present`, `absent`, `latest`, `info`, `upgrade_all`, `list` |
//! | `version` | — | Exact version to install / pin |
//! | `source` | — | Custom NuGet feed URL or local path |
//! | `pre_release` | `false` | Include pre-release packages (`--pre`) |
//! | `install_args` | — | Extra arguments forwarded to the installer |
//! | `package_params` | — | Package-specific parameters (`--package-parameters`) |
//! | `ignore_checksums` | `false` | Pass `--ignore-checksums` (use with caution) |
//! | `ignore_dependencies` | `false` | Pass `--ignore-dependencies` |
//! | `skip_scripts` | `false` | Pass `--skip-scripts` |
//! | `force` | `false` | Pass `--force` (reinstall even if already installed) |
//! | `no_progress` | `true` | Pass `--no-progress` |
//! | `timeout` | — | Execution timeout in seconds |
//! | `choco_bin` | `choco` | Path to the Chocolatey CLI binary |
//!
//! ## NuGet / custom feed example
//!
//! ```yaml
//! - chocolatey:
//!     name: mypackage
//!     source: https://myserver/nuget/feed
//!     version: "1.2.3"
//! ```

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct ChocolateyModule;

impl ModuleInvoker for ChocolateyModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let bin = args.get_str("choco_bin").unwrap_or("choco").to_string();
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");

        match state {
            "upgrade_all" => choco_upgrade_all(&bin, args, host),
            "list" => choco_list(&bin, args, host),
            "info" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "chocolatey: 'name' is required for state=info".to_string(),
                    ));
                }
                choco_info(&bin, &packages, args, host)
            }
            "absent" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "chocolatey: 'name' is required for state=absent".to_string(),
                    ));
                }
                choco_uninstall(&bin, &packages, args, host)
            }
            "latest" => {
                if packages.is_empty() {
                    return choco_upgrade_all(&bin, args, host);
                }
                choco_upgrade(&bin, &packages, args, host)
            }
            _ => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "chocolatey: 'name' is required for state=present".to_string(),
                    ));
                }
                choco_install(&bin, &packages, args, host)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn base_flags(args: &ModuleArgs) -> Vec<String> {
    let mut v = vec!["-y".to_string()];
    if bool_arg(args, "no_progress", true) {
        v.push("--no-progress".into());
    }
    if let Some(src) = args.get_str("source") {
        v.push(format!("--source={src}"));
    }
    if bool_arg(args, "pre_release", false) {
        v.push("--pre".into());
    }
    if bool_arg(args, "ignore_checksums", false) {
        v.push("--ignore-checksums".into());
    }
    if bool_arg(args, "ignore_dependencies", false) {
        v.push("--ignore-dependencies".into());
    }
    if bool_arg(args, "skip_scripts", false) {
        v.push("--skip-scripts".into());
    }
    if bool_arg(args, "force", false) {
        v.push("--force".into());
    }
    if let Some(t) = args.args.get("timeout").and_then(|v| v.as_u64()) {
        v.push(format!("--timeout={t}"));
    }
    if let Some(e) = args.get_str("extra_args") {
        v.extend(e.split_whitespace().map(|s| s.to_string()));
    }
    v
}

fn version_flags(args: &ModuleArgs) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(ver) = args.get_str("version") {
        v.push(format!("--version={ver}"));
    }
    if let Some(pp) = args.get_str("package_params") {
        v.push(format!("--package-parameters={pp}"));
    }
    if let Some(ia) = args.get_str("install_args") {
        v.push(format!("--install-arguments={ia}"));
    }
    v
}

fn choco_install(
    bin: &str,
    packages: &[String],
    args: &ModuleArgs,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("install");
    cmd.args(base_flags(args));
    cmd.args(version_flags(args));
    for p in packages {
        cmd.arg(p);
    }
    let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
    if ok {
        let changed = !stdout.contains("already installed");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = if changed {
            format!("installed: {}", packages.join(", "))
        } else {
            "already installed".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("choco install failed (rc={rc}): {stderr}"),
        ))
    }
}

fn choco_upgrade(
    bin: &str,
    packages: &[String],
    args: &ModuleArgs,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("upgrade");
    cmd.args(base_flags(args));
    cmd.args(version_flags(args));
    for p in packages {
        cmd.arg(p);
    }
    let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
    if ok {
        let changed = !stdout.contains("already at the latest version");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = if changed {
            format!("upgraded: {}", packages.join(", "))
        } else {
            "already up-to-date".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("choco upgrade failed (rc={rc}): {stderr}"),
        ))
    }
}

fn choco_upgrade_all(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("upgrade");
    cmd.args(base_flags(args));
    cmd.arg("all");
    let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
    if ok {
        let mut r = TaskResult::changed(host);
        r.stdout = stdout;
        r.msg = "choco upgraded all packages".into();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("choco upgrade all failed (rc={rc}): {stderr}"),
        ))
    }
}

fn choco_uninstall(
    bin: &str,
    packages: &[String],
    args: &ModuleArgs,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("uninstall");
    cmd.args(base_flags(args));
    for p in packages {
        cmd.arg(p);
    }
    let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
    if ok {
        let mut r = TaskResult::changed(host);
        r.stdout = stdout;
        r.msg = format!("uninstalled: {}", packages.join(", "));
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("choco uninstall failed (rc={rc}): {stderr}"),
        ))
    }
}

fn choco_list(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("list");
    if bool_arg(args, "local_only", true) {
        cmd.arg("--local-only");
    }
    if let Some(src) = args.get_str("source") {
        cmd.arg(format!("--source={src}"));
    }
    let (ok, stdout, _stderr, _rc) = run_cmd(cmd)?;
    if ok {
        let packages: Vec<Value> = stdout
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with("Chocolatey"))
            .map(|l| Value::String(l.to_string()))
            .collect();
        let mut r = TaskResult::ok(host);
        r.stdout = stdout;
        r.vars.insert("packages".into(), Value::Array(packages));
        Ok(r)
    } else {
        Ok(TaskResult::ok(host)) // list failure is non-fatal
    }
}

fn choco_info(bin: &str, packages: &[String], args: &ModuleArgs, host: &str) -> Result<TaskResult> {
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

fn run_cmd(mut cmd: Command) -> Result<(bool, String, String, i32)> {
    let out = cmd.output().context("choco")?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    ))
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
        let r = ChocolateyModule
            .invoke(&ModuleArgs::new(HashMap::new()), "h", &mut c)
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails() {
        let mut c = ctx();
        let r = ChocolateyModule
            .invoke(
                &args(&[("state", Value::String("absent".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_base_flags_no_progress_default() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(base_flags(&a).contains(&"--no-progress".to_string()));
    }
    #[test]
    fn test_base_flags_source() {
        let a = args(&[("source", Value::String("https://myserver/nuget".into()))]);
        let flags = base_flags(&a);
        assert!(flags.iter().any(|f| f.contains("myserver/nuget")));
    }
    #[test]
    fn test_version_flags_pin() {
        let a = args(&[("version", Value::String("1.2.3".into()))]);
        let flags = version_flags(&a);
        assert!(flags.iter().any(|f| f.contains("1.2.3")));
    }
    #[test]
    fn test_ignore_checksums_default_false() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "ignore_checksums", false));
    }
    #[test]
    fn test_collect_packages() {
        let a = args(&[("name", Value::String("git".into()))]);
        assert_eq!(collect_packages(&a), vec!["git"]);
    }
}
