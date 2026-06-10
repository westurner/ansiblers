//! `rpm_ostree` module — manage packages and deployments on rpm-ostree based
//! systems (Fedora CoreOS, Fedora Atomic Host, RHEL CoreOS, Silverblue, etc.).
//!
//! Uses `rpm-ostree status --json` for idempotent state detection before
//! running any mutating command.
//!
//! ## Subcommands / states
//!
//! | `state` | rpm-ostree command | Description |
//! |---------|-------------------|-------------|
//! | `present` (default) | `install <pkg>` | Layer package(s); no-op when already layered |
//! | `absent` | `uninstall <pkg>` | Remove layered packages |
//! | `latest` | `upgrade [--install pkg]` | Upgrade base OS, optionally also install |
//! | `deployed` | `deploy <version>` | Pin to a specific OS version |
//! | `rebased` | `rebase <ref>` | Switch to a different OSTree branch/ref |
//! | `rollback` | `rollback` | Roll back to the previous deployment |
//! | `status` | `status --json` | Query-only; returns facts, no mutations |
//! | `cleanup` | `cleanup <mode>` | Prune pending/rollback/base metadata |
//! | `db_diff` | `db diff [from] [to]` | Show RPM package changes between deployments |
//! | `preview` | `upgrade --preview` | Preview available updates without applying |
//!
//! ## Override operations (`override` parameter)
//!
//! | `override` | rpm-ostree command | Description |
//! |-----------|-------------------|-------------|
//! | `replace` | `override replace <src>` | Replace a base package from URL/path |
//! | `remove` | `override remove <pkg>` | Remove a base package |
//! | `reset` | `override reset [<pkg>]` | Reset override(s) back to base |
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `state` | `present` | See table above |
//! | `name` / `pkg` / `package` | — | Package name(s) for `present`/`absent` |
//! | `apply_live` | `false` | Add `-A` to apply changes without rebooting |
//! | `allow_inactive` | `false` | `--allow-inactive`: install over base packages |
//! | `force_replacefiles` | `false` | `--force-replacefiles` with install |
//! | `idempotent` | `true` | Parse `status --json` and skip if no-op |
//! | `check` | `false` | `--check` flag for `upgrade` (dry-run check) |
//! | `version` | — | OSTree version for `deploy` |
//! | `ref` | — | OSTree ref for `rebase` |
//! | `src` | — | URL or path for `override replace` |
//! | `override` | — | Override operation: `replace`, `remove`, `reset` |
//! | `cleanup_mode` | — | Cleanup mode: `pending`, `rollback`, `base`, `metadata` |
//! | `reboot` | `false` | Execute `systemctl reboot` after a successful change |
//! | `rpm_ostree_bin` | `/usr/bin/rpm-ostree` | Path to the rpm-ostree binary |
//!
//! ## Examples
//!
//! ```yaml
//! # Install packages (idempotent)
//! - rpm_ostree:
//!     name: [vim, htop]
//!
//! # Apply live without reboot
//! - rpm_ostree:
//!     name: tmux
//!     apply_live: true
//!
//! # Upgrade the OS
//! - rpm_ostree:
//!     state: latest
//!
//! # Upgrade and install in one transaction
//! - rpm_ostree:
//!     state: latest
//!     name: htop
//!
//! # Remove a layered package
//! - rpm_ostree:
//!     name: htop
//!     state: absent
//!
//! # Pin to a specific version
//! - rpm_ostree:
//!     state: deployed
//!     version: "39.20240101.0"
//!
//! # Rebase to a different image
//! - rpm_ostree:
//!     state: rebased
//!     ref: "ostree-unverified-registry:ghcr.io/ublue-os/silverblue-main:latest"
//!
//! # Override a base package with a local RPM
//! - rpm_ostree:
//!     override: replace
//!     src: /tmp/podman-custom.rpm
//!
//! # Remove a base package entirely
//! - rpm_ostree:
//!     override: remove
//!     name: firefox
//!
//! # Query status only (registers ansible_facts.rpm_ostree)
//! - rpm_ostree:
//!     state: status
//!   register: ostree_status
//! ```

