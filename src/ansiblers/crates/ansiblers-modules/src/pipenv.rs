//! `pipenv` module — manage Python virtual environments and packages via
//! [`Pipenv`](https://pipenv.pypa.io/).
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Package name(s), optionally with version specifier |
//! | `state` | `present` | `present` (install pkg), `absent` (uninstall pkg), `latest` (update), `install_all` (install from Pipfile), `lock`, `run`, `check`, `graph`, `clean`, `shell`, `requirements` |
//! | `version` | — | Version specifier appended to the package name |
//! | `dev` | `false` | Install as a dev dependency (`--dev`) |
//! | `python` | — | Python version for `--python` |
//! | `pre` | `false` | Allow pre-release versions |
//! | `system` | `false` | Use system Python (`--system`) |
//! | `deploy` | `false` | `--deploy` — fail if Pipfile.lock is out-of-date |
//! | `ignore_pipfile` | `false` | `--ignore-pipfile` during install |
//! | `skip_lock` | `false` | `--skip-lock` |
//! | `keep_outdated` | `false` | `--keep-outdated` |
//! | `categories` | — | Install from specific categories |
//! | `command` | — | Command to run for `state=run` |
//! | `requirements_file` | — | Output file for `state=requirements` |
//! | `project` | — | Working directory (sets `PIPENV_PIPFILE` env var) |
//! | `extra_args` | — | Extra flags forwarded verbatim |
//! | `pipenv_bin` | `pipenv` | Path to the pipenv binary |

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct PipenvModule;

impl ModuleInvoker for PipenvModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let bin = args.get_str("pipenv_bin").unwrap_or("pipenv").to_string();
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");

        match state {
            "install_all" => pipenv_install_all(&bin, args, host),
            "lock" => pipenv_lock(&bin, args, host),
            "check" => pipenv_check(&bin, args, host),
            "graph" => pipenv_graph(&bin, args, host),
            "clean" => pipenv_clean(&bin, args, host),
            "requirements" => pipenv_requirements(&bin, args, host),
            "run" => {
                let cmd = args.get_str("command").ok_or_else(|| {
                    anyhow::anyhow!("pipenv: 'command' is required for state=run")
                })?;
                pipenv_run(&bin, cmd, args, host)
            }
            "absent" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pipenv: 'name' is required for state=absent".to_string(),
                    ));
                }
                pipenv_uninstall(&bin, &packages, args, host)
            }
            "latest" => {
                if packages.is_empty() {
                    return pipenv_update_all(&bin, args, host);
                }
                pipenv_update(&bin, &packages, args, host)
            }
            _ => {
                // present
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pipenv: 'name' is required for state=present".to_string(),
                    ));
                }
                pipenv_install_pkgs(&bin, &packages, args, host)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_cmd(bin: &str, args: &ModuleArgs) -> Command {
    let mut cmd = Command::new(bin);
    // Set PIPENV_PIPFILE to use a non-default project dir.
    if let Some(project) = args.get_str("project") {
        cmd.env("PIPENV_PIPFILE", format!("{project}/Pipfile"));
    }
    cmd
}

fn common_flags(args: &ModuleArgs) -> Vec<String> {
    let mut v = Vec::new();
    if bool_arg(args, "pre", false) {
        v.push("--pre".into());
    }
    if bool_arg(args, "skip_lock", false) {
        v.push("--skip-lock".into());
    }
    if bool_arg(args, "keep_outdated", false) {
        v.push("--keep-outdated".into());
    }
    if let Some(e) = args.get_str("extra_args") {
        v.extend(e.split_whitespace().map(|s| s.to_string()));
    }
    v
}

fn pipenv_install_pkgs(
    bin: &str,
    packages: &[String],
    args: &ModuleArgs,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, args);
    cmd.arg("install");
    if bool_arg(args, "dev", false) {
        cmd.arg("--dev");
    }
    if let Some(py) = args.get_str("python") {
        cmd.args(["--python", py]);
    }
    cmd.args(common_flags(args));
    for p in packages {
        cmd.arg(p);
    }

    let out = cmd.output().context("pipenv install")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let changed = stdout.contains("Installing") || stdout.contains("Adding");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = format!("installed: {}", packages.join(", "));
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pipenv install failed: {stderr}"),
        ))
    }
}

fn pipenv_uninstall(
    bin: &str,
    packages: &[String],
    args: &ModuleArgs,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, args);
    cmd.arg("uninstall");
    if bool_arg(args, "dev", false) {
        cmd.arg("--dev");
    }
    cmd.args(common_flags(args));
    for p in packages {
        cmd.arg(p);
    }

    let out = cmd.output().context("pipenv uninstall")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = format!("uninstalled: {}", packages.join(", "));
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pipenv uninstall failed: {stderr}"),
        ))
    }
}

fn pipenv_update(
    bin: &str,
    packages: &[String],
    args: &ModuleArgs,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, args);
    cmd.arg("update");
    cmd.args(common_flags(args));
    for p in packages {
        cmd.arg(p);
    }
    run_change(cmd, host, &format!("updated: {}", packages.join(", ")))
}

fn pipenv_update_all(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, args);
    cmd.arg("update");
    cmd.args(common_flags(args));
    run_change(cmd, host, "updated all packages")
}

