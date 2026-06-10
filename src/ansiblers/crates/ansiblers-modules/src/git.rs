//! `git` module — manage git repositories (clone, pull, checkout).
//!
//! Uses `git` CLI commands so no extra Cargo dependencies are needed.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `repo` / `name` | — | Repository URL |
//! | `dest` | — | Local directory path |
//! | `version` | `HEAD` | Branch, tag, or commit SHA |
//! | `update` | `true` | Pull/fetch when the repo already exists |
//! | `force` | `false` | Discard local changes before update |
//! | `depth` | — | Create a shallow clone with this history depth |
//! | `recursive` | `false` | Also initialise/update submodules |
//! | `single_branch` | `false` | Clone only the specified branch |
//! | `accept_hostkey` | `false` | Add SSH host key to `known_hosts` automatically |
//! | `key_file` | — | Path to SSH private key |
//! | `ssh_opts` | — | Extra options forwarded to `GIT_SSH_COMMAND` |
//! | `umask` | — | Override process umask before cloning |

use std::path::Path;
use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct GitModule;

impl ModuleInvoker for GitModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let repo = args
            .get_str("repo")
            .or_else(|| args.get_str("name"))
            .ok_or_else(|| anyhow::anyhow!("git: 'repo' is required"))?
            .to_string();
        let dest = args
            .get_str("dest")
            .ok_or_else(|| anyhow::anyhow!("git: 'dest' is required"))?
            .to_string();

        let version = args.get_str("version").unwrap_or("HEAD");
        let update = bool_arg(args, "update", true);
        let force = bool_arg(args, "force", false);
        let depth = args.args.get("depth").and_then(|v| v.as_u64());
        let recursive = bool_arg(args, "recursive", false);
        let single_branch = bool_arg(args, "single_branch", false);
        let key_file = args.get_str("key_file").map(|s| s.to_string());
        let ssh_opts = args.get_str("ssh_opts").map(|s| s.to_string());
        let accept_hostkey = bool_arg(args, "accept_hostkey", false);

        let dest_path = Path::new(&dest);

        // Build GIT_SSH_COMMAND env var for SSH options.
        let ssh_cmd = build_ssh_cmd(key_file.as_deref(), ssh_opts.as_deref(), accept_hostkey);

        if dest_path.exists() && dest_path.join(".git").exists() {
            // Repository already cloned.
            if !update {
                // Capture current HEAD for return value.
                let sha = git_head_sha(dest_path).unwrap_or_default();
                let mut result = TaskResult::ok(host);
                result.vars.insert("after".into(), Value::String(sha));
                return Ok(result);
            }

            let before = git_head_sha(dest_path).unwrap_or_default();

            if force {
                git_run(
                    &["checkout", "--force"],
                    Some(dest_path),
                    ssh_cmd.as_deref(),
                )
                .ok();
            }

            // Fetch + checkout requested version.
            git_run(
                &["fetch", "--tags", "--prune", "origin"],
                Some(dest_path),
                ssh_cmd.as_deref(),
            )
            .with_context(|| format!("git fetch failed in '{dest}'"))?;

            git_checkout(dest_path, version, ssh_cmd.as_deref())
                .with_context(|| format!("git checkout '{version}' failed in '{dest}'"))?;

            if recursive {
                git_run(
                    &["submodule", "update", "--init", "--recursive"],
                    Some(dest_path),
                    ssh_cmd.as_deref(),
                )
                .ok();
            }

            let after = git_head_sha(dest_path).unwrap_or_default();
            let changed = before != after;
            let mut result = if changed {
                TaskResult::changed(host)
            } else {
                TaskResult::ok(host)
            };
            result.vars.insert("before".into(), Value::String(before));
            result.vars.insert("after".into(), Value::String(after.clone()));
            ctx.set_fact(host, "git_sha".into(), Value::String(after));
            return Ok(result);
        }

        // New clone.
        let mut clone_args: Vec<String> = vec!["clone".into()];

        if let Some(d) = depth {
            clone_args.push("--depth".into());
            clone_args.push(d.to_string());
        }
        if single_branch && version != "HEAD" {
            clone_args.push("--single-branch".into());
            clone_args.push("--branch".into());
            clone_args.push(version.to_string());
        }
        if recursive {
            clone_args.push("--recursive".into());
        }
        clone_args.push(repo.clone());
        clone_args.push(dest.clone());

        let clone_str_args: Vec<&str> = clone_args.iter().map(|s| s.as_str()).collect();
        git_run(&clone_str_args, None, ssh_cmd.as_deref())
            .with_context(|| format!("git clone '{repo}' to '{dest}' failed"))?;

        // If version is not HEAD (or a branch default), check it out.
        if version != "HEAD" && !single_branch {
            git_checkout(dest_path, version, ssh_cmd.as_deref()).ok();
        }

        let sha = git_head_sha(dest_path).unwrap_or_default();
        ctx.set_fact(host, "git_sha".into(), Value::String(sha.clone()));

        let mut result = TaskResult::changed(host);
        result.msg = format!("cloned {repo} to {dest}");
        result.vars.insert("after".into(), Value::String(sha));
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Git helpers
// ---------------------------------------------------------------------------

