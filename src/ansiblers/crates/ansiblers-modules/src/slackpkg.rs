//! `slackpkg` module — manage packages on Slackware Linux.
//!
//! Supports three Slackware package tools, selected automatically or by the
//! `tool` parameter:
//!
//! | Tool | Default binary | Description |
//! |------|---------------|-------------|
//! | `pkgtool` | `installpkg` / `removepkg` | Slackware native (low-level) |
//! | `slackpkg` | `slackpkg` | Official Slackware network package tool |
//! | `slapt_get` | `slapt-get` | `apt`-like front-end for Slackware (Slapt-Get) |
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Package name(s) |
//! | `state` | `present` | `present`, `absent`, `latest`, `info`, `upgrade_all`, `update_cache` |
//! | `tool` | auto | `pkgtool`, `slackpkg`, or `slapt_get` |
//! | `terse` | `true` | Pass `--terse` to slackpkg |
//! | `no_questions` | `true` | Non-interactive (`-batch=on -default_answer=y` / `--no-prompt`) |
//! | `src` | — | Path to a `.tgz`/`.txz`/`.tlz` package for pkgtool direct install |
//! | `upgrade` | `false` | Use `upgradepkg` / `slapt-get --upgrade` |
//! | `installpkg_bin` | `installpkg` | pkgtool install binary |
//! | `removepkg_bin` | `removepkg` | pkgtool remove binary |
//! | `upgradepkg_bin` | `upgradepkg` | pkgtool upgrade binary |
//! | `slackpkg_bin` | `slackpkg` | slackpkg binary |
//! | `slapt_get_bin` | `slapt-get` | slapt-get binary |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct SlackpkgModule;

impl ModuleInvoker for SlackpkgModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");
        let tool = detect_tool(args);

        match tool.as_str() {
            "pkgtool" => pkgtool_dispatch(state, &packages, args, host),
            "slapt_get" | "slapt-get" => slapt_dispatch(state, &packages, args, host),
            _ => slackpkg_dispatch(state, &packages, args, host),
        }
    }
}

// ---------------------------------------------------------------------------
// Tool detection
// ---------------------------------------------------------------------------

fn detect_tool(args: &ModuleArgs) -> String {
    if let Some(t) = args.get_str("tool") {
        return t.to_string();
    }
    // Prefer slapt-get if available (richer feature set), then slackpkg.
    if which("slapt-get") {
        return "slapt_get".into();
    }
    if which("slackpkg") {
        return "slackpkg".into();
    }
    "pkgtool".into()
}

