//! `conda` module — manage conda/mamba/micromamba environments and packages.
//!
//! Supports cross-platform builds via `--platform` (e.g. `emscripten-wasm32`,
//! `linux-aarch64`, `win-64`) which is essential for WASM and cross-compilation
//! workflows.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `name` / `pkg` | — | Package name(s); single string or list |
//! | `state` | `present` | `present`, `absent`, `latest`, `info`, `create`, `remove_env` |
//! | `environment` / `env` | — | Conda environment name or path to activate |
//! | `channels` | — | List of channels (e.g. `[conda-forge, bioconda]`) |
//! | `channel` | — | Single channel shorthand |
//! | `platform` | — | Target platform override (e.g. `emscripten-wasm32`, `linux-aarch64`) |
//! | `python_version` | — | Python version when creating an environment |
//! | `prefix` | — | Explicit environment prefix path |
//! | `file` | — | Path to an `environment.yml` to create/update from |
//! | `update_deps` | `false` | Pass `--update-deps` on install |
//! | `auto_activate_base` | — | Configure `auto_activate_base` (true/false) |
//! | `conda_bin` | `conda` | Binary to use (`conda`, `mamba`, `micromamba`) |
//!
//! ## Platform examples
//!
//! ```yaml
//! # Install for WebAssembly target (requires conda-forge channel)
//! - conda:
//!     name: python
//!     channels: [conda-forge, emscripten-forge]
//!     platform: emscripten-wasm32
//!     environment: wasm-env
//!
//! # Cross-build for aarch64 on x86_64 host
//! - conda:
//!     name: [numpy, scipy]
//!     channels: [conda-forge]
//!     platform: linux-aarch64
//!     environment: cross-env
//!
//! # Use micromamba
//! - conda:
//!     name: htop
//!     conda_bin: micromamba
//!     environment: base
//! ```

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct CondaModule;

impl ModuleInvoker for CondaModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let bin = args.get_str("conda_bin").unwrap_or("conda").to_string();
        let packages = collect_packages(args);
        let state = args.get_str("state").unwrap_or("present");
        let env = args
            .get_str("environment")
            .or_else(|| args.get_str("env"))
            .map(|s| s.to_string());
        let prefix = args.get_str("prefix").map(|s| s.to_string());
        let channels = collect_channels(args);
        let platform = args.get_str("platform").map(|s| s.to_string());
        let file = args.get_str("file").map(|s| s.to_string());
        let update_deps = bool_arg(args, "update_deps", false);

        match state {
            "create" => conda_create_env(
                &bin,
                &env,
                prefix.as_deref(),
                args.get_str("python_version"),
                &channels,
                platform.as_deref(),
                file.as_deref(),
                host,
            ),
            "remove_env" => conda_remove_env(&bin, &env, prefix.as_deref(), host),
            "absent" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "conda: 'name' is required for state=absent".to_string(),
                    ));
                }
                conda_uninstall(
                    &bin,
                    &packages,
                    &env,
                    prefix.as_deref(),
                    platform.as_deref(),
                    host,
                )
            }
            "latest" => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "conda: 'name' is required for state=latest".to_string(),
                    ));
                }
                conda_install(
                    &bin,
                    &packages,
                    &env,
                    prefix.as_deref(),
                    &channels,
                    platform.as_deref(),
                    true,
                    update_deps,
                    host,
                )
            }
            "info" => conda_info(&bin, &env, prefix.as_deref(), host),
            _ => {
                if packages.is_empty() {
                    return Ok(TaskResult::failed(
                        host,
                        "conda: 'name' is required for state=present".to_string(),
                    ));
                }
                conda_install(
                    &bin,
                    &packages,
                    &env,
                    prefix.as_deref(),
                    &channels,
                    platform.as_deref(),
                    false,
                    update_deps,
                    host,
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_env_args(env: &Option<String>, prefix: Option<&str>) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(p) = prefix {
        v.push("--prefix".into());
        v.push(p.to_string());
    } else if let Some(e) = env {
        v.push("--name".into());
        v.push(e.clone());
    }
    v
}

fn build_channel_args(channels: &[String]) -> Vec<String> {
    channels
        .iter()
        .flat_map(|c| vec!["--channel".to_string(), c.clone()])
        .collect()
}

fn build_platform_args(platform: Option<&str>) -> Vec<String> {
    if let Some(p) = platform {
        vec!["--platform".to_string(), p.to_string()]
    } else {
        vec![]
    }
}

fn conda_install(
    bin: &str,
    packages: &[String],
    env: &Option<String>,
    prefix: Option<&str>,
    channels: &[String],
    platform: Option<&str>,
    upgrade: bool,
    update_deps: bool,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(["install", "-y"]);
    cmd.args(build_env_args(env, prefix));
    cmd.args(build_channel_args(channels));
    cmd.args(build_platform_args(platform));
    if upgrade {
        cmd.arg("--update-all");
    }
    if update_deps {
        cmd.arg("--update-deps");
    }
    cmd.args(["--json"]);
    for p in packages {
        cmd.arg(p);
    }

    let out = cmd.output().context("conda install")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();

    if out.status.success() {
        // Check JSON output for actual changes.
        let changed = serde_json::from_str::<serde_json::Value>(&stdout)
            .map(|v| {
                v.get("actions")
                    .and_then(|a| a.get("INSTALL"))
                    .and_then(|i| i.as_array())
                    .map(|arr| !arr.is_empty())
                    .unwrap_or(true) // assume changed if we can't parse
            })
            .unwrap_or(true);

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
            format!("conda install failed: {stderr}"),
        ))
    }
}

