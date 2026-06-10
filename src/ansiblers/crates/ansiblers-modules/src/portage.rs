//! `portage` module — manage packages on Gentoo Linux via `emerge`.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Atom(s) (e.g. `dev-vcs/git`, `=app-editors/vim-9.0`) |
//! | `state` | `present` | `present`, `absent`, `latest`, `info`, `sync`, `world_update` |
//! | `update` | `false` | Pass `--update` on install |
//! | `deep` | `false` | Pass `--deep` (full dependency update) |
//! | `newuse` | `false` | Pass `--newuse` (recompile if USE flags changed) |
//! | `changed_use` | `false` | Pass `--changed-use` |
//! | `noreplace` | `true` | Pass `--noreplace` (skip already installed) |
//! | `oneshot` | `false` | Pass `--oneshot` (don't add to world set) |
//! | `nodeps` | `false` | Pass `--nodeps` |
//! | `ask` | `false` | Pass `--ask` (interactive; avoid in automation) |
//! | `usepkg` | `false` | Pass `--usepkg` (use binary packages) |
//! | `jobs` | — | Number of parallel jobs (`--jobs=N`) |
//! | `load_average` | — | Load-average limit (`--load-average=N`) |
//! | `extra_args` | — | Extra flags forwarded verbatim |
//! | `emerge_bin` | `emerge` | Path to the emerge binary |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct PortageModule;

impl ModuleInvoker for PortageModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let bin = args.get_str("emerge_bin").unwrap_or("emerge").to_string();
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");

        match state {
            "sync" => emerge_sync(&bin, host),
            "world_update" => emerge_world_update(&bin, args, host),
            "info" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "portage: 'name' is required for state=info".to_string(),
                    ));
                }
                emerge_info(&bin, &packages, host)
            }
            "absent" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "portage: 'name' is required for state=absent".to_string(),
                    ));
                }
                emerge_unmerge(&bin, &packages, args, host)
            }
            _ => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "portage: 'name' is required for state=present/latest".to_string(),
                    ));
                }
                let upgrade = state == "latest";
                emerge_install(&bin, &packages, upgrade, args, host)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_flags(args: &ModuleArgs, upgrade: bool) -> Vec<String> {
    let mut flags = Vec::new();
    if upgrade || bool_arg(args, "update", false) {
        flags.push("--update".into());
    }
    if bool_arg(args, "deep", false) {
        flags.push("--deep".into());
    }
    if bool_arg(args, "newuse", false) {
        flags.push("--newuse".into());
    }
    if bool_arg(args, "changed_use", false) {
        flags.push("--changed-use".into());
    }
    if bool_arg(args, "noreplace", true) {
        flags.push("--noreplace".into());
    }
    if bool_arg(args, "oneshot", false) {
        flags.push("--oneshot".into());
    }
    if bool_arg(args, "nodeps", false) {
        flags.push("--nodeps".into());
    }
    if bool_arg(args, "usepkg", false) {
        flags.push("--usepkg".into());
    }
    if bool_arg(args, "ask", false) {
        flags.push("--ask".into());
    }
    if let Some(j) = args.args.get("jobs").and_then(|v| v.as_u64()) {
        flags.push(format!("--jobs={j}"));
    }
    if let Some(la) = args.get_str("load_average") {
        flags.push(format!("--load-average={la}"));
    }
    if let Some(e) = args.get_str("extra_args") {
        flags.extend(e.split_whitespace().map(|s| s.to_string()));
    }
    flags
}

fn emerge_install(
    bin: &str,
    packages: &[String],
    upgrade: bool,
    args: &ModuleArgs,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(build_flags(args, upgrade));
    for p in packages {
        cmd.arg(p);
    }
    let out = cmd.output().context("emerge install")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let changed = stdout.contains(">>> Emerging") || stdout.contains(">>> Installing");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = if changed {
            format!("emerged: {}", packages.join(", "))
        } else {
            "already installed".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(host, format!("emerge failed: {stderr}")))
    }
}

