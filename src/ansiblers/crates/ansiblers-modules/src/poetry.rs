//! `poetry` module — manage Python projects and packages via [Poetry](https://python-poetry.org/).
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Package name(s) with optional version constraint |
//! | `state` | `present` | `present` (add), `absent` (remove), `latest` (update), `install`, `build`, `publish`, `run`, `env_info`, `check`, `lock`, `export` |
//! | `version` | — | Version constraint appended to the package name |
//! | `dev` | `false` | Add to `[tool.poetry.dev-dependencies]` (`--group dev`) |
//! | `group` | — | Dependency group name (Poetry 1.2+) |
//! | `extras` | — | List of extras to enable on add |
//! | `optional` | `false` | Mark as optional dependency |
//! | `allow_prereleases` | `false` | Allow pre-release versions |
//! | `source` | — | Package source name to use |
//! | `project` | — | Path to the project directory (`--directory`) |
//! | `no_dev` | `false` | `--without dev` on `install` |
//! | `only_root` | `false` | `--only-root` |
//! | `sync` | `false` | `--sync` on install |
//! | `frozen` | `false` | Do not update `poetry.lock` |
//! | `with_groups` | — | Comma-separated groups to include on `install` |
//! | `without_groups` | — | Comma-separated groups to exclude on `install` |
//! | `command` | — | Command for `state=run` |
//! | `build_format` | — | `sdist` or `wheel` for `state=build` |
//! | `publish_repo` | — | Repository name for `state=publish` |
//! | `dry_run` | `false` | `--dry-run` where supported |
//! | `poetry_bin` | `poetry` | Path to the poetry binary |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct PoetryModule;

impl ModuleInvoker for PoetryModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let bin = args.get_str("poetry_bin").unwrap_or("poetry").to_string();
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");

        match state {
            "install" => poetry_install(&bin, args, host),
            "lock" => poetry_lock(&bin, args, host),
            "check" => poetry_check(&bin, args, host),
            "build" => poetry_build(&bin, args, host),
            "publish" => poetry_publish(&bin, args, host),
            "export" => poetry_export(&bin, args, host),
            "env_info" => poetry_env_info(&bin, args, host),
            "run" => {
                let cmd = args.get_str("command").ok_or_else(|| {
                    anyhow::anyhow!("poetry: 'command' is required for state=run")
                })?;
                poetry_run(&bin, cmd, args, host)
            }
            "absent" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "poetry: 'name' is required for state=absent".to_string(),
                    ));
                }
                poetry_remove(&bin, &packages, args, host)
            }
            "latest" => {
                if packages.is_empty() {
                    // Update all
                    return poetry_update_all(&bin, args, host);
                }
                poetry_update(&bin, &packages, args, host)
            }
            _ => {
                // present / add
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "poetry: 'name' is required for state=present".to_string(),
                    ));
                }
                poetry_add(&bin, &packages, args, host)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn project_args(args: &ModuleArgs) -> Vec<String> {
    if let Some(d) = args.get_str("project") {
        vec!["--directory".to_string(), d.to_string()]
    } else {
        vec![]
    }
}

fn group_args(args: &ModuleArgs, for_add: bool) -> Vec<String> {
    let mut v = Vec::new();
    if for_add {
        if bool_arg(args, "dev", false) {
            v.push("--group".into());
            v.push("dev".into());
        } else if let Some(g) = args.get_str("group") {
            v.push("--group".into());
            v.push(g.to_string());
        }
    } else {
        // install
        if bool_arg(args, "no_dev", false) {
            v.push("--without".into());
            v.push("dev".into());
        }
        if let Some(w) = args.get_str("with_groups") {
            v.push("--with".into());
            v.push(w.to_string());
        }
        if let Some(wo) = args.get_str("without_groups") {
            v.push("--without".into());
            v.push(wo.to_string());
        }
    }
    v
}

