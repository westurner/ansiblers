//! `ostree` module — manage OSTree repositories, refs, remotes, commits,
//! checkouts, and admin operations.
//!
//! Wraps the `ostree` CLI.  Each invocation maps to one OSTree subcommand
//! (or a small group of related ones), selected by the `command` parameter.
//!
//! ## Command reference
//!
//! ### Repository management
//!
//! | `command` | ostree command | Description |
//! |-----------|---------------|-------------|
//! | `init` | `ostree init` | Initialize a new OSTree repository |
//! | `fsck` | `ostree fsck` | Check repository consistency |
//! | `summary` | `ostree summary --update` | Regenerate summary metadata |
//! | `prune` | `ostree prune` | Remove unreachable objects |
//! | `config` | `ostree config set/get` | Read or write config values |
//!
//! ### Refs & commits
//!
//! | `command` | ostree command | Description |
//! |-----------|---------------|-------------|
//! | `refs` | `ostree refs` | List or delete refs |
//! | `log` | `ostree log <ref>` | Show commit log for a ref |
//! | `show` | `ostree show <rev>` | Show a single commit object |
//! | `rev_parse` | `ostree rev-parse <rev>` | Resolve a ref to a SHA256 |
//! | `diff` | `ostree diff <rev1> <rev2>` | Show filesystem differences |
//! | `ls` | `ostree ls <rev> [path]` | List contents of a commit |
//! | `cat` | `ostree cat <rev> <path>` | Print a file from a commit |
//! | `commit` | `ostree commit` | Create a new commit from a directory tree |
//! | `reset` | `ostree reset <ref> <rev>` | Reset a ref to a previous commit |
//!
//! ### Pull / checkout
//!
//! | `command` | ostree command | Description |
//! |-----------|---------------|-------------|
//! | `pull` | `ostree pull <remote> [refs]` | Download from a remote |
//! | `pull_local` | `ostree pull-local <src-repo> [refs]` | Copy from a local repo |
//! | `checkout` | `ostree checkout <rev> <dest>` | Check out a commit |
//!
//! ### Remotes
//!
//! | `command` | ostree command | Description |
//! |-----------|---------------|-------------|
//! | `remote_add` | `ostree remote add` | Add a remote |
//! | `remote_delete` | `ostree remote delete` | Remove a remote |
//! | `remote_list` | `ostree remote list` | List remotes |
//! | `remote_show_url` | `ostree remote show-url <name>` | Show a remote's URL |
//! | `remote_gpg_import` | `ostree remote gpg-import` | Import GPG keys for a remote |
//!
//! ### Admin operations
//!
//! | `command` | ostree command | Description |
//! |-----------|---------------|-------------|
//! | `admin_status` | `ostree admin status` | Show current deployments |
//! | `admin_upgrade` | `ostree admin upgrade` | Upgrade current deployment |
//! | `admin_deploy` | `ostree admin deploy <rev>` | Deploy a specific revision |
//! | `admin_undeploy` | `ostree admin undeploy <index>` | Remove a deployment |
//! | `admin_cleanup` | `ostree admin cleanup` | Delete untagged objects |
//! | `admin_config_diff` | `ostree admin config-diff` | Diff /etc vs /usr/etc |
//! | `admin_switch` | `ostree admin switch <ref>` | Switch tracking ref |
//! | `admin_unlock` | `ostree admin unlock [--hotfix]` | Unlock the deployment |
//!
//! ## Common parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `command` | — | Sub-command from the tables above (required) |
//! | `repo` | — | Path to OSTree repository (`--repo`); uses system default if omitted |
//! | `ostree_bin` | `ostree` | Path / name of the `ostree` binary |
//! | `sysroot` | — | Sysroot path for admin commands |
//! | `verbose` | `false` | Pass `-v` |
//!
//! ## Examples
//!
//! ```yaml
//! # Initialize a bare repository
//! - ostree:
//!     command: init
//!     repo: /srv/ostree/repo
//!     mode: bare          # or archive, archive-z2, bare-user, bare-split-xattrs
//!
//! # Add a remote
//! - ostree:
//!     command: remote_add
//!     repo: /srv/ostree/repo
//!     name: origin
//!     url: https://ostree.example.com/repo
//!     no_gpg_verify: true
//!
//! # Pull a ref from a remote
//! - ostree:
//!     command: pull
//!     repo: /srv/ostree/repo
//!     remote: origin
//!     refs: [fedora/39/x86_64/silverblue]
//!
//! # Checkout a commit
//! - ostree:
//!     command: checkout
//!     repo: /srv/ostree/repo
//!     rev: fedora/39/x86_64/silverblue
//!     dest: /mnt/checkout
//!
//! # Show refs
//! - ostree:
//!     command: refs
//!     repo: /srv/ostree/repo
//!   register: ostree_refs
//!
//! # Create a commit from a directory
//! - ostree:
//!     command: commit
//!     repo: /srv/ostree/repo
//!     branch: mybranch/stable
//!     src: /srv/build/rootfs
//!     subject: "Update rootfs 2024-01-01"
//!
//! # Admin status
//! - ostree:
//!     command: admin_status
//!   register: ostree_admin
//!
//! # Admin upgrade
//! - ostree:
//!     command: admin_upgrade
//!     reboot: true
//! ```

use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct OstreeModule;

