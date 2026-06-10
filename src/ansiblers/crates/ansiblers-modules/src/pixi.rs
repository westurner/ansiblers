//! `pixi` module — manage project environments and packages via `pixi`.
//!
//! [Pixi](https://prefix.dev/docs/pixi) is a fast, cross-platform package
//! manager built on top of the conda ecosystem.  It manages per-project
//! environments defined in `pixi.toml` / `pyproject.toml` and supports
//! multiple platforms including `emscripten-wasm32`.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Package name(s) for `add`/`remove` |
//! | `state` | `present` | `present` (add), `absent` (remove), `latest` (update), `init`, `install`, `run`, `shell`, `info`, `clean` |
//! | `manifest_path` | — | Path to `pixi.toml` or `pyproject.toml` |
//! | `environment` / `env` | — | Pixi environment name (e.g. `default`, `dev`) |
//! | `platform` | — | Target platform (e.g. `linux-64`, `emscripten-wasm32`, `osx-arm64`) |
//! | `channel` | — | Additional channel to add on `init` |
//! | `channels` | — | List of channels for `init` |
//! | `feature` | — | Feature name for `add --feature` |
//! | `pypi` | `false` | Use `--pypi` flag (install from PyPI) |
//! | `command` | — | Command to run for `state=run` |
//! | `frozen` | `false` | Pass `--frozen` to install (no lockfile update) |
//! | `locked` | `false` | Pass `--locked` (respect exact lockfile) |
//! | `no_lockfile_update` | `false` | `--no-lockfile-update` |
//! | `pixi_bin` | `pixi` | Path to the pixi binary |
//!
//! ## Platform examples
//!
//! ```yaml
//! # Add a package for a specific platform
//! - pixi:
//!     name: numpy
//!     platform: emscripten-wasm32
//!
//! # Install the project environment
//! - pixi:
//!     state: install
//!     manifest_path: /srv/myproject/pixi.toml
//!
//! # Run a task
//! - pixi:
//!     state: run
//!     command: test
//!     manifest_path: /srv/myproject/pixi.toml
//! ```

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct PixiModule;

impl ModuleInvoker for PixiModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let bin = args.get_str("pixi_bin").unwrap_or("pixi").to_string();
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");
        let manifest = args.get_str("manifest_path").map(|s| s.to_string());
        let env = args
            .get_str("environment")
            .or_else(|| args.get_str("env"))
            .map(|s| s.to_string());
        let platform = args.get_str("platform").map(|s| s.to_string());
        let feature = args.get_str("feature").map(|s| s.to_string());
        let channels = collect_channels(args);
        let pypi = bool_arg(args, "pypi", false);
        let frozen = bool_arg(args, "frozen", false);
        let locked = bool_arg(args, "locked", false);
        let no_lockfile_update = bool_arg(args, "no_lockfile_update", false);

        match state {
            "init" => pixi_init(
                &bin,
                manifest.as_deref(),
                &channels,
                platform.as_deref(),
                host,
            ),
            "install" => pixi_install(
                &bin,
                manifest.as_deref(),
                env.as_deref(),
                frozen,
                locked,
                no_lockfile_update,
                host,
            ),
            "clean" => pixi_clean(&bin, manifest.as_deref(), host),
            "info" => pixi_info(&bin, manifest.as_deref(), host),
            "run" => {
                let cmd = args
                    .get_str("command")
                    .ok_or_else(|| anyhow::anyhow!("pixi: 'command' is required for state=run"))?;
                pixi_run(&bin, cmd, manifest.as_deref(), env.as_deref(), host)
            }
            "absent" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pixi: 'name' is required for state=absent".to_string(),
                    ));
                }
                pixi_remove(
                    &bin,
                    &packages,
                    manifest.as_deref(),
                    env.as_deref(),
                    platform.as_deref(),
                    feature.as_deref(),
                    pypi,
                    host,
                )
            }
            "latest" => {
                if packages.is_empty() {
                    return pixi_update_all(&bin, manifest.as_deref(), host);
                }
                pixi_update(&bin, &packages, manifest.as_deref(), host)
            }
            _ => {
                // present / add
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "pixi: 'name' is required for state=present".to_string(),
                    ));
                }
                pixi_add(
                    &bin,
                    &packages,
                    manifest.as_deref(),
                    env.as_deref(),
                    platform.as_deref(),
                    feature.as_deref(),
                    &channels,
                    pypi,
                    host,
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn manifest_args(manifest: Option<&str>) -> Vec<String> {
    if let Some(m) = manifest {
        vec!["--manifest-path".to_string(), m.to_string()]
    } else {
        vec![]
    }
}

fn env_args(env: Option<&str>) -> Vec<String> {
    if let Some(e) = env {
        vec!["--environment".to_string(), e.to_string()]
    } else {
        vec![]
    }
}

fn platform_args(platform: Option<&str>) -> Vec<String> {
    if let Some(p) = platform {
        vec!["--platform".to_string(), p.to_string()]
    } else {
        vec![]
    }
}

fn pixi_init(
    bin: &str,
    manifest: Option<&str>,
    channels: &[String],
    platform: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("init");
    cmd.args(manifest_args(manifest));
    for c in channels {
        cmd.args(["--channel", c]);
    }
    cmd.args(platform_args(platform));
    let out = cmd.output().context("pixi init")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = "pixi project initialized".into();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pixi init failed: {stderr}"),
        ))
    }
}