fn emerge_unmerge(
    bin: &str,
    packages: &[String],
    args: &ModuleArgs,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("--unmerge");
    if let Some(e) = args.get_str("extra_args") {
        cmd.args(e.split_whitespace());
    }
    for p in packages {
        cmd.arg(p);
    }
    let out = cmd.output().context("emerge --unmerge")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = format!("unmerged: {}", packages.join(", "));
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("emerge unmerge failed: {stderr}"),
        ))
    }
}

fn emerge_sync(bin: &str, host: &str) -> Result<TaskResult> {
    let out = Command::new(bin)
        .arg("--sync")
        .output()
        .context("emerge --sync")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = "portage tree synced".into();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("emerge --sync failed: {stderr}"),
        ))
    }
}

fn emerge_world_update(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(build_flags(args, true));
    cmd.arg("@world");
    let out = cmd.output().context("emerge @world")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let changed = stdout.contains(">>> Emerging");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = if changed {
            "world updated".into()
        } else {
            "world already up-to-date".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("emerge @world failed: {stderr}"),
        ))
    }
}

fn emerge_info(bin: &str, packages: &[String], host: &str) -> Result<TaskResult> {
    let mut out_all = String::new();
    for pkg in packages {
        if let Ok(out) = Command::new(bin).args(["--info", pkg]).output() {
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
        let r = PortageModule
            .invoke(&ModuleArgs::new(HashMap::new()), "h", &mut c)
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails() {
        let mut c = ctx();
        let r = PortageModule
            .invoke(
                &args(&[("state", Value::String("absent".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_build_flags_update() {
        let a = args(&[("deep", Value::Bool(true)), ("newuse", Value::Bool(true))]);
        let flags = build_flags(&a, true);
        assert!(flags.contains(&"--update".to_string()));
        assert!(flags.contains(&"--deep".to_string()));
        assert!(flags.contains(&"--newuse".to_string()));
    }
    #[test]
    fn test_noreplace_default_true() {
        let a = ModuleArgs::new(HashMap::new());
        let flags = build_flags(&a, false);
        assert!(flags.contains(&"--noreplace".to_string()));
    }
    #[test]
    fn test_jobs_flag() {
        let a = args(&[("jobs", Value::Number(4.into()))]);
        let flags = build_flags(&a, false);
        assert!(flags.contains(&"--jobs=4".to_string()));
    }

    #[test]
    fn test_oneshot_flag() {
        let a = args(&[("oneshot", Value::Bool(true))]);
        let flags = build_flags(&a, false);
        assert!(flags.contains(&"--oneshot".to_string()));
    }

    #[test]
    fn test_usepkg_flag() {
        let a = args(&[("usepkg", Value::Bool(true))]);
        let flags = build_flags(&a, false);
        assert!(flags.contains(&"--usepkg".to_string()));
    }

    #[test]
    fn test_changed_use_flag() {
        let a = args(&[("changed_use", Value::Bool(true))]);
        let flags = build_flags(&a, false);
        assert!(flags.contains(&"--changed-use".to_string()));
    }

    #[test]
    fn test_nodeps_flag() {
        let a = args(&[("nodeps", Value::Bool(true))]);
        let flags = build_flags(&a, false);
        assert!(flags.contains(&"--nodeps".to_string()));
    }

    #[test]
    fn test_no_name_info_fails() {
        let mut c = ctx();
        let r = PortageModule
            .invoke(
                &args(&[("state", Value::String("info".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }

    #[test]
    fn test_collect_packages_array() {
        let a = args(&[(
            "name",
            Value::Array(vec![
                Value::String("dev-vcs/git".into()),
                Value::String("app-editors/vim".into()),
            ]),
        )]);
        assert_eq!(collect_packages(&a).len(), 2);
    }

    #[test]
    fn test_extra_args_appended() {
        let a = args(&[("extra_args", Value::String("--ask".into()))]);
        let flags = build_flags(&a, false);
        assert!(flags.contains(&"--ask".to_string()));
    }
}