fn conda_uninstall(
    bin: &str,
    packages: &[String],
    env: &Option<String>,
    prefix: Option<&str>,
    platform: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(["remove", "-y"]);
    cmd.args(build_env_args(env, prefix));
    cmd.args(build_platform_args(platform));
    cmd.arg("--json");
    for p in packages {
        cmd.arg(p);
    }

    let out = cmd.output().context("conda remove")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.stdout = stdout;
        r.msg = format!("removed: {}", packages.join(", "));
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("conda remove failed: {stderr}"),
        ))
    }
}

fn conda_create_env(
    bin: &str,
    env: &Option<String>,
    prefix: Option<&str>,
    python_version: Option<&str>,
    channels: &[String],
    platform: Option<&str>,
    file: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(["create", "-y"]);
    cmd.args(build_env_args(env, prefix));
    cmd.args(build_channel_args(channels));
    cmd.args(build_platform_args(platform));
    if let Some(f) = file {
        cmd.args(["--file", f]);
    }
    if let Some(py) = python_version {
        cmd.arg(format!("python={py}"));
    }

    let out = cmd.output().context("conda create")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = "environment created".into();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("conda create failed: {stderr}"),
        ))
    }
}

fn conda_remove_env(
    bin: &str,
    env: &Option<String>,
    prefix: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(["remove", "-y", "--all"]);
    cmd.args(build_env_args(env, prefix));
    let out = cmd.output().context("conda remove --all")?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.msg = "environment removed".into();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("conda remove env failed: {stderr}"),
        ))
    }
}

fn conda_info(
    bin: &str,
    env: &Option<String>,
    prefix: Option<&str>,
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = Command::new(bin);
    cmd.args(["info", "--json"]);
    cmd.args(build_env_args(env, prefix));
    let out = cmd.output().context("conda info")?;
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
        let r = CondaModule
            .invoke(&ModuleArgs::new(HashMap::new()), "h", &mut c)
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_no_name_absent_fails() {
        let mut c = ctx();
        let r = CondaModule
            .invoke(
                &args(&[("state", Value::String("absent".into()))]),
                "h",
                &mut c,
            )
            .unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_build_platform_args_wasm() {
        let v = build_platform_args(Some("emscripten-wasm32"));
        assert_eq!(v, vec!["--platform", "emscripten-wasm32"]);
    }
    #[test]
    fn test_build_platform_args_none() {
        let v = build_platform_args(None);
        assert!(v.is_empty());
    }
    #[test]
    fn test_collect_channels_single() {
        let a = args(&[("channel", Value::String("conda-forge".into()))]);
        assert_eq!(collect_channels(&a), vec!["conda-forge"]);
    }
    #[test]
    fn test_collect_channels_array() {
        let a = args(&[(
            "channels",
            Value::Array(vec![
                Value::String("conda-forge".into()),
                Value::String("bioconda".into()),
            ]),
        )]);
        assert_eq!(collect_channels(&a), vec!["conda-forge", "bioconda"]);
    }
    #[test]
    fn test_build_channel_args() {
        let v = build_channel_args(&["conda-forge".to_string(), "bioconda".to_string()]);
        assert_eq!(v, vec!["--channel", "conda-forge", "--channel", "bioconda"]);
    }
    #[test]
    fn test_build_env_args_name() {
        let env = Some("myenv".to_string());
        let v = build_env_args(&env, None);
        assert_eq!(v, vec!["--name", "myenv"]);
    }
    #[test]
    fn test_build_env_args_prefix_overrides_name() {
        let env = Some("myenv".to_string());
        let v = build_env_args(&env, Some("/opt/envs/myenv"));
        assert_eq!(v, vec!["--prefix", "/opt/envs/myenv"]);
    }
}