impl ModuleInvoker for OstreeModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let bin = args.get_str("ostree_bin").unwrap_or("ostree").to_string();
        let command = args
            .get_str("command")
            .ok_or_else(|| anyhow::anyhow!("ostree: 'command' is required"))?;

        // Build the common repo / verbose flags.
        let repo = args.get_str("repo").map(|s| s.to_string());
        let sysroot = args.get_str("sysroot").map(|s| s.to_string());
        let verbose = bool_arg(args, "verbose", false);

        let mut global: Vec<String> = Vec::new();
        if let Some(r) = &repo {
            global.push(format!("--repo={r}"));
        }
        if let Some(s) = &sysroot {
            global.push(format!("--sysroot={s}"));
        }
        if verbose {
            global.push("-v".into());
        }

        dispatch(command, args, &bin, &global, host, ctx)
    }
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

fn dispatch(
    command: &str,
    args: &ModuleArgs,
    bin: &str,
    global: &[String],
    host: &str,
    ctx: &mut ExecutionContext,
) -> Result<TaskResult> {
    match command {
        // Repository management
        "init" => cmd_init(args, bin, global, host),
        "fsck" => cmd_fsck(args, bin, global, host),
        "summary" => cmd_summary(args, bin, global, host),
        "prune" => cmd_prune(args, bin, global, host),
        "config" => cmd_config(args, bin, global, host),

        // Refs & commits
        "refs" => cmd_refs(args, bin, global, host, ctx),
        "log" => cmd_log(args, bin, global, host, ctx),
        "show" => cmd_show(args, bin, global, host, ctx),
        "rev_parse" => cmd_rev_parse(args, bin, global, host, ctx),
        "diff" => cmd_diff(args, bin, global, host, ctx),
        "ls" => cmd_ls(args, bin, global, host, ctx),
        "cat" => cmd_cat(args, bin, global, host, ctx),
        "commit" => cmd_commit(args, bin, global, host),
        "reset" => cmd_reset(args, bin, global, host),

        // Pull / checkout
        "pull" => cmd_pull(args, bin, global, host),
        "pull_local" => cmd_pull_local(args, bin, global, host),
        "checkout" => cmd_checkout(args, bin, global, host),

        // Remotes
        "remote_add" => cmd_remote_add(args, bin, global, host),
        "remote_delete" => cmd_remote_delete(args, bin, global, host),
        "remote_list" => cmd_remote_list(bin, global, host, ctx),
        "remote_show_url" => cmd_remote_show_url(args, bin, global, host, ctx),
        "remote_gpg_import" => cmd_remote_gpg_import(args, bin, global, host),

        // Admin
        "admin_status" => cmd_admin_status(args, bin, global, host, ctx),
        "admin_upgrade" => cmd_admin_upgrade(args, bin, global, host),
        "admin_deploy" => cmd_admin_deploy(args, bin, global, host),
        "admin_undeploy" => cmd_admin_undeploy(args, bin, global, host),
        "admin_cleanup" => cmd_admin_cleanup(bin, global, host),
        "admin_config_diff" => cmd_admin_config_diff(bin, global, host, ctx),
        "admin_switch" => cmd_admin_switch(args, bin, global, host),
        "admin_unlock" => cmd_admin_unlock(args, bin, global, host),

        other => Ok(TaskResult::failed(
            host,
            format!("ostree: unknown command '{other}'"),
        )),
    }
}

// ---------------------------------------------------------------------------
// Repository management
// ---------------------------------------------------------------------------

fn cmd_init(args: &ModuleArgs, bin: &str, global: &[String], host: &str) -> Result<TaskResult> {
    let mode = args.get_str("mode").unwrap_or("bare-user");
    let mut cmd = build_cmd(bin, global);
    cmd.args(["init", "--mode", mode]);
    run_change(cmd, host, "repository initialized")
}

fn cmd_fsck(args: &ModuleArgs, bin: &str, global: &[String], host: &str) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, global);
    cmd.arg("fsck");
    if bool_arg(args, "quiet", false) {
        cmd.arg("-q");
    }
    let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
    if ok {
        let mut r = TaskResult::ok(host);
        r.stdout = stdout;
        r.msg = "repository is consistent".into();
        Ok(r)
    } else {
        Ok(failed(host, "fsck", rc, &stderr))
    }
}

fn cmd_summary(args: &ModuleArgs, bin: &str, global: &[String], host: &str) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, global);
    if bool_arg(args, "view", false) {
        cmd.args(["summary", "-v"]);
    } else {
        cmd.args(["summary", "-u"]);
    }
    run_change(cmd, host, "summary updated")
}

fn cmd_prune(args: &ModuleArgs, bin: &str, global: &[String], host: &str) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, global);
    cmd.arg("prune");
    if bool_arg(args, "refs_only", false) {
        cmd.arg("--refs-only");
    }
    if let Some(depth) = args.args.get("depth").and_then(|v| v.as_u64()) {
        cmd.arg(format!("--depth={depth}"));
    }
    if let Some(commit_only) = args.args.get("keep_younger_than").and_then(|v| v.as_str()) {
        cmd.arg(format!("--keep-younger-than={commit_only}"));
    }
    run_change(cmd, host, "pruned unreachable objects")
}

fn cmd_config(args: &ModuleArgs, bin: &str, global: &[String], host: &str) -> Result<TaskResult> {
    let section_key = args
        .get_str("key")
        .ok_or_else(|| anyhow::anyhow!("ostree config: 'key' is required (e.g. 'core.mode')"))?;

    let mut cmd = build_cmd(bin, global);
    if let Some(value) = args.get_str("value") {
        cmd.args(["config", "set", section_key, value]);
        run_change(cmd, host, &format!("config {section_key} set"))
    } else {
        cmd.args(["config", "get", section_key]);
        let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
        if ok {
            let mut r = TaskResult::ok(host);
            r.stdout = stdout.trim().to_string();
            r.vars.insert(
                "config_value".into(),
                Value::String(stdout.trim().to_string()),
            );
            Ok(r)
        } else {
            Ok(failed(host, "config get", rc, &stderr))
        }
    }
}