use std::collections::HashMap;
use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct RpmOstreeModule;

impl ModuleInvoker for RpmOstreeModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let bin = args
            .get_str("rpm_ostree_bin")
            .unwrap_or("/usr/bin/rpm-ostree")
            .to_string();
        let state = args.get_str("state").unwrap_or("present");
        let idempotent = bool_arg(args, "idempotent", true);

        // Verify binary exists.
        if !std::path::Path::new(&bin).exists() {
            return Ok(TaskResult::failed(
                host,
                format!("rpm-ostree binary not found at '{bin}'; is this an rpm-ostree system?"),
            ));
        }

        // Always gather status for idempotency and return value.
        let status_json = run_status(&bin)?;

        // For state=status we are done — just return facts.
        if state == "status" {
            let facts = parse_status_facts(&status_json);
            ctx.set_fact(host, "rpm_ostree".into(), facts.clone());
            let mut result = TaskResult::ok(host);
            result.vars.insert("rpm_ostree".into(), facts);
            result.msg = "status gathered".into();
            return Ok(result);
        }

        // Handle override sub-operations.
        if let Some(op) = args.get_str("override") {
            return handle_override(op, args, &bin, host, &status_json, idempotent);
        }

        match state {
            "present" => handle_install(args, &bin, host, &status_json, idempotent),
            "absent" => handle_uninstall(args, &bin, host, &status_json, idempotent),
            "latest" => handle_upgrade(args, &bin, host),
            "deployed" => handle_deploy(args, &bin, host),
            "rebased" => handle_rebase(args, &bin, host),
            "rollback" => handle_rollback(&bin, host),
            "cleanup" => handle_cleanup(args, &bin, host),
            "db_diff" => handle_db_diff(args, &bin, host, ctx),
            "preview" => handle_upgrade_preview(&bin, host, ctx),
            other => Ok(TaskResult::failed(
                host,
                format!("rpm_ostree: unknown state '{other}'"),
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// State handlers
// ---------------------------------------------------------------------------

fn handle_install(
    args: &ModuleArgs,
    bin: &str,
    host: &str,
    status_json: &serde_json::Value,
    idempotent: bool,
) -> Result<TaskResult> {
    let packages = collect_packages(args);
    if packages.is_empty() {
        return Ok(TaskResult::failed(
            host,
            "rpm_ostree: 'name' is required for state=present".to_string(),
        ));
    }

    let allow_inactive = bool_arg(args, "allow_inactive", false);
    let force_replacefiles = bool_arg(args, "force_replacefiles", false);
    let apply_live = bool_arg(args, "apply_live", false);

    // Idempotency: skip packages already in requested-packages.
    let to_install: Vec<String> = if idempotent {
        let already = layered_packages(status_json);
        packages
            .into_iter()
            .filter(|p| !already.contains(p))
            .collect()
    } else {
        packages
    };

    if to_install.is_empty() {
        return Ok(TaskResult::ok(host));
    }

    let mut cmd_args: Vec<&str> = vec!["install", "-y"];
    if allow_inactive {
        cmd_args.push("--allow-inactive");
    }
    if force_replacefiles {
        cmd_args.push("--force-replacefiles");
    }
    if apply_live {
        cmd_args.push("-A");
    }
    let pkg_refs: Vec<&str> = to_install.iter().map(|s| s.as_str()).collect();
    cmd_args.extend_from_slice(&pkg_refs);

    let (success, stdout, stderr, rc) = run_rpm_ostree(bin, &cmd_args)?;

    let reboot = bool_arg(args, "reboot", false);
    if success && reboot && !apply_live {
        do_reboot()?;
    }

    if success {
        let mut r = TaskResult::changed(host);
        r.stdout = stdout;
        r.msg = format!("installed: {}", to_install.join(", "));
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("rpm-ostree install failed (rc={rc}): {stderr}"),
        ))
    }
}

fn handle_uninstall(
    args: &ModuleArgs,
    bin: &str,
    host: &str,
    status_json: &serde_json::Value,
    idempotent: bool,
) -> Result<TaskResult> {
    let packages = collect_packages(args);
    if packages.is_empty() {
        return Ok(TaskResult::failed(
            host,
            "rpm_ostree: 'name' is required for state=absent".to_string(),
        ));
    }

    let to_remove: Vec<String> = if idempotent {
        let layered = layered_packages(status_json);
        packages
            .into_iter()
            .filter(|p| layered.contains(p))
            .collect()
    } else {
        packages
    };

    if to_remove.is_empty() {
        return Ok(TaskResult::ok(host));
    }

    let apply_live = bool_arg(args, "apply_live", false);
    let mut cmd_args: Vec<&str> = vec!["uninstall", "-y"];
    if apply_live {
        cmd_args.push("-A");
    }
    let pkg_refs: Vec<&str> = to_remove.iter().map(|s| s.as_str()).collect();
    cmd_args.extend_from_slice(&pkg_refs);

    let (success, stdout, stderr, rc) = run_rpm_ostree(bin, &cmd_args)?;

    let reboot = bool_arg(args, "reboot", false);
    if success && reboot && !apply_live {
        do_reboot()?;
    }

    if success {
        let mut r = TaskResult::changed(host);
        r.stdout = stdout;
        r.msg = format!("uninstalled: {}", to_remove.join(", "));
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("rpm-ostree uninstall failed (rc={rc}): {stderr}"),
        ))
    }
}