fn pipenv_install_all(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, args);
    cmd.arg("install");
    if bool_arg(args, "dev", false) {
        cmd.arg("--dev");
    }
    if let Some(py) = args.get_str("python") {
        cmd.args(["--python", py]);
    }
    if bool_arg(args, "deploy", false) {
        cmd.arg("--deploy");
    }
    if bool_arg(args, "ignore_pipfile", false) {
        cmd.arg("--ignore-pipfile");
    }
    if bool_arg(args, "system", false) {
        cmd.arg("--system");
    }
    if let Some(cats) = args.get_str("categories") {
        cmd.args(["--categories", cats]);
    }
    cmd.args(common_flags(args));

    let out = cmd.output().context("pipenv install")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let changed = !stdout.contains("All dependencies are already satisfied");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = if changed {
            "Pipfile dependencies installed".into()
        } else {
            "already up-to-date".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pipenv install failed: {stderr}"),
        ))
    }
}

fn pipenv_lock(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, args);
    cmd.arg("lock");
    if bool_arg(args, "dev", false) {
        cmd.arg("--dev");
    }
    if bool_arg(args, "pre", false) {
        cmd.arg("--pre");
    }
    if let Some(e) = args.get_str("extra_args") {
        cmd.args(e.split_whitespace());
    }
    run_change(cmd, host, "Pipfile.lock updated")
}

fn pipenv_check(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, args);
    cmd.arg("check");
    let out = cmd.output().context("pipenv check")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::ok(host);
        r.stdout = stdout;
        r.msg = "no security vulnerabilities found".into();
        Ok(r)
    } else {
        // pipenv check exits non-zero when vulnerabilities found.
        let mut r = TaskResult::failed(host, format!("vulnerabilities found: {stdout}{stderr}"));
        r.vars
            .insert("vulnerabilities".into(), Value::String(stdout));
        Ok(r)
    }
}

fn pipenv_graph(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, args);
    cmd.args(["graph", "--json"]);
    let out = cmd.output().context("pipenv graph")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let graph: Value = serde_json::from_str(&stdout).unwrap_or(Value::String(stdout.clone()));
    let mut r = TaskResult::ok(host);
    r.stdout = stdout;
    r.vars.insert("graph".into(), graph);
    Ok(r)
}

fn pipenv_clean(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, args);
    cmd.arg("clean");
    if bool_arg(args, "dry_run", false) {
        cmd.arg("--dry-run");
    }
    run_change(cmd, host, "removed unused packages")
}

fn pipenv_requirements(bin: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, args);
    cmd.arg("requirements");
    if bool_arg(args, "dev", false) {
        cmd.arg("--dev-only");
    }
    if bool_arg(args, "hash", false) {
        cmd.arg("--hash");
    }
    let out = cmd.output().context("pipenv requirements")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        // Optionally write to file.
        if let Some(file) = args.get_str("requirements_file") {
            std::fs::write(file, &stdout)?;
            let mut r = TaskResult::changed(host);
            r.stdout = stdout;
            r.msg = format!("requirements written to '{file}'");
            return Ok(r);
        }
        let mut r = TaskResult::ok(host);
        r.stdout = stdout.clone();
        r.vars.insert("requirements".into(), Value::String(stdout));
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pipenv requirements failed: {stderr}"),
        ))
    }
}

fn pipenv_run(bin: &str, command: &str, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, args);
    cmd.arg("run");
    cmd.args(command.split_whitespace());
    let out = cmd.output().context("pipenv run")?;
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
            format!("pipenv run '{command}' failed (rc={rc}): {stderr}"),
        ))
    }
}

fn run_change(mut cmd: Command, host: &str, msg: &str) -> Result<TaskResult> {
    let out = cmd.output().context("pipenv")?;
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
            format!("pipenv failed (rc={rc}): {stderr}"),
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
                if p.contains(['=', '<', '>', '~', '!']) {
                    p
                } else {
                    format!("{p}{v}")
                }
            } else {
                p
            }
        })
        .collect()
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
        let r = PipenvModule
            .invoke(&ModuleArgs::new(HashMap::new()), "h", &mut c)
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails() {
        let mut c = ctx();
        let r = PipenvModule
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
        let r = PipenvModule.invoke(
            &args(&[("state", Value::String("run".into()))]),
            "h",
            &mut c,
        );
        assert!(r.is_err());
    }
    #[test]
    fn test_collect_packages_version_appended() {
        let a = args(&[
            ("name", Value::String("requests".into())),
            ("version", Value::String("==2.28.0".into())),
        ]);
        assert_eq!(collect_packages(&a), vec!["requests==2.28.0"]);
    }
    #[test]
    fn test_collect_packages_existing_specifier_preserved() {
        let a = args(&[("name", Value::String("requests>=2.0".into()))]);
        assert_eq!(collect_packages(&a), vec!["requests>=2.0"]);
    }
    #[test]
    fn test_dev_default_false() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "dev", false));
    }
    #[test]
    fn test_deploy_default_false() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "deploy", false));
    }
    #[test]
    fn test_common_flags_pre() {
        let a = args(&[("pre", Value::Bool(true))]);
        assert!(common_flags(&a).contains(&"--pre".to_string()));
    }
    #[test]
    fn test_project_env_var_set() {
        // Verify build_cmd doesn't panic with a project path.
        let a = args(&[("project", Value::String("/srv/myapp".into()))]);
        let _ = build_cmd("pipenv", &a);
    }
}