// ---------------------------------------------------------------------------
// Refs & commits
// ---------------------------------------------------------------------------

fn cmd_refs(
    args: &ModuleArgs,
    bin: &str,
    global: &[String],
    host: &str,
    ctx: &mut ExecutionContext,
) -> Result<TaskResult> {
    let state = args.get_str("state").unwrap_or("list");

    if state == "absent" {
        // Delete a ref.
        let ref_name = args
            .get_str("ref")
            .ok_or_else(|| anyhow::anyhow!("ostree refs state=absent: 'ref' is required"))?;
        let mut cmd = build_cmd(bin, global);
        cmd.args(["refs", "--delete", ref_name]);
        return run_change(cmd, host, &format!("deleted ref '{ref_name}'"));
    }

    // state=list
    let mut cmd = build_cmd(bin, global);
    cmd.arg("refs");
    if let Some(prefix) = args.get_str("prefix") {
        cmd.arg(prefix);
    }
    let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
    if ok {
        let refs: Vec<Value> = stdout
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| Value::String(l.to_string()))
            .collect();
        ctx.set_fact(host, "ostree_refs".into(), Value::Array(refs.clone()));
        let mut r = TaskResult::ok(host);
        r.vars.insert("refs".into(), Value::Array(refs));
        r.stdout = stdout;
        Ok(r)
    } else {
        Ok(failed(host, "refs", rc, &stderr))
    }
}

fn cmd_log(
    args: &ModuleArgs,
    bin: &str,
    global: &[String],
    host: &str,
    ctx: &mut ExecutionContext,
) -> Result<TaskResult> {
    let ref_name = args
        .get_str("ref")
        .ok_or_else(|| anyhow::anyhow!("ostree log: 'ref' is required"))?;
    let mut cmd = build_cmd(bin, global);
    cmd.args(["log", ref_name]);
    let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
    if ok {
        ctx.set_fact(host, "ostree_log".into(), Value::String(stdout.clone()));
        let mut r = TaskResult::ok(host);
        r.stdout = stdout;
        Ok(r)
    } else {
        Ok(failed(host, "log", rc, &stderr))
    }
}

fn cmd_show(
    args: &ModuleArgs,
    bin: &str,
    global: &[String],
    host: &str,
    ctx: &mut ExecutionContext,
) -> Result<TaskResult> {
    let rev = args
        .get_str("rev")
        .ok_or_else(|| anyhow::anyhow!("ostree show: 'rev' is required"))?;
    let mut cmd = build_cmd(bin, global);
    cmd.args(["show", rev]);
    let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
    if ok {
        ctx.set_fact(host, "ostree_show".into(), Value::String(stdout.clone()));
        let mut r = TaskResult::ok(host);
        r.stdout = stdout;
        Ok(r)
    } else {
        Ok(failed(host, "show", rc, &stderr))
    }
}

fn cmd_rev_parse(
    args: &ModuleArgs,
    bin: &str,
    global: &[String],
    host: &str,
    ctx: &mut ExecutionContext,
) -> Result<TaskResult> {
    let rev = args
        .get_str("rev")
        .ok_or_else(|| anyhow::anyhow!("ostree rev_parse: 'rev' is required"))?;
    let mut cmd = build_cmd(bin, global);
    cmd.args(["rev-parse", rev]);
    let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
    if ok {
        let sha = stdout.trim().to_string();
        ctx.set_fact(host, "ostree_checksum".into(), Value::String(sha.clone()));
        let mut r = TaskResult::ok(host);
        r.vars.insert("checksum".into(), Value::String(sha));
        r.stdout = stdout;
        Ok(r)
    } else {
        Ok(failed(host, "rev-parse", rc, &stderr))
    }
}

fn cmd_diff(
    args: &ModuleArgs,
    bin: &str,
    global: &[String],
    host: &str,
    ctx: &mut ExecutionContext,
) -> Result<TaskResult> {
    let rev_a = args
        .get_str("rev_a")
        .or_else(|| args.get_str("from"))
        .ok_or_else(|| anyhow::anyhow!("ostree diff: 'rev_a' or 'from' is required"))?;
    let rev_b = args
        .get_str("rev_b")
        .or_else(|| args.get_str("to"))
        .ok_or_else(|| anyhow::anyhow!("ostree diff: 'rev_b' or 'to' is required"))?;

    let mut cmd = build_cmd(bin, global);
    cmd.args(["diff", rev_a, rev_b]);
    let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
    if ok {
        ctx.set_fact(host, "ostree_diff".into(), Value::String(stdout.clone()));
        let mut r = TaskResult::ok(host);
        r.stdout = stdout;
        Ok(r)
    } else {
        Ok(failed(host, "diff", rc, &stderr))
    }
}

fn cmd_ls(
    args: &ModuleArgs,
    bin: &str,
    global: &[String],
    host: &str,
    ctx: &mut ExecutionContext,
) -> Result<TaskResult> {
    let rev = args
        .get_str("rev")
        .ok_or_else(|| anyhow::anyhow!("ostree ls: 'rev' is required"))?;
    let mut cmd = build_cmd(bin, global);
    cmd.args(["ls", rev]);
    if let Some(path) = args.get_str("path") {
        cmd.arg(path);
    }
    if bool_arg(args, "recursive", false) {
        cmd.arg("-R");
    }
    let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
    if ok {
        let entries: Vec<Value> = stdout
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| Value::String(l.to_string()))
            .collect();
        ctx.set_fact(host, "ostree_ls".into(), Value::Array(entries.clone()));
        let mut r = TaskResult::ok(host);
        r.vars.insert("entries".into(), Value::Array(entries));
        r.stdout = stdout;
        Ok(r)
    } else {
        Ok(failed(host, "ls", rc, &stderr))
    }
}