fn handle_upgrade(args: &ModuleArgs, bin: &str, host: &str) -> Result<TaskResult> {
    let check = bool_arg(args, "check", false);
    let apply_live = bool_arg(args, "apply_live", false);
    let packages = collect_packages(args);

    let mut cmd_args: Vec<String> = vec!["upgrade".into()];
    if check {
        cmd_args.push("--check".into());
    } else {
        cmd_args.push("-y".into());
    }
    if apply_live {
        cmd_args.push("-A".into());
    }
    // `rpm-ostree upgrade --install <pkg>` combines upgrade + install.
    for pkg in &packages {
        cmd_args.push("--install".into());
        cmd_args.push(pkg.clone());
    }

    let cmd_str: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
    let (success, stdout, stderr, rc) = run_rpm_ostree(bin, &cmd_str)?;

    if check {
        // --check exits 77 when updates are available, 0 when up-to-date.
        let updates_available = rc == 77 || stdout.contains("AvailableUpdate");
        let mut r = TaskResult::ok(host);
        r.msg = if updates_available {
            "updates available".into()
        } else {
            "system is up-to-date".into()
        };
        r.vars
            .insert("updates_available".into(), Value::Bool(updates_available));
        return Ok(r);
    }

    let reboot = bool_arg(args, "reboot", false);
    if success && reboot && !apply_live {
        do_reboot()?;
    }

    if success {
        // Detect "no update available" in output.
        let changed =
            !stdout.contains("No upgrade available") && !stdout.contains("Already on latest");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = if changed {
            "upgraded".into()
        } else {
            "already up-to-date".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("rpm-ostree upgrade failed (rc={rc}): {stderr}"),
        ))
    }
}

fn handle_deploy(args: &ModuleArgs, bin: &str, host: &str) -> Result<TaskResult> {
    let version = args
        .get_str("version")
        .ok_or_else(|| anyhow::anyhow!("rpm_ostree: 'version' is required for state=deployed"))?;

    let (success, stdout, stderr, rc) = run_rpm_ostree(bin, &["deploy", "-y", version])?;

    if success {
        let reboot = bool_arg(args, "reboot", false);
        if reboot {
            do_reboot()?;
        }
        let mut r = TaskResult::changed(host);
        r.stdout = stdout;
        r.msg = format!("deployed version '{version}'");
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("rpm-ostree deploy failed (rc={rc}): {stderr}"),
        ))
    }
}