fn pixi_install(
    bin: &str,
    manifest: Option<&str>,
    env: Option<&str>,
    frozen: bool,
    locked: bool,
    no_lockfile_update: bool,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("install");
    cmd.args(manifest_args(manifest));
    cmd.args(env_args(env));
    if frozen {
        cmd.arg("--frozen");
    }
    if locked {
        cmd.arg("--locked");
    }
    if no_lockfile_update {
        cmd.arg("--no-lockfile-update");
    }
    let out = cmd.output().context("pixi install")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        // pixi install says "nothing to do" when already installed
        let changed = !stdout.contains("Nothing to do") && !stderr.contains("Nothing to do");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = if changed {
            "environment installed".into()
        } else {
            "environment already up-to-date".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pixi install failed: {stderr}"),
        ))
    }
}

fn pixi_add(
    bin: &str,
    packages: &[String],
    manifest: Option<&str>,
    env: Option<&str>,
    platform: Option<&str>,
    feature: Option<&str>,
    channels: &[String],
    pypi: bool,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("add");
    cmd.args(manifest_args(manifest));
    cmd.args(env_args(env));
    cmd.args(platform_args(platform));
    if let Some(f) = feature {
        cmd.args(["--feature", f]);
    }
    for c in channels {
        cmd.args(["--channel", c]);
    }
    if pypi {
        cmd.arg("--pypi");
    }
    for p in packages {
        cmd.arg(p);
    }

    let out = cmd.output().context("pixi add")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = format!("added: {}", packages.join(", "));
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pixi add failed: {stderr}"),
        ))
    }
}

fn pixi_remove(
    bin: &str,
    packages: &[String],
    manifest: Option<&str>,
    env: Option<&str>,
    platform: Option<&str>,
    feature: Option<&str>,
    pypi: bool,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("remove");
    cmd.args(manifest_args(manifest));
    cmd.args(env_args(env));
    cmd.args(platform_args(platform));
    if let Some(f) = feature {
        cmd.args(["--feature", f]);
    }
    if pypi {
        cmd.arg("--pypi");
    }
    for p in packages {
        cmd.arg(p);
    }

    let out = cmd.output().context("pixi remove")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = format!("removed: {}", packages.join(", "));
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pixi remove failed: {stderr}"),
        ))
    }
}