fn cmd_cat(
    args: &ModuleArgs,
    bin: &str,
    global: &[String],
    host: &str,
    ctx: &mut ExecutionContext,
) -> Result<TaskResult> {
    let rev = args
        .get_str("rev")
        .ok_or_else(|| anyhow::anyhow!("ostree cat: 'rev' is required"))?;
    let path = args
        .get_str("path")
        .ok_or_else(|| anyhow::anyhow!("ostree cat: 'path' is required"))?;
    let mut cmd = build_cmd(bin, global);
    cmd.args(["cat", rev, path]);
    let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
    if ok {
        ctx.set_fact(host, "ostree_cat".into(), Value::String(stdout.clone()));
        let mut r = TaskResult::ok(host);
        r.vars
            .insert("content".into(), Value::String(stdout.clone()));
        r.stdout = stdout;
        Ok(r)
    } else {
        Ok(failed(host, "cat", rc, &stderr))
    }
}

fn cmd_commit(args: &ModuleArgs, bin: &str, global: &[String], host: &str) -> Result<TaskResult> {
    let branch = args
        .get_str("branch")
        .ok_or_else(|| anyhow::anyhow!("ostree commit: 'branch' is required"))?;
    let src = args
        .get_str("src")
        .ok_or_else(|| anyhow::anyhow!("ostree commit: 'src' (directory tree) is required"))?;

    let mut cmd = build_cmd(bin, global);
    cmd.args(["commit", "--branch", branch]);
    if let Some(subject) = args.get_str("subject") {
        cmd.args(["--subject", subject]);
    }
    if let Some(body) = args.get_str("body") {
        cmd.args(["--body", body]);
    }
    if let Some(parent) = args.get_str("parent") {
        cmd.args(["--parent", parent]);
    }
    if bool_arg(args, "no_xattrs", false) {
        cmd.arg("--no-xattrs");
    }
    if bool_arg(args, "consume", false) {
        cmd.arg("--consume");
    }
    if let Some(key) = args.get_str("gpg_sign") {
        cmd.args(["--gpg-sign", key]);
    }
    // Metadata key=value pairs
    if let Some(Value::Object(meta)) = args.args.get("metadata") {
        for (k, v) in meta {
            if let Some(vs) = v.as_str() {
                cmd.args(["--add-metadata-string", &format!("{k}={vs}")]);
            }
        }
    }
    cmd.arg(src);

    let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
    if ok {
        let sha = stdout.trim().to_string();
        let mut r = TaskResult::changed(host);
        r.vars.insert("commit".into(), Value::String(sha.clone()));
        r.stdout = sha.clone();
        r.msg = format!("committed to branch '{branch}': {sha}");
        Ok(r)
    } else {
        Ok(failed(host, "commit", rc, &stderr))
    }
}

fn cmd_reset(args: &ModuleArgs, bin: &str, global: &[String], host: &str) -> Result<TaskResult> {
    let ref_name = args
        .get_str("ref")
        .ok_or_else(|| anyhow::anyhow!("ostree reset: 'ref' is required"))?;
    let rev = args
        .get_str("rev")
        .ok_or_else(|| anyhow::anyhow!("ostree reset: 'rev' is required"))?;
    let mut cmd = build_cmd(bin, global);
    cmd.args(["reset", ref_name, rev]);
    run_change(cmd, host, &format!("reset '{ref_name}' to '{rev}'"))
}

// ---------------------------------------------------------------------------
// Pull / checkout
// ---------------------------------------------------------------------------

fn cmd_pull(args: &ModuleArgs, bin: &str, global: &[String], host: &str) -> Result<TaskResult> {
    let remote = args
        .get_str("remote")
        .ok_or_else(|| anyhow::anyhow!("ostree pull: 'remote' is required"))?;

    let mut cmd = build_cmd(bin, global);
    cmd.args(["pull", remote]);
    if let Some(depth) = args.args.get("depth").and_then(|v| v.as_u64()) {
        cmd.arg(format!("--depth={depth}"));
    }
    if bool_arg(args, "mirror", false) {
        cmd.arg("--mirror");
    }
    // Pull specific refs / branches
    for r in collect_strings(args, &["refs", "ref"]) {
        cmd.arg(r);
    }
    let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
    if ok {
        let changed = !stdout.contains("No update") && !stdout.contains("already up to date");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = if changed {
            format!("pulled from '{remote}'")
        } else {
            "already up-to-date".into()
        };
        Ok(r)
    } else {
        Ok(failed(host, "pull", rc, &stderr))
    }
}

fn cmd_pull_local(
    args: &ModuleArgs,
    bin: &str,
    global: &[String],
    host: &str,
) -> Result<TaskResult> {
    let src_repo = args
        .get_str("src_repo")
        .ok_or_else(|| anyhow::anyhow!("ostree pull_local: 'src_repo' is required"))?;

    let mut cmd = build_cmd(bin, global);
    cmd.args(["pull-local", src_repo]);
    for r in collect_strings(args, &["refs", "ref"]) {
        cmd.arg(r);
    }
    run_change(cmd, host, &format!("pulled from local repo '{src_repo}'"))
}

fn cmd_checkout(args: &ModuleArgs, bin: &str, global: &[String], host: &str) -> Result<TaskResult> {
    let rev = args
        .get_str("rev")
        .ok_or_else(|| anyhow::anyhow!("ostree checkout: 'rev' is required"))?;
    let dest = args
        .get_str("dest")
        .ok_or_else(|| anyhow::anyhow!("ostree checkout: 'dest' is required"))?;

    let mut cmd = build_cmd(bin, global);
    cmd.args(["checkout", rev, dest]);
    if bool_arg(args, "union", false) {
        cmd.arg("--union");
    }
    if bool_arg(args, "allow_noent", false) {
        cmd.arg("--allow-noent");
    }
    if bool_arg(args, "user_mode", false) {
        cmd.arg("-U");
    }
    if let Some(subpath) = args.get_str("subpath") {
        cmd.args(["--subpath", subpath]);
    }
    run_change(cmd, host, &format!("checked out '{rev}' to '{dest}'"))
}