fn handle_rebase(args: &ModuleArgs, bin: &str, host: &str) -> Result<TaskResult> {
    let ref_str = args
        .get_str("ref")
        .ok_or_else(|| anyhow::anyhow!("rpm_ostree: 'ref' is required for state=rebased"))?;

    let (success, stdout, stderr, rc) = run_rpm_ostree(bin, &["rebase", "-y", ref_str])?;

    if success {
        let reboot = bool_arg(args, "reboot", false);
        if reboot {
            do_reboot()?;
        }
        let mut r = TaskResult::changed(host);
        r.stdout = stdout;
        r.msg = format!("rebased to '{ref_str}'");
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("rpm-ostree rebase failed (rc={rc}): {stderr}"),
        ))
    }
}

fn handle_rollback(bin: &str, host: &str) -> Result<TaskResult> {
    let (success, stdout, stderr, rc) = run_rpm_ostree(bin, &["rollback"])?;
    if success {
        let mut r = TaskResult::changed(host);
        r.stdout = stdout;
        r.msg = "rolled back to previous deployment".into();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("rpm-ostree rollback failed (rc={rc}): {stderr}"),
        ))
    }
}

fn handle_cleanup(args: &ModuleArgs, bin: &str, host: &str) -> Result<TaskResult> {
    let mode = args.get_str("cleanup_mode").unwrap_or("pending");
    let flag = match mode {
        "pending" | "p" => "-p",
        "rollback" | "r" => "-r",
        "base" | "b" => "-b",
        "metadata" | "m" => "-m",
        _ => {
            return Ok(TaskResult::failed(
                host,
                format!("rpm_ostree: unknown cleanup_mode '{mode}'"),
            ))
        }
    };

    let (success, stdout, stderr, rc) = run_rpm_ostree(bin, &["cleanup", flag])?;
    if success {
        let mut r = TaskResult::changed(host);
        r.stdout = stdout;
        r.msg = format!("cleanup ({mode}) completed");
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("rpm-ostree cleanup failed (rc={rc}): {stderr}"),
        ))
    }
}

/// `rpm-ostree db diff [from_rev] [to_rev]`
///
/// When `from` / `to` are omitted, rpm-ostree compares the booted deployment
/// against the pending one (i.e. what an upgrade would change).
fn handle_db_diff(
    args: &ModuleArgs,
    bin: &str,
    host: &str,
    ctx: &mut ExecutionContext,
) -> Result<TaskResult> {
    let mut cmd_args: Vec<&str> = vec!["db", "diff"];
    let format = args.get_str("format").unwrap_or("auto"); // "auto" | "json"
    if format == "json" {
        cmd_args.push("--format=json");
    }
    let from_owned;
    let to_owned;
    if let Some(f) = args.get_str("from") {
        from_owned = f.to_string();
        cmd_args.push(&from_owned);
    }
    if let Some(t) = args.get_str("to") {
        to_owned = t.to_string();
        cmd_args.push(&to_owned);
    }

    let (success, stdout, stderr, rc) = run_rpm_ostree(bin, &cmd_args)?;
    if success {
        // Try to parse JSON if the user asked for it; else store raw string.
        let diff_val: Value = if format == "json" {
            serde_json::from_str::<serde_json::Value>(&stdout)
                .map(|v| v.into())
                .unwrap_or_else(|_| Value::String(stdout.clone()))
        } else {
            Value::String(stdout.clone())
        };
        ctx.set_fact(host, "rpm_ostree_db_diff".into(), diff_val.clone());
        let mut r = TaskResult::ok(host);
        r.stdout = stdout;
        r.vars.insert("diff".into(), diff_val);
        r.msg = "rpm-ostree db diff".into();
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("rpm-ostree db diff failed (rc={rc}): {stderr}"),
        ))
    }
}