fn which(binary: &str) -> bool {
    Command::new("which")
        .arg(binary)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// pkgtool (installpkg / removepkg / upgradepkg)
// ---------------------------------------------------------------------------

fn pkgtool_dispatch(
    state: &str,
    packages: &[String],
    args: &ModuleArgs,
    host: &str,
) -> Result<TaskResult> {
    match state {
        "absent" => {
            if packages.is_empty() {
                return Ok(TaskResult::failed(
                    host,
                    "slackpkg: 'name' required for state=absent".to_string(),
                ));
            }
            let bin = args.get_str("removepkg_bin").unwrap_or("removepkg");
            let mut cmd = Command::new(bin);
            for p in packages {
                cmd.arg(p);
            }
            let out = cmd.output().context("removepkg")?;
            if out.status.success() {
                let mut r = TaskResult::changed(host);
                r.msg = format!("removed: {}", packages.join(", "));
                Ok(r)
            } else {
                Ok(TaskResult::failed(
                    host,
                    format!("removepkg failed: {}", String::from_utf8_lossy(&out.stderr)),
                ))
            }
        }
        "latest" => {
            if packages.is_empty() {
                return Ok(TaskResult::failed(
                    host,
                    "slackpkg: 'name' required for state=latest".to_string(),
                ));
            }
            let src = args.get_str("src");
            let bin = args.get_str("upgradepkg_bin").unwrap_or("upgradepkg");
            let mut cmd = Command::new(bin);
            if let Some(s) = src {
                cmd.arg(s);
            } else {
                for p in packages {
                    cmd.arg(p);
                }
            }
            let out = cmd.output().context("upgradepkg")?;
            if out.status.success() {
                let mut r = TaskResult::changed(host);
                r.msg = format!("upgraded: {}", packages.join(", "));
                Ok(r)
            } else {
                Ok(TaskResult::failed(
                    host,
                    format!(
                        "upgradepkg failed: {}",
                        String::from_utf8_lossy(&out.stderr)
                    ),
                ))
            }
        }
        _ => {
            // present: use installpkg
            let src = args.get_str("src");
            if src.is_none() && packages.is_empty() {
                return Ok(TaskResult::failed(
                    host,
                    "slackpkg: 'name' or 'src' required for state=present".to_string(),
                ));
            }
            let bin = args.get_str("installpkg_bin").unwrap_or("installpkg");
            let mut cmd = Command::new(bin);
            if let Some(s) = src {
                cmd.arg(s);
            } else {
                for p in packages {
                    cmd.arg(p);
                }
            }
            let out = cmd.output().context("installpkg")?;
            if out.status.success() {
                let mut r = TaskResult::changed(host);
                r.msg = format!("installed: {}", packages.join(", "));
                Ok(r)
            } else {
                Ok(TaskResult::failed(
                    host,
                    format!(
                        "installpkg failed: {}",
                        String::from_utf8_lossy(&out.stderr)
                    ),
                ))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// slackpkg
// ---------------------------------------------------------------------------

fn slackpkg_dispatch(
    state: &str,
    packages: &[String],
    args: &ModuleArgs,
    host: &str,
) -> Result<TaskResult> {
    let bin = args
        .get_str("slackpkg_bin")
        .unwrap_or("slackpkg")
        .to_string();
    let terse = bool_arg(args, "terse", true);
    let no_q = bool_arg(args, "no_questions", true);

    let mut base_flags: Vec<&str> = vec![];
    if terse {
        base_flags.push("-terse");
    }
    if no_q {
        base_flags.push("-batch=on");
        base_flags.push("-default_answer=y");
    }

    match state {
        "update_cache" => {
            let mut cmd = Command::new(&bin);
            cmd.args(&base_flags);
            cmd.arg("update");
            run_slackpkg(cmd, host, "slackpkg update")
        }
        "upgrade_all" => {
            let mut cmd = Command::new(&bin);
            cmd.args(&base_flags);
            cmd.arg("upgrade-all");
            run_slackpkg(cmd, host, "slackpkg upgrade-all")
        }
        "absent" => {
            if packages.is_empty() {
                return Ok(TaskResult::failed(
                    host,
                    "slackpkg: 'name' required for state=absent".to_string(),
                ));
            }
            let mut cmd = Command::new(&bin);
            cmd.args(&base_flags);
            cmd.arg("remove");
            for p in packages {
                cmd.arg(p);
            }
            run_slackpkg(cmd, host, "slackpkg remove")
        }
        "latest" => {
            if packages.is_empty() {
                return Ok(TaskResult::failed(
                    host,
                    "slackpkg: 'name' required for state=latest".to_string(),
                ));
            }
            let mut cmd = Command::new(&bin);
            cmd.args(&base_flags);
            cmd.arg("upgrade");
            for p in packages {
                cmd.arg(p);
            }
            run_slackpkg(cmd, host, "slackpkg upgrade")
        }
        _ => {
            if packages.is_empty() {
                return Ok(TaskResult::failed(
                    host,
                    "slackpkg: 'name' required for state=present".to_string(),
                ));
            }
            let mut cmd = Command::new(&bin);
            cmd.args(&base_flags);
            cmd.arg("install");
            for p in packages {
                cmd.arg(p);
            }
            run_slackpkg(cmd, host, "slackpkg install")
        }
    }
}

fn run_slackpkg(mut cmd: Command, host: &str, label: &str) -> Result<TaskResult> {
    let out = cmd.output().with_context(|| label.to_string())?;
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        r.msg = label.to_string();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("{label} failed: {}", String::from_utf8_lossy(&out.stderr)),
        ))
    }
}

// ---------------------------------------------------------------------------
// slapt-get
// ---------------------------------------------------------------------------

fn slapt_dispatch(
    state: &str,
    packages: &[String],
    args: &ModuleArgs,
    host: &str,
) -> Result<TaskResult> {
    let bin = args
        .get_str("slapt_get_bin")
        .unwrap_or("slapt-get")
        .to_string();
    let no_prompt = bool_arg(args, "no_questions", true);

    match state {
        "update_cache" => {
            let mut cmd = Command::new(&bin);
            if no_prompt {
                cmd.arg("--no-prompt");
            }
            cmd.arg("--update");
            run_slackpkg(cmd, host, "slapt-get --update")
        }
        "upgrade_all" => {
            let mut cmd = Command::new(&bin);
            if no_prompt {
                cmd.arg("--no-prompt");
            }
            cmd.arg("--upgrade");
            run_slackpkg(cmd, host, "slapt-get --upgrade")
        }
        "absent" => {
            if packages.is_empty() {
                return Ok(TaskResult::failed(
                    host,
                    "slackpkg: 'name' required for state=absent".to_string(),
                ));
            }
            let mut cmd = Command::new(&bin);
            if no_prompt {
                cmd.arg("--no-prompt");
            }
            cmd.arg("--remove");
            for p in packages {
                cmd.arg(p);
            }
            run_slackpkg(cmd, host, "slapt-get --remove")
        }
        "latest" => {
            if packages.is_empty() {
                return Ok(TaskResult::failed(
                    host,
                    "slackpkg: 'name' required for state=latest".to_string(),
                ));
            }
            let mut cmd = Command::new(&bin);
            if no_prompt {
                cmd.arg("--no-prompt");
            }
            cmd.args(["--install", "--reinstall"]);
            for p in packages {
                cmd.arg(p);
            }
            run_slackpkg(cmd, host, "slapt-get --install --reinstall")
        }
        _ => {
            if packages.is_empty() {
                return Ok(TaskResult::failed(
                    host,
                    "slackpkg: 'name' required for state=present".to_string(),
                ));
            }
            let mut cmd = Command::new(&bin);
            if no_prompt {
                cmd.arg("--no-prompt");
            }
            cmd.arg("--install");
            for p in packages {
                cmd.arg(p);
            }
            run_slackpkg(cmd, host, "slapt-get --install")
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
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
    fn test_pkgtool_no_name_or_src_fails() {
        let r = pkgtool_dispatch("present", &[], &ModuleArgs::new(HashMap::new()), "h").unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_slackpkg_no_name_present_fails() {
        let r = slackpkg_dispatch("present", &[], &ModuleArgs::new(HashMap::new()), "h").unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_slapt_no_name_present_fails() {
        let r = slapt_dispatch("present", &[], &ModuleArgs::new(HashMap::new()), "h").unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_collect_packages_string() {
        let a = args(&[("name", Value::String("aaa_base".into()))]);
        assert_eq!(collect_packages(&a), vec!["aaa_base"]);
    }
    #[test]
    fn test_no_questions_default_true() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(bool_arg(&a, "no_questions", true));
    }
}