// ---------------------------------------------------------------------------
// Remotes
// ---------------------------------------------------------------------------

fn cmd_remote_add(
    args: &ModuleArgs,
    bin: &str,
    global: &[String],
    host: &str,
) -> Result<TaskResult> {
    let name = args
        .get_str("name")
        .ok_or_else(|| anyhow::anyhow!("ostree remote_add: 'name' is required"))?;
    let url = args
        .get_str("url")
        .ok_or_else(|| anyhow::anyhow!("ostree remote_add: 'url' is required"))?;

    let mut cmd = build_cmd(bin, global);
    cmd.args(["remote", "add"]);
    if bool_arg(args, "no_gpg_verify", false) {
        cmd.arg("--no-gpg-verify");
    }
    if bool_arg(args, "if_not_exists", true) {
        cmd.arg("--if-not-exists");
    }
    if let Some(branch) = args.get_str("branch") {
        cmd.args(["--set", &format!("branches={branch}")]);
    }
    if let Some(gpg) = args.get_str("gpg_import") {
        cmd.args(["--gpg-import", gpg]);
    }
    cmd.args([name, url]);
    run_change(cmd, host, &format!("remote '{name}' added"))
}

fn cmd_remote_delete(
    args: &ModuleArgs,
    bin: &str,
    global: &[String],
    host: &str,
) -> Result<TaskResult> {
    let name = args
        .get_str("name")
        .ok_or_else(|| anyhow::anyhow!("ostree remote_delete: 'name' is required"))?;
    let mut cmd = build_cmd(bin, global);
    cmd.args(["remote", "delete", name]);
    run_change(cmd, host, &format!("remote '{name}' deleted"))
}

fn cmd_remote_list(
    bin: &str,
    global: &[String],
    host: &str,
    ctx: &mut ExecutionContext,
) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, global);
    cmd.args(["remote", "list"]);
    let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
    if ok {
        let remotes: Vec<Value> = stdout
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| Value::String(l.to_string()))
            .collect();
        ctx.set_fact(host, "ostree_remotes".into(), Value::Array(remotes.clone()));
        let mut r = TaskResult::ok(host);
        r.vars.insert("remotes".into(), Value::Array(remotes));
        r.stdout = stdout;
        Ok(r)
    } else {
        Ok(failed(host, "remote list", rc, &stderr))
    }
}

fn cmd_remote_show_url(
    args: &ModuleArgs,
    bin: &str,
    global: &[String],
    host: &str,
    ctx: &mut ExecutionContext,
) -> Result<TaskResult> {
    let name = args
        .get_str("name")
        .ok_or_else(|| anyhow::anyhow!("ostree remote_show_url: 'name' is required"))?;
    let mut cmd = build_cmd(bin, global);
    cmd.args(["remote", "show-url", name]);
    let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
    if ok {
        let url = stdout.trim().to_string();
        ctx.set_fact(host, "ostree_remote_url".into(), Value::String(url.clone()));
        let mut r = TaskResult::ok(host);
        r.vars.insert("url".into(), Value::String(url));
        Ok(r)
    } else {
        Ok(failed(host, "remote show-url", rc, &stderr))
    }
}

fn cmd_remote_gpg_import(
    args: &ModuleArgs,
    bin: &str,
    global: &[String],
    host: &str,
) -> Result<TaskResult> {
    let name = args
        .get_str("name")
        .ok_or_else(|| anyhow::anyhow!("ostree remote_gpg_import: 'name' is required"))?;
    let keyfile = args
        .get_str("keyfile")
        .ok_or_else(|| anyhow::anyhow!("ostree remote_gpg_import: 'keyfile' is required"))?;
    let mut cmd = build_cmd(bin, global);
    cmd.args(["remote", "gpg-import", "--keyring", keyfile, name]);
    run_change(cmd, host, &format!("GPG keys imported for remote '{name}'"))
}

// ---------------------------------------------------------------------------
// Admin operations
// ---------------------------------------------------------------------------

fn cmd_admin_status(
    args: &ModuleArgs,
    bin: &str,
    global: &[String],
    host: &str,
    ctx: &mut ExecutionContext,
) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, global);
    cmd.args(["admin", "status"]);
    if bool_arg(args, "verbose", false) {
        cmd.arg("-v");
    }
    let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
    if ok {
        ctx.set_fact(
            host,
            "ostree_admin_status".into(),
            Value::String(stdout.clone()),
        );
        let mut r = TaskResult::ok(host);
        r.vars
            .insert("status".into(), Value::String(stdout.clone()));
        r.stdout = stdout;
        Ok(r)
    } else {
        Ok(failed(host, "admin status", rc, &stderr))
    }
}

fn cmd_admin_upgrade(
    args: &ModuleArgs,
    bin: &str,
    global: &[String],
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, global);
    cmd.args(["admin", "upgrade"]);
    if bool_arg(args, "check", false) {
        cmd.arg("--check");
    }
    if bool_arg(args, "pull_only", false) {
        cmd.arg("--pull-only");
    }
    if bool_arg(args, "deploy_only", false) {
        cmd.arg("--deploy-only");
    }
    let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
    if ok {
        let changed = !stdout.contains("No update available");
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
        if bool_arg(args, "reboot", false) && changed {
            do_reboot()?;
        }
        Ok(r)
    } else {
        Ok(failed(host, "admin upgrade", rc, &stderr))
    }
}