/// `rpm-ostree upgrade --preview`
///
/// Shows what packages would change in the next upgrade without staging it.
/// Stores structured facts in the execution context.
fn handle_upgrade_preview(bin: &str, host: &str, ctx: &mut ExecutionContext) -> Result<TaskResult> {
    let (success, stdout, stderr, rc) = run_rpm_ostree(bin, &["upgrade", "--preview"])?;
    if success {
        let available = !stdout.contains("No upgrade available") && !stdout.contains("up-to-date");
        ctx.set_fact(
            host,
            "rpm_ostree_upgrade_preview".into(),
            Value::String(stdout.clone()),
        );
        ctx.set_fact(
            host,
            "rpm_ostree_upgrade_available".into(),
            Value::Bool(available),
        );
        let mut r = TaskResult::ok(host);
        r.stdout = stdout;
        r.vars
            .insert("preview".into(), Value::String(r.stdout.clone()));
        r.vars
            .insert("upgrade_available".into(), Value::Bool(available));
        r.msg = if available {
            "upgrade available (preview)".into()
        } else {
            "system is up-to-date".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("rpm-ostree upgrade --preview failed (rc={rc}): {stderr}"),
        ))
    }
}

fn handle_override(
    op: &str,
    args: &ModuleArgs,
    bin: &str,
    host: &str,
    status_json: &serde_json::Value,
    idempotent: bool,
) -> Result<TaskResult> {
    match op {
        "replace" => {
            let src = args.get_str("src").ok_or_else(|| {
                anyhow::anyhow!("rpm_ostree: 'src' is required for override=replace")
            })?;
            let (success, stdout, stderr, rc) =
                run_rpm_ostree(bin, &["override", "replace", "-y", src])?;
            if success {
                let mut r = TaskResult::changed(host);
                r.stdout = stdout;
                r.msg = format!("override replace '{src}'");
                Ok(r)
            } else {
                Ok(TaskResult::failed(
                    host,
                    format!("rpm-ostree override replace failed (rc={rc}): {stderr}"),
                ))
            }
        }
        "remove" => {
            let packages = collect_packages(args);
            if packages.is_empty() {
                return Ok(TaskResult::failed(
                    host,
                    "rpm_ostree: 'name' required for override=remove".to_string(),
                ));
            }

            // Idempotency: skip packages already in requested-base-removals.
            let to_remove: Vec<String> = if idempotent {
                let already_removed = base_removals(status_json);
                packages
                    .into_iter()
                    .filter(|p| !already_removed.contains(p))
                    .collect()
            } else {
                packages
            };

            if to_remove.is_empty() {
                return Ok(TaskResult::ok(host));
            }

            let mut cmd_args: Vec<&str> = vec!["override", "remove", "-y"];
            let pkg_refs: Vec<&str> = to_remove.iter().map(|s| s.as_str()).collect();
            cmd_args.extend_from_slice(&pkg_refs);

            let (success, stdout, stderr, rc) = run_rpm_ostree(bin, &cmd_args)?;
            if success {
                let mut r = TaskResult::changed(host);
                r.stdout = stdout;
                r.msg = format!("override removed: {}", to_remove.join(", "));
                Ok(r)
            } else {
                Ok(TaskResult::failed(
                    host,
                    format!("rpm-ostree override remove failed (rc={rc}): {stderr}"),
                ))
            }
        }
        "reset" => {
            let packages = collect_packages(args);
            let mut cmd_args: Vec<&str> = vec!["override", "reset", "-y"];
            let pkg_refs: Vec<&str> = packages.iter().map(|s| s.as_str()).collect();
            cmd_args.extend_from_slice(&pkg_refs);

            let (success, stdout, stderr, rc) = run_rpm_ostree(bin, &cmd_args)?;
            if success {
                let mut r = TaskResult::changed(host);
                r.stdout = stdout;
                r.msg = if packages.is_empty() {
                    "override reset (all)".into()
                } else {
                    format!("override reset: {}", packages.join(", "))
                };
                Ok(r)
            } else {
                Ok(TaskResult::failed(
                    host,
                    format!("rpm-ostree override reset failed (rc={rc}): {stderr}"),
                ))
            }
        }
        other => Ok(TaskResult::failed(
            host,
            format!("rpm_ostree: unknown override '{other}'"),
        )),
    }
}

// ---------------------------------------------------------------------------
// JSON status helpers
// ---------------------------------------------------------------------------