fn poetry_add(bin: &str, packages: &[String], args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(project_args(args));
    cmd.arg("add");
    cmd.args(group_args(args, true));
    if bool_arg(args, "optional", false) {
        cmd.arg("--optional");
    }
    if bool_arg(args, "allow_prereleases", false) {
        cmd.arg("--allow-prereleases");
    }
    if bool_arg(args, "dry_run", false) {
        cmd.arg("--dry-run");
    }
    if let Some(src) = args.get_str("source") {
        cmd.args(["--source", src]);
    }
    if let Some(extras) = collect_list(args, "extras") {
        for e in extras {
            cmd.args(["--extras", &e]);
        }
    }
    for p in packages {
        cmd.arg(p);
    }

    run_change(cmd, host, &format!("added: {}", packages.join(", ")))
}

fn poetry_remove(
    bin: &str,
    packages: &[String],
    args: &ModuleArgs,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(project_args(args));
    cmd.arg("remove");
    cmd.args(group_args(args, true));
    if bool_arg(args, "dry_run", false) {
        cmd.arg("--dry-run");
    }
    for p in packages {
        cmd.arg(p);
    }
    run_change(cmd, host, &format!("removed: {}", packages.join(", ")))
}

fn poetry_update(
    bin: &str,
    packages: &[String],
    args: &ModuleArgs,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(project_args(args));
    cmd.arg("update");
    if bool_arg(args, "dry_run", false) {
        cmd.arg("--dry-run");
    }
    for p in packages {
        cmd.arg(p);
    }
    run_change(cmd, host, &format!("updated: {}", packages.join(", ")))
}

fn poetry_update_all(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(project_args(args));
    cmd.arg("update");
    if bool_arg(args, "dry_run", false) {
        cmd.arg("--dry-run");
    }
    run_change(cmd, host, "updated all dependencies")
}

fn poetry_install(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(project_args(args));
    cmd.arg("install");
    cmd.args(group_args(args, false));
    if bool_arg(args, "only_root", false) {
        cmd.arg("--only-root");
    }
    if bool_arg(args, "sync", false) {
        cmd.arg("--sync");
    }
    if bool_arg(args, "frozen", false) {
        cmd.arg("--frozen");
    }
    if bool_arg(args, "dry_run", false) {
        cmd.arg("--dry-run");
    }

    let out = cmd.output().context("poetry install")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let changed = !stdout.contains("No dependencies to install or update");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = if changed {
            "dependencies installed".into()
        } else {
            "already up-to-date".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("poetry install failed: {stderr}"),
        ))
    }
}

fn poetry_lock(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(project_args(args));
    cmd.arg("lock");
    if bool_arg(args, "no_update", false) {
        cmd.arg("--no-update");
    }
    run_change(cmd, host, "lockfile updated")
}

fn poetry_check(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(project_args(args));
    cmd.arg("check");
    let out = cmd.output().context("poetry check")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::ok(host);
        r.stdout = stdout;
        r.msg = "pyproject.toml is valid".into();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("poetry check failed: {stdout}{stderr}"),
        ))
    }
}

fn poetry_build(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(project_args(args));
    cmd.arg("build");
    if let Some(fmt) = args.get_str("build_format") {
        cmd.args(["--format", fmt]);
    }
    run_change(cmd, host, "package built")
}

fn poetry_publish(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(project_args(args));
    cmd.arg("publish");
    if let Some(repo) = args.get_str("publish_repo") {
        cmd.args(["--repository", repo]);
    }
    if bool_arg(args, "dry_run", false) {
        cmd.arg("--dry-run");
    }
    run_change(cmd, host, "package published")
}

fn poetry_export(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let fmt = args.get_str("format").unwrap_or("requirements.txt");
    let output = args.get_str("output").unwrap_or("requirements.txt");
    let mut cmd = Command::new(bin);
    cmd.args(project_args(args));
    cmd.args(["export", "--format", fmt, "--output", output]);
    if bool_arg(args, "without_hashes", false) {
        cmd.arg("--without-hashes");
    }
    run_change(cmd, host, &format!("exported to '{output}'"))
}