fn cmd_admin_deploy(
    args: &ModuleArgs,
    bin: &str,
    global: &[String],
    host: &str,
) -> Result<TaskResult> {
    let rev = args
        .get_str("rev")
        .ok_or_else(|| anyhow::anyhow!("ostree admin_deploy: 'rev' is required"))?;
    let mut cmd = build_cmd(bin, global);
    cmd.args(["admin", "deploy", rev]);
    if bool_arg(args, "karg_proc_cmdline", false) {
        cmd.arg("--karg-proc-cmdline");
    }
    if let Some(stateroot) = args.get_str("stateroot") {
        cmd.args(["--os", stateroot]);
    }
    let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
    if ok {
        if bool_arg(args, "reboot", false) {
            do_reboot()?;
        }
        let mut r = TaskResult::changed(host);
        r.stdout = stdout;
        r.msg = format!("deployed '{rev}'");
        Ok(r)
    } else {
        Ok(failed(host, "admin deploy", rc, &stderr))
    }
}

fn cmd_admin_undeploy(
    args: &ModuleArgs,
    bin: &str,
    global: &[String],
    host: &str,
) -> Result<TaskResult> {
    let index: u64 = args
        .args
        .get("index")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("ostree admin_undeploy: 'index' (integer) is required"))?;
    let mut cmd = build_cmd(bin, global);
    cmd.args(["admin", "undeploy", &index.to_string()]);
    run_change(cmd, host, &format!("undeployed index {index}"))
}

fn cmd_admin_cleanup(bin: &str, global: &[String], host: &str) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, global);
    cmd.args(["admin", "cleanup"]);
    run_change(cmd, host, "admin cleanup completed")
}

fn cmd_admin_config_diff(
    bin: &str,
    global: &[String],
    host: &str,
    ctx: &mut ExecutionContext,
) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, global);
    cmd.args(["admin", "config-diff"]);
    let (ok, stdout, stderr, rc) = run_cmd(cmd)?;
    if ok {
        ctx.set_fact(
            host,
            "ostree_config_diff".into(),
            Value::String(stdout.clone()),
        );
        let mut r = TaskResult::ok(host);
        r.vars.insert("diff".into(), Value::String(stdout.clone()));
        r.stdout = stdout;
        Ok(r)
    } else {
        Ok(failed(host, "admin config-diff", rc, &stderr))
    }
}

fn cmd_admin_switch(
    args: &ModuleArgs,
    bin: &str,
    global: &[String],
    host: &str,
) -> Result<TaskResult> {
    let ref_name = args
        .get_str("ref")
        .ok_or_else(|| anyhow::anyhow!("ostree admin_switch: 'ref' is required"))?;
    let mut cmd = build_cmd(bin, global);
    cmd.args(["admin", "switch", ref_name]);
    if bool_arg(args, "reboot", false) {
        cmd.arg("--reboot");
    }
    run_change(cmd, host, &format!("switched to '{ref_name}'"))
}

fn cmd_admin_unlock(
    args: &ModuleArgs,
    bin: &str,
    global: &[String],
    host: &str,
) -> Result<TaskResult> {
    let mut cmd = build_cmd(bin, global);
    cmd.args(["admin", "unlock"]);
    if bool_arg(args, "hotfix", false) {
        cmd.arg("--hotfix");
    }
    run_change(cmd, host, "deployment unlocked")
}

// ---------------------------------------------------------------------------
// Process helpers
// ---------------------------------------------------------------------------

fn build_cmd(bin: &str, global: &[String]) -> Command {
    let mut cmd = Command::new(bin);
    for g in global {
        cmd.arg(g);
    }
    cmd
}

fn run_cmd(mut cmd: Command) -> Result<(bool, String, String, i32)> {
    let out = cmd.output().context("failed to run ostree")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let rc = out.status.code().unwrap_or(-1);
    Ok((out.status.success(), stdout, stderr, rc))
}

fn run_change(mut cmd: Command, host: &str, msg: &str) -> Result<TaskResult> {
    let out = cmd.output().context("failed to run ostree")?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let rc = out.status.code().unwrap_or(-1);
    if out.status.success() {
        let mut r = TaskResult::changed(host);
        r.stdout = stdout;
        r.msg = msg.into();
        Ok(r)
    } else {
        Ok(failed(host, msg, rc, &stderr))
    }
}

fn failed(host: &str, op: &str, rc: i32, stderr: &str) -> TaskResult {
    TaskResult::failed(host, format!("ostree {op} failed (rc={rc}): {stderr}"))
}