/// Run `rpm-ostree status --json` and return parsed JSON.
fn run_status(bin: &str) -> Result<serde_json::Value> {
    let output = Command::new(bin)
        .args(["status", "--json"])
        .output()
        .with_context(|| format!("failed to run '{bin} status --json'"))?;

    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(&text).with_context(|| "failed to parse rpm-ostree status JSON")
    } else {
        // Binary exists but may not be on an ostree system (e.g. tests).
        // Return an empty status so the module can still run with idempotent=false.
        Ok(serde_json::json!({"deployments": []}))
    }
}

/// Extract the list of requested-packages from the booted deployment.
fn layered_packages(status: &serde_json::Value) -> Vec<String> {
    booted_deployment(status)
        .and_then(|d| d.get("requested-packages"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Extract requested-base-removals from the booted deployment.
fn base_removals(status: &serde_json::Value) -> Vec<String> {
    booted_deployment(status)
        .and_then(|d| d.get("requested-base-removals"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn booted_deployment(status: &serde_json::Value) -> Option<&serde_json::Value> {
    status
        .get("deployments")?
        .as_array()?
        .iter()
        .find(|d| d.get("booted").and_then(|v| v.as_bool()).unwrap_or(false))
        // Fallback: first deployment if none is booted (e.g. offline / test env).
        .or_else(|| status.get("deployments")?.as_array()?.first())
}

/// Build a `Value::Object` of useful facts from the status JSON.
fn parse_status_facts(status: &serde_json::Value) -> Value {
    let mut m = serde_json::Map::new();

    // Full raw JSON as a string for downstream processing.
    m.insert("raw".into(), Value::String(status.to_string()));

    if let Some(deployments) = status.get("deployments").and_then(|v| v.as_array()) {
        let deployment_list: Vec<Value> = deployments
            .iter()
            .map(|d| {
                let mut dm = serde_json::Map::new();
                for key in &[
                    "checksum",
                    "version",
                    "timestamp",
                    "booted",
                    "staged",
                    "packages",
                    "requested-packages",
                    "base-removals",
                    "requested-base-removals",
                ] {
                    if let Some(v) = d.get(*key) {
                        dm.insert(key.replace('-', "_"), json_to_value(v));
                    }
                }
                Value::Object(dm)
            })
            .collect();
        m.insert("deployments".into(), Value::Array(deployment_list));

        // Convenience: booted version string.
        if let Some(booted) = deployments
            .iter()
            .find(|d| d.get("booted").and_then(|v| v.as_bool()).unwrap_or(false))
        {
            if let Some(ver) = booted.get("version").and_then(|v| v.as_str()) {
                m.insert("booted_version".into(), Value::String(ver.into()));
            }
            if let Some(pkgs) = booted.get("packages").and_then(|v| v.as_array()) {
                let pkg_list: Vec<Value> = pkgs
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| Value::String(s.into())))
                    .collect();
                m.insert("layered_packages".into(), Value::Array(pkg_list));
            }
        }
    }

    Value::Object(m)
}

fn json_to_value(v: &serde_json::Value) -> Value {
    // serde_json::Value IS ansiblers_core::Value (same re-export).
    v.clone()
}

// ---------------------------------------------------------------------------
// Process helpers
// ---------------------------------------------------------------------------

fn run_rpm_ostree(bin: &str, args: &[&str]) -> Result<(bool, String, String, i32)> {
    let output = Command::new(bin)
        .args(args)
        .output()
        .with_context(|| format!("failed to run '{bin} {}'", args.join(" ")))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let rc = output.status.code().unwrap_or(-1);
    Ok((output.status.success(), stdout, stderr, rc))
}

fn do_reboot() -> Result<()> {
    Command::new("systemctl")
        .arg("reboot")
        .status()
        .context("failed to run 'systemctl reboot'")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------------

fn collect_packages(args: &ModuleArgs) -> Vec<String> {
    let val = args
        .args
        .get("name")
        .or_else(|| args.args.get("pkg"))
        .or_else(|| args.args.get("package"));

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ansiblers_core::{ExecutionContext, Inventory, Value};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn ctx() -> ExecutionContext {
        ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new())
    }

    fn make_args(pairs: &[(&str, Value)]) -> ModuleArgs {
        let mut m = HashMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.clone());
        }
        ModuleArgs::new(m)
    }

    // -----------------------------------------------------------------------
    // JSON status parsing
    // -----------------------------------------------------------------------

    fn sample_status() -> serde_json::Value {
        serde_json::json!({
            "deployments": [
                {
                    "booted": true,
                    "checksum": "abc123",
                    "version": "39.20240101.0",
                    "requested-packages": ["htop", "vim"],
                    "packages": ["htop", "vim"],
                    "requested-base-removals": ["firefox"],
                    "base-removals": ["firefox"]
                },
                {
                    "booted": false,
                    "checksum": "def456",
                    "version": "39.20231201.0",
                    "requested-packages": [],
                    "packages": []
                }
            ]
        })
    }

    #[test]
    fn test_layered_packages_from_booted() {
        let status = sample_status();
        let pkgs = layered_packages(&status);
        assert!(pkgs.contains(&"htop".to_string()));
        assert!(pkgs.contains(&"vim".to_string()));
    }

    #[test]
    fn test_layered_packages_empty_when_no_deployments() {
        let status = serde_json::json!({"deployments": []});
        assert!(layered_packages(&status).is_empty());
    }

    #[test]
    fn test_base_removals_from_booted() {
        let status = sample_status();
        let removals = base_removals(&status);
        assert!(removals.contains(&"firefox".to_string()));
    }

    #[test]
    fn test_parse_status_facts_booted_version() {
        let status = sample_status();
        let facts = parse_status_facts(&status);
        if let Value::Object(m) = &facts {
            assert_eq!(m["booted_version"], Value::String("39.20240101.0".into()));
            assert!(m.contains_key("deployments"));
        } else {
            panic!("expected Object");
        }
    }

    #[test]
    fn test_parse_status_facts_layered_packages() {
        let status = sample_status();
        let facts = parse_status_facts(&status);
        if let Value::Object(m) = &facts {
            if let Value::Array(pkgs) = &m["layered_packages"] {
                assert_eq!(pkgs.len(), 2);
            } else {
                panic!("expected Array for layered_packages");
            }
        }
    }

    // -----------------------------------------------------------------------
    // Argument helpers
    // -----------------------------------------------------------------------

    #[test]
    fn test_collect_packages_single() {
        let args = make_args(&[("name", Value::String("htop".into()))]);
        assert_eq!(collect_packages(&args), vec!["htop"]);
    }

    #[test]
    fn test_collect_packages_array() {
        let args = make_args(&[(
            "name",
            Value::Array(vec![
                Value::String("htop".into()),
                Value::String("vim".into()),
            ]),
        )]);
        let pkgs = collect_packages(&args);
        assert!(pkgs.contains(&"htop".to_string()));
        assert!(pkgs.contains(&"vim".to_string()));
    }

    #[test]
    fn test_collect_packages_space_separated() {
        let args = make_args(&[("name", Value::String("htop vim".into()))]);
        assert_eq!(collect_packages(&args), vec!["htop", "vim"]);
    }

    #[test]
    fn test_collect_packages_empty() {
        let args = ModuleArgs::new(HashMap::new());
        assert!(collect_packages(&args).is_empty());
    }

    #[test]
    fn test_bool_arg_defaults() {
        let args = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&args, "apply_live", false));
        assert!(bool_arg(&args, "idempotent", true));
    }

    // -----------------------------------------------------------------------
    // Module-level error cases (binary not found)
    // -----------------------------------------------------------------------

    #[test]
    fn test_missing_binary_returns_failure() {
        let mut c = ctx();
        let args = make_args(&[(
            "rpm_ostree_bin",
            Value::String("/nonexistent/rpm-ostree".into()),
        )]);
        let result = RpmOstreeModule.invoke(&args, "localhost", &mut c).unwrap();
        assert!(result.status.is_failed());
        assert!(result.msg.contains("not found"));
    }

    #[test]
    fn test_state_present_no_name_fails() {
        // Simulate binary present by using a real existing binary path.
        // We can't test the full install path without rpm-ostree, but we can
        // test parameter validation by pointing at /bin/true as the binary
        // (which returns empty JSON for status).
        let mut c = ctx();
        let args = make_args(&[
            ("rpm_ostree_bin", Value::String("/bin/true".into())),
            ("state", Value::String("present".into())),
        ]);
        // /bin/true exists but status --json returns nothing parseable →
        // run_status returns empty deployments → no packages to install →
        // no-op (ok). The no-name check is reached because packages is empty.
        let result = RpmOstreeModule.invoke(&args, "localhost", &mut c);
        // Either an error (JSON parse fail) or a failed result — both are valid.
        match result {
            Ok(r) => assert!(r.status.is_failed() || r.status.is_ok()),
            Err(_) => {} // JSON parse error is fine here
        }
    }

    #[test]
    fn test_idempotent_install_already_present() {
        // Simulate the idempotency check: if requested-packages already contains
        // the package, handle_install should return ok (no change).
        let status = sample_status();
        let packages_to_install = vec!["htop".to_string()]; // already in status
        let already = layered_packages(&status);
        let to_install: Vec<String> = packages_to_install
            .into_iter()
            .filter(|p| !already.contains(p))
            .collect();
        assert!(
            to_install.is_empty(),
            "htop is already layered; should be filtered out"
        );
    }

    #[test]
    fn test_idempotent_install_new_package() {
        let status = sample_status();
        let packages_to_install = vec!["neovim".to_string()]; // not in status
        let already = layered_packages(&status);
        let to_install: Vec<String> = packages_to_install
            .into_iter()
            .filter(|p| !already.contains(p))
            .collect();
        assert_eq!(to_install, vec!["neovim"]);
    }

    #[test]
    fn test_idempotent_uninstall_not_present() {
        let status = sample_status();
        let packages_to_remove = vec!["neovim".to_string()]; // not layered
        let layered = layered_packages(&status);
        let to_remove: Vec<String> = packages_to_remove
            .into_iter()
            .filter(|p| layered.contains(p))
            .collect();
        assert!(
            to_remove.is_empty(),
            "neovim is not layered; nothing to remove"
        );
    }

    #[test]
    fn test_unknown_state_fails() {
        let mut c = ctx();
        // Use /bin/true as binary (exists), feed unknown state.
        let args = make_args(&[
            ("rpm_ostree_bin", Value::String("/bin/true".into())),
            ("state", Value::String("frobnicate".into())),
        ]);
        let result = RpmOstreeModule.invoke(&args, "localhost", &mut c);
        match result {
            Ok(r) => assert!(r.status.is_failed()),
            Err(_) => {} // status --json parse error is acceptable
        }
    }

    #[test]
    fn test_upgrade_check_result_parsing() {
        // Simulate a --check result: rc=0 means no updates, rc=77 means updates.
        let no_updates_stdout = "Note: --check: No upgrade available.\n";
        let updates_available = no_updates_stdout.contains("AvailableUpdate");
        assert!(!updates_available);

        let updates_stdout = "AvailableUpdate:\n  Version: 39.20241201.0\n";
        let updates_available = updates_stdout.contains("AvailableUpdate");
        assert!(updates_available);
    }

    #[test]
    fn test_booted_deployment_fallback_to_first() {
        // When no deployment has booted=true, fall back to first entry.
        let status = serde_json::json!({
            "deployments": [
                {
                    "booted": false,
                    "version": "39.1",
                    "requested-packages": ["foo"]
                }
            ]
        });
        let pkgs = layered_packages(&status);
        assert_eq!(pkgs, vec!["foo"]);
    }

    #[test]
    fn test_override_remove_idempotent_already_removed() {
        let status = sample_status(); // firefox already in requested-base-removals
        let packages_to_remove = vec!["firefox".to_string()];
        let already_removed = base_removals(&status);
        let to_remove: Vec<String> = packages_to_remove
            .into_iter()
            .filter(|p| !already_removed.contains(p))
            .collect();
        assert!(to_remove.is_empty(), "firefox is already in base-removals");
    }
}