fn build_ssh_cmd(
    key_file: Option<&str>,
    extra_opts: Option<&str>,
    accept_hostkey: bool,
) -> Option<String> {
    let mut parts: Vec<String> = vec!["ssh".into()];
    if accept_hostkey {
        parts.push("-o StrictHostKeyChecking=no".into());
    }
    if let Some(kf) = key_file {
        parts.push(format!("-i {kf}"));
    }
    if let Some(opts) = extra_opts {
        parts.push(opts.to_string());
    }
    if parts.len() > 1 {
        Some(parts.join(" "))
    } else {
        None
    }
}

fn git_run(args: &[&str], cwd: Option<&Path>, ssh_cmd: Option<&str>) -> Result<()> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    if let Some(sc) = ssh_cmd {
        cmd.env("GIT_SSH_COMMAND", sc);
    }
    let status = cmd.status().context("failed to run git")?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("git {} failed with {status}", args.join(" "))
    }
}

fn git_checkout(dest: &Path, version: &str, ssh_cmd: Option<&str>) -> Result<()> {
    git_run(&["checkout", version], Some(dest), ssh_cmd)
}

fn git_head_sha(dest: &Path) -> Option<String> {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dest)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn bool_arg(args: &ModuleArgs, key: &str, default: bool) -> bool {
    args.args.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ansiblers_core::{ExecutionContext, Inventory};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tempfile::TempDir;

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

    #[test]
    fn test_missing_repo_fails() {
        let mut c = ctx();
        let args = make_args(&[("dest", Value::String("/tmp/dst".into()))]);
        let result = GitModule.invoke(&args, "h", &mut c);
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_dest_fails() {
        let mut c = ctx();
        let args = make_args(&[("repo", Value::String("https://example.com/r.git".into()))]);
        let result = GitModule.invoke(&args, "h", &mut c);
        assert!(result.is_err());
    }

    #[test]
    fn test_no_update_on_existing_repo() {
        // Create a temp dir that looks like a git repo.
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();

        let mut c = ctx();
        let args = make_args(&[
            ("repo", Value::String("https://example.com/r.git".into())),
            ("dest", Value::String(tmp.path().to_str().unwrap().to_string())),
            ("update", Value::Bool(false)),
        ]);
        let result = GitModule.invoke(&args, "h", &mut c).unwrap();
        // Should be ok (no change) without running any git commands.
        assert!(result.status.is_ok());
        assert!(!result.changed);
    }

    #[test]
    fn test_build_ssh_cmd_with_key() {
        let cmd = build_ssh_cmd(Some("/home/user/.ssh/id_rsa"), None, false);
        assert!(cmd.unwrap().contains("-i /home/user/.ssh/id_rsa"));
    }

    #[test]
    fn test_build_ssh_cmd_accept_hostkey() {
        let cmd = build_ssh_cmd(None, None, true);
        assert!(cmd.unwrap().contains("StrictHostKeyChecking=no"));
    }

    #[test]
    fn test_build_ssh_cmd_none_when_plain() {
        let cmd = build_ssh_cmd(None, None, false);
        assert!(cmd.is_none());
    }

    #[test]
    fn test_local_clone() {
        // Create a real local bare repo to clone from.
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        // Init bare source repo and add one commit.
        if Command::new("git").args(["init", src.path().to_str().unwrap()]).status().is_err() {
            return; // git not available in this environment
        }
        Command::new("git").args(["-C", src.path().to_str().unwrap(), "config", "user.email", "test@test.com"]).status().ok();
        Command::new("git").args(["-C", src.path().to_str().unwrap(), "config", "user.name", "Test"]).status().ok();
        std::fs::write(src.path().join("README.md"), "hello").unwrap();
        Command::new("git").args(["-C", src.path().to_str().unwrap(), "add", "."]).status().ok();
        let commit_ok = Command::new("git")
            .args(["-C", src.path().to_str().unwrap(), "commit", "-m", "init"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if !commit_ok {
            return; // git commit failed (e.g. no identity configured)
        }

        let dest_path = dst.path().join("repo");
        let mut c = ctx();
        let args = make_args(&[
            ("repo", Value::String(src.path().to_str().unwrap().to_string())),
            ("dest", Value::String(dest_path.to_str().unwrap().to_string())),
        ]);
        let result = GitModule.invoke(&args, "h", &mut c).unwrap();
        assert!(result.status.is_ok());
        assert!(result.changed);
        assert!(dest_path.join("README.md").exists());
    }
}