fn poetry_env_info(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(project_args(args));
    cmd.args(["env", "info", "--json"]);
    let out = cmd.output().context("poetry env info")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let info: Value = serde_json::from_str(&stdout).unwrap_or(Value::String(stdout.clone()));
    let mut r = TaskResult::ok(host);
    r.stdout = stdout;
    r.vars.insert("env_info".into(), info);
    Ok(r)
}

fn poetry_run(bin: &str, command: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(project_args(args));
    cmd.arg("run");
    cmd.args(command.split_whitespace());
    let out = cmd.output().context("poetry run")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let rc = out.status.code().unwrap_or(-1);
    if out.status.success() {
        let mut r = TaskResult::ok(host);
        r.stdout = stdout;
        r.msg = format!("ran: {command}");
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("poetry run '{command}' failed (rc={rc}): {stderr}"),
        ))
    }
}

fn run_change(mut cmd: Command, host: &str, msg: &str) -> Result<TaskResult> {
    let out = cmd.output().context("poetry")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let rc = out.status.code().unwrap_or(-1);
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        r.msg = msg.to_string();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("poetry failed (rc={rc}): {stderr}"),
        ))
    }
}

fn collect_packages(args: &ModuleArgs) -> Vec<String> {
    let version = args.get_str("version");
    let val = args.args.get("name").or_else(|| args.args.get("pkg"));
    let pkgs: Vec<String> = match val {
        None => vec![],
        Some(Value::String(s)) => s.split_whitespace().map(|s| s.to_string()).collect(),
        Some(Value::Array(seq)) => seq
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => vec![],
    };
    pkgs.into_iter()
        .map(|p| {
            if let Some(v) = version {
                if p.contains(['=', '<', '>', '^', '~', '!', '@']) {
                    p
                } else {
                    format!("{p}@{v}")
                }
            } else {
                p
            }
        })
        .collect()
}

fn collect_list(args: &ModuleArgs, key: &str) -> Option<Vec<String>> {
    match args.args.get(key)? {
        Value::String(s) => Some(s.split(',').map(|s| s.trim().to_string()).collect()),
        Value::Array(seq) => Some(
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
        ),
        _ => None,
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
        let r = PoetryModule
            .invoke(&ModuleArgs::new(HashMap::new()), "h", &mut c)
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails() {
        let mut c = ctx();
        let r = PoetryModule
            .invoke(
                &args(&[("state", Value::String("absent".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_run_missing_command_errors() {
        let mut c = ctx();
        let r = PoetryModule.invoke(
            &args(&[("state", Value::String("run".into()))]),
            "h",
            &mut c,
        );
        assert!(r.is_err());
    }
    #[test]
    fn test_project_args_set() {
        let a = args(&[("project", Value::String("/srv/myapp".into()))]);
        assert_eq!(project_args(&a), vec!["--directory", "/srv/myapp"]);
    }
    #[test]
    fn test_project_args_empty() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(project_args(&a).is_empty());
    }
    #[test]
    fn test_group_args_dev() {
        let a = args(&[("dev", Value::Bool(true))]);
        let v = group_args(&a, true);
        assert!(v.contains(&"dev".to_string()));
    }
    #[test]
    fn test_collect_packages_version_appended() {
        let a = args(&[
            ("name", Value::String("requests".into())),
            ("version", Value::String("^2.28".into())),
        ]);
        let pkgs = collect_packages(&a);
        assert_eq!(pkgs, vec!["requests@^2.28"]);
    }
    #[test]
    fn test_collect_packages_existing_constraint_preserved() {
        let a = args(&[("name", Value::String("requests>=2.0".into()))]);
        assert_eq!(collect_packages(&a), vec!["requests>=2.0"]);
    }
    #[test]
    fn test_dev_default_false() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "dev", false));
    }
}