fn pixi_update(
    bin: &str,
    packages: &[String],
    manifest: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("update");
    cmd.args(manifest_args(manifest));
    for p in packages {
        cmd.arg(p);
    }
    let out = cmd.output().context("pixi update")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = format!("updated: {}", packages.join(", "));
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pixi update failed: {stderr}"),
        ))
    }
}

fn pixi_update_all(bin: &str, manifest: Option<&str>, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(["update", "--all"]);
    cmd.args(manifest_args(manifest));
    let out = cmd.output().context("pixi update --all")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = "all packages updated".into();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pixi update --all failed: {stderr}"),
        ))
    }
}

fn pixi_run(
    bin: &str,
    command: &str,
    manifest: Option<&str>,
    env: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("run");
    cmd.args(manifest_args(manifest));
    cmd.args(env_args(env));
    cmd.args(command.split_whitespace());
    let out = cmd.output().context("pixi run")?;
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
            format!("pixi run '{command}' failed (rc={rc}): {stderr}"),
        ))
    }
}

fn pixi_clean(bin: &str, manifest: Option<&str>, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.arg("clean");
    cmd.args(manifest_args(manifest));
    let out = cmd.output().context("pixi clean")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = "pixi environment cleaned".into();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("pixi clean failed: {stderr}"),
        ))
    }
}

fn pixi_info(bin: &str, manifest: Option<&str>, host: &str) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(["info", "--json"]);
    cmd.args(manifest_args(manifest));
    let out = cmd.output().context("pixi info")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let info: Value = serde_json::from_str(&stdout).unwrap_or(Value::String(stdout.clone()));
    let mut r = TaskResult::ok(host);
    r.stdout = stdout;
    r.vars.insert("info".into(), info);
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

fn collect_channels(args: &ModuleArgs) -> Vec<String> {
    if let Some(val) = args
        .args
        .get("channels")
        .or_else(|| args.args.get("channel"))
    {
        match val {
            Value::String(s) => s.split_whitespace().map(|s| s.to_string()).collect(),
            Value::Array(seq) => seq
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            _ => vec![],
        }
    } else {
        vec![]
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
    use ansiblers_core::{ExecutionContext, Inventory, Value};
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
        let r = PixiModule
            .invoke(&ModuleArgs::new(HashMap::new()), "h", &mut c)
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails() {
        let mut c = ctx();
        let r = PixiModule
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
        let r = PixiModule.invoke(
            &args(&[("state", Value::String("run".into()))]),
            "h",
            &mut c,
        );
        assert!(r.is_err());
    }
    #[test]
    fn test_manifest_args_some() {
        let v = manifest_args(Some("/project/pixi.toml"));
        assert_eq!(v, vec!["--manifest-path", "/project/pixi.toml"]);
    }
    #[test]
    fn test_manifest_args_none() {
        assert!(manifest_args(None).is_empty());
    }
    #[test]
    fn test_platform_args_wasm32() {
        let v = platform_args(Some("emscripten-wasm32"));
        assert_eq!(v, vec!["--platform", "emscripten-wasm32"]);
    }
    #[test]
    fn test_env_args() {
        let v = env_args(Some("dev"));
        assert_eq!(v, vec!["--environment", "dev"]);
    }
    #[test]
    fn test_collect_channels_array() {
        let a = args(&[(
            "channels",
            Value::Array(vec![
                Value::String("conda-forge".into()),
                Value::String("emscripten-forge".into()),
            ]),
        )]);
        assert_eq!(
            collect_channels(&a),
            vec!["conda-forge", "emscripten-forge"]
        );
    }
    #[test]
    fn test_pypi_default_false() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&a, "pypi", false));
    }
    #[test]
    fn test_frozen_locked_flags() {
        let a = args(&[("frozen", Value::Bool(true)), ("locked", Value::Bool(true))]);
        assert!(bool_arg(&a, "frozen", false));
        assert!(bool_arg(&a, "locked", false));
    }
}