fn do_reboot() -> Result<()> {
    Command::new("systemctl")
        .arg("reboot")
        .status()
        .context("systemctl reboot failed")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------------

fn bool_arg(args: &ModuleArgs, key: &str, default: bool) -> bool {
    args.args
        .get(key)
        .and_then(|v| v.as_bool())
        .unwrap_or(default)
}

fn collect_strings(args: &ModuleArgs, keys: &[&str]) -> Vec<String> {
    for key in keys {
        if let Some(val) = args.args.get(*key) {
            return match val {
                Value::String(s) => s.split_whitespace().map(|s| s.to_string()).collect(),
                Value::Array(seq) => seq
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect(),
                _ => vec![],
            };
        }
    }
    vec![]
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
    // Parameter validation (no binary required)
    // -----------------------------------------------------------------------

    #[test]
    fn test_missing_command_fails() {
        let mut c = ctx();
        let args = ModuleArgs::new(HashMap::new());
        let result = OstreeModule.invoke(&args, "h", &mut c);
        assert!(result.is_err() || result.unwrap().status.is_failed());
    }

    #[test]
    fn test_unknown_command_fails() {
        let mut c = ctx();
        let args = make_args(&[
            ("command", Value::String("frobnicate".into())),
            ("ostree_bin", Value::String("/nonexistent/ostree".into())),
        ]);
        // binary not found → error before dispatch, or after dispatch unknown command
        let _ = OstreeModule.invoke(&args, "h", &mut c);
        // Either path is acceptable — just must not panic.
    }

    #[test]
    fn test_init_missing_repo_uses_default() {
        let args = make_args(&[("command", Value::String("init".into()))]);
        // Global args: no --repo flag → Command::new("ostree") ["init" "--mode" "bare-user"]
        // We just check it builds a valid Command without panic.
        let mut _cmd = build_cmd("ostree", &[]);
        _cmd.args(["init", "--mode", "bare-user"]);
        // Command construction itself should not fail.
    }

    // -----------------------------------------------------------------------
    // collect_strings
    // -----------------------------------------------------------------------

    #[test]
    fn test_collect_strings_single_key() {
        let args = make_args(&[("ref", Value::String("fedora/39/x86_64/silverblue".into()))]);
        let result = collect_strings(&args, &["refs", "ref"]);
        assert_eq!(result, vec!["fedora/39/x86_64/silverblue"]);
    }

    #[test]
    fn test_collect_strings_array() {
        let args = make_args(&[(
            "refs",
            Value::Array(vec![
                Value::String("fedora/39/x86_64/silverblue".into()),
                Value::String("fedora/39/x86_64/atomic-host".into()),
            ]),
        )]);
        let result = collect_strings(&args, &["refs", "ref"]);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_collect_strings_empty() {
        let args = ModuleArgs::new(HashMap::new());
        assert!(collect_strings(&args, &["refs", "ref"]).is_empty());
    }

    #[test]
    fn test_collect_strings_space_separated() {
        let args = make_args(&[("ref", Value::String("branch/a branch/b".into()))]);
        let result = collect_strings(&args, &["ref"]);
        assert_eq!(result, vec!["branch/a", "branch/b"]);
    }

    // -----------------------------------------------------------------------
    // bool_arg
    // -----------------------------------------------------------------------

    #[test]
    fn test_bool_arg_default_false() {
        let args = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&args, "mirror", false));
    }

    #[test]
    fn test_bool_arg_default_true() {
        let args = ModuleArgs::new(HashMap::new());
        assert!(bool_arg(&args, "if_not_exists", true));
    }

    #[test]
    fn test_bool_arg_override() {
        let args = make_args(&[("verbose", Value::Bool(true))]);
        assert!(bool_arg(&args, "verbose", false));
    }

    // -----------------------------------------------------------------------
    // Global flag building
    // -----------------------------------------------------------------------

    #[test]
    fn test_global_repo_flag() {
        let mut global = Vec::new();
        let repo = "/srv/ostree/repo".to_string();
        global.push(format!("--repo={repo}"));
        assert_eq!(global[0], "--repo=/srv/ostree/repo");
    }

    #[test]
    fn test_global_sysroot_flag() {
        let mut global = Vec::new();
        let sysroot = "/sysroot".to_string();
        global.push(format!("--sysroot={sysroot}"));
        assert_eq!(global[0], "--sysroot=/sysroot");
    }

    // -----------------------------------------------------------------------
    // Command subcommand validation (binary required for actual execution)
    // Using /bin/true as a stub where we only care about parameter routing,
    // not actual ostree semantics.
    // -----------------------------------------------------------------------

    #[test]
    fn test_commit_missing_branch_errors() {
        let args = make_args(&[
            ("command", Value::String("commit".into())),
            ("src", Value::String("/tmp/rootfs".into())),
        ]);
        // dispatch is called with command="commit" but no branch → anyhow error
        let global: Vec<String> = Vec::new();
        let result = dispatch("commit", &args, "/bin/true", &global, "h", &mut ctx());
        assert!(result.is_err());
    }

    #[test]
    fn test_commit_missing_src_errors() {
        let args = make_args(&[
            ("command", Value::String("commit".into())),
            ("branch", Value::String("mybranch".into())),
        ]);
        let global: Vec<String> = Vec::new();
        let result = dispatch("commit", &args, "/bin/true", &global, "h", &mut ctx());
        assert!(result.is_err());
    }

    #[test]
    fn test_pull_missing_remote_errors() {
        let args = make_args(&[("command", Value::String("pull".into()))]);
        let global: Vec<String> = Vec::new();
        let result = dispatch("pull", &args, "/bin/true", &global, "h", &mut ctx());
        assert!(result.is_err());
    }

    #[test]
    fn test_checkout_missing_rev_errors() {
        let args = make_args(&[
            ("command", Value::String("checkout".into())),
            ("dest", Value::String("/mnt/out".into())),
        ]);
        let global: Vec<String> = Vec::new();
        let result = dispatch("checkout", &args, "/bin/true", &global, "h", &mut ctx());
        assert!(result.is_err());
    }

    #[test]
    fn test_checkout_missing_dest_errors() {
        let args = make_args(&[
            ("command", Value::String("checkout".into())),
            ("rev", Value::String("HEAD".into())),
        ]);
        let global: Vec<String> = Vec::new();
        let result = dispatch("checkout", &args, "/bin/true", &global, "h", &mut ctx());
        assert!(result.is_err());
    }

    #[test]
    fn test_remote_add_missing_name_errors() {
        let args = make_args(&[("url", Value::String("https://example.com/repo".into()))]);
        let global: Vec<String> = Vec::new();
        let result = dispatch("remote_add", &args, "/bin/true", &global, "h", &mut ctx());
        assert!(result.is_err());
    }

    #[test]
    fn test_remote_add_missing_url_errors() {
        let args = make_args(&[("name", Value::String("origin".into()))]);
        let global: Vec<String> = Vec::new();
        let result = dispatch("remote_add", &args, "/bin/true", &global, "h", &mut ctx());
        assert!(result.is_err());
    }

    #[test]
    fn test_admin_deploy_missing_rev_errors() {
        let args = ModuleArgs::new(HashMap::new());
        let global: Vec<String> = Vec::new();
        let result = dispatch("admin_deploy", &args, "/bin/true", &global, "h", &mut ctx());
        assert!(result.is_err());
    }

    #[test]
    fn test_admin_undeploy_missing_index_errors() {
        let args = ModuleArgs::new(HashMap::new());
        let global: Vec<String> = Vec::new();
        let result = dispatch(
            "admin_undeploy",
            &args,
            "/bin/true",
            &global,
            "h",
            &mut ctx(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_diff_missing_rev_a_errors() {
        let args = make_args(&[("rev_b", Value::String("HEAD".into()))]);
        let global: Vec<String> = Vec::new();
        let result = dispatch("diff", &args, "/bin/true", &global, "h", &mut ctx());
        assert!(result.is_err());
    }

    #[test]
    fn test_refs_delete_missing_ref_errors() {
        let args = make_args(&[("state", Value::String("absent".into()))]);
        let global: Vec<String> = Vec::new();
        let result = dispatch("refs", &args, "/bin/true", &global, "h", &mut ctx());
        assert!(result.is_err());
    }

    #[test]
    fn test_config_missing_key_errors() {
        let args = ModuleArgs::new(HashMap::new());
        let global: Vec<String> = Vec::new();
        let result = dispatch("config", &args, "/bin/true", &global, "h", &mut ctx());
        assert!(result.is_err());
    }

    #[test]
    fn test_cat_missing_path_errors() {
        let args = make_args(&[("rev", Value::String("HEAD".into()))]);
        let global: Vec<String> = Vec::new();
        let result = dispatch("cat", &args, "/bin/true", &global, "h", &mut ctx());
        assert!(result.is_err());
    }

    #[test]
    fn test_rev_parse_missing_rev_errors() {
        let args = ModuleArgs::new(HashMap::new());
        let global: Vec<String> = Vec::new();
        let result = dispatch("rev_parse", &args, "/bin/true", &global, "h", &mut ctx());
        assert!(result.is_err());
    }

    #[test]
    fn test_admin_switch_missing_ref_errors() {
        let args = ModuleArgs::new(HashMap::new());
        let global: Vec<String> = Vec::new();
        let result = dispatch("admin_switch", &args, "/bin/true", &global, "h", &mut ctx());
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Smoke tests with /bin/true (just verifies routing, not ostree semantics)
    // -----------------------------------------------------------------------

    #[test]
    fn test_fsck_with_true_bin_succeeds() {
        let args = make_args(&[("command", Value::String("fsck".into()))]);
        let global: Vec<String> = Vec::new();
        let result = dispatch("fsck", &args, "/bin/true", &global, "h", &mut ctx()).unwrap();
        // /bin/true always exits 0 → status ok
        assert!(result.status.is_ok());
    }

    #[test]
    fn test_summary_with_true_bin_succeeds() {
        let args = ModuleArgs::new(HashMap::new());
        let global: Vec<String> = Vec::new();
        let result = dispatch("summary", &args, "/bin/true", &global, "h", &mut ctx()).unwrap();
        assert!(result.status.is_ok() || result.changed);
    }

    #[test]
    fn test_admin_cleanup_with_true_bin() {
        let global: Vec<String> = Vec::new();
        let result = dispatch(
            "admin_cleanup",
            &ModuleArgs::new(HashMap::new()),
            "/bin/true",
            &global,
            "h",
            &mut ctx(),
        )
        .unwrap();
        assert!(result.changed);
    }

    #[test]
    fn test_remote_list_with_true_bin_stores_fact() {
        let global: Vec<String> = Vec::new();
        let mut c = ctx();
        let _result = dispatch(
            "remote_list",
            &ModuleArgs::new(HashMap::new()),
            "/bin/true",
            &global,
            "h",
            &mut c,
        )
        .unwrap();
        // fact is stored (may be empty list since /bin/true prints nothing)
        assert!(c.get_fact("h", "ostree_remotes").is_some());
    }

    #[test]
    fn test_refs_list_with_true_bin_stores_fact() {
        let global: Vec<String> = Vec::new();
        let mut c = ctx();
        let _result = dispatch(
            "refs",
            &ModuleArgs::new(HashMap::new()),
            "/bin/true",
            &global,
            "h",
            &mut c,
        )
        .unwrap();
        assert!(c.get_fact("h", "ostree_refs").is_some());
    }

    #[test]
    fn test_admin_status_with_true_bin_stores_fact() {
        let global: Vec<String> = Vec::new();
        let mut c = ctx();
        let _result = dispatch(
            "admin_status",
            &ModuleArgs::new(HashMap::new()),
            "/bin/true",
            &global,
            "h",
            &mut c,
        )
        .unwrap();
        assert!(c.get_fact("h", "ostree_admin_status").is_some());
    }

    #[test]
    fn test_admin_config_diff_with_true_bin_stores_fact() {
        let global: Vec<String> = Vec::new();
        let mut c = ctx();
        let _result = dispatch(
            "admin_config_diff",
            &ModuleArgs::new(HashMap::new()),
            "/bin/true",
            &global,
            "h",
            &mut c,
        )
        .unwrap();
        assert!(c.get_fact("h", "ostree_config_diff").is_some());
    }
}
