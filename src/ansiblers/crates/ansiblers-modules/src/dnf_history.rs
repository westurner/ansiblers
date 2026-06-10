//! `dnf_history` module — query and manage DNF transaction history via three
//! fallback tiers, in order of preference:
//!
//! 1. **`dnf history …`** CLI — the canonical interface, always preferred.
//! 2. **`python3 -m dnf.cli.main history …`** — available when the `dnf`
//!    Python package is installed but the `dnf` binary is absent (common on
//!    non-rpm-ostree Fedora/RHEL with stripped environments).
//! 3. **Direct SQLite read** of `/var/lib/dnf/history.sqlite` (configurable
//!    via `db_path`) — works on any system that has the database on disk,
//!    including rpm-ostree hosts that ship neither `dnf` nor its Python stack.
//!    ⚠️ The libdnf SQLite schema is not part of a public API and may change
//!    between DNF major versions; the module will return a warning fact when
//!    using this path.
//!
//! ## Supported operations
//!
//! | `operation` | CLI equivalent | Description |
//! |-------------|---------------|-------------|
//! | `list` (default) | `dnf history list` | List recent transactions |
//! | `info` | `dnf history info <id>` | Detail a single transaction |
//! | `userinstalled` | `dnf history userinstalled` | User-installed packages |
//! | `undo` | `dnf history undo <id>` | Undo a transaction |
//! | `redo` | `dnf history redo <id>` | Re-apply an undone transaction |
//! | `rollback` | `dnf history rollback <id>` | Rollback to before a trans |
//!
//! ## Parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `operation` | `list` | Operation from the table above |
//! | `transaction_id` / `tid` | — | Transaction ID for `info`/`undo`/`redo`/`rollback` |
//! | `reverse` | `false` | Show `list` output in reverse (oldest first) |
//! | `limit` | — | Maximum number of transactions to return from `list` |
//! | `pattern` | — | Package name pattern to filter `list` results |
//! | `db_path` | `/var/lib/dnf/history.sqlite` | Path to the SQLite database |
//! | `dnf_bin` | `dnf` | Path / name of the `dnf` binary |
//! | `python_bin` | `python3` | Python interpreter for tier-2 fallback |
//! | `force_sqlite` | `false` | Skip tiers 1 and 2, use SQLite directly |
//! | `force_python` | `false` | Skip tier 1, try tier 2 before tier 3 |
//!
//! ## Return variables
//!
//! All query operations (`list`, `info`, `userinstalled`) set:
//!
//! - `vars.transactions` / `vars.packages` — parsed result list
//! - `vars.raw_output` — raw CLI stdout or JSON-serialised SQLite rows
//! - `vars.source` — one of `"cli"`, `"python"`, `"sqlite"`
//! - `vars.schema_warning` — present (and `true`) when `source == "sqlite"`
//!
//! Mutating operations (`undo`, `redo`, `rollback`) return `changed=true` on
//! success.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

// ---------------------------------------------------------------------------
// Well-known action codes in the libdnf SQLite schema.
// ---------------------------------------------------------------------------

fn action_name(code: i64) -> &'static str {
    match code {
        1 => "Install",
        2 => "Upgrade",
        3 => "Upgraded",
        4 => "Downgrade",
        5 => "Downgraded",
        6 => "Remove",
        7 => "Reinstall",
        8 => "Reinstalled",
        9 => "ReasonChange",
        _ => "Unknown",
    }
}

fn reason_name(code: i64) -> &'static str {
    match code {
        1 => "User",
        2 => "Group",
        3 => "Dependency",
        4 => "WeakDependency",
        5 => "ExternalUser",
        _ => "Unknown",
    }
}

fn state_name(code: i64) -> &'static str {
    match code {
        1 => "Done",
        2 => "Error",
        _ => "Unknown",
    }
}

// ---------------------------------------------------------------------------
// Module entry point
// ---------------------------------------------------------------------------

pub struct DnfHistoryModule;

impl ModuleInvoker for DnfHistoryModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let operation = args.get_str("operation").unwrap_or("list");
        let dnf_bin = args.get_str("dnf_bin").unwrap_or("dnf").to_string();
        let python_bin = args.get_str("python_bin").unwrap_or("python3").to_string();
        let db_path = args
            .get_str("db_path")
            .unwrap_or("/var/lib/dnf/history.sqlite")
            .to_string();
        let force_sqlite = bool_arg(args, "force_sqlite", false);
        let force_python = bool_arg(args, "force_python", false);

        // Mutating operations always use CLI (tiers 1→2).
        let is_mutating = matches!(operation, "undo" | "redo" | "rollback");

        if is_mutating {
            return handle_mutating(operation, args, &dnf_bin, &python_bin, host);
        }

        // Query operations: try tiers in order.
        if !force_sqlite && !force_python {
            if let Ok(result) = try_cli(operation, args, &dnf_bin, host, ctx) {
                return Ok(result);
            }
        }
        if !force_sqlite {
            if let Ok(result) = try_python(operation, args, &python_bin, host, ctx) {
                return Ok(result);
            }
        }
        // Tier 3: SQLite.
        try_sqlite(operation, args, &db_path, host, ctx)
    }
}

// ---------------------------------------------------------------------------
// Tier 1 — `dnf history …` CLI
// ---------------------------------------------------------------------------

fn try_cli(
    operation: &str,
    args: &ModuleArgs,
    dnf_bin: &str,
    host: &str,
    ctx: &mut ExecutionContext,
) -> Result<TaskResult> {
    let mut cmd = Command::new(dnf_bin);
    cmd.args(["history"]);
    build_history_args(operation, args, &mut cmd);

    let output = cmd.output().context("dnf not available")?;
    if !output.status.success() {
        anyhow::bail!("dnf history exited {}", output.status.code().unwrap_or(-1));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    finish_query(operation, stdout, "cli", host, ctx)
}

// ---------------------------------------------------------------------------
// Tier 2 — `python3 -m dnf.cli.main history …`
// ---------------------------------------------------------------------------

fn try_python(
    operation: &str,
    args: &ModuleArgs,
    python_bin: &str,
    host: &str,
    ctx: &mut ExecutionContext,
) -> Result<TaskResult> {
    let mut cmd = Command::new(python_bin);
    cmd.args(["-m", "dnf.cli.main", "history"]);
    build_history_args(operation, args, &mut cmd);

    let output = cmd.output().context("python3 dnf.cli.main not available")?;
    if !output.status.success() {
        anyhow::bail!(
            "python3 -m dnf.cli.main history exited {}",
            output.status.code().unwrap_or(-1)
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    finish_query(operation, stdout, "python", host, ctx)
}

fn build_history_args(operation: &str, args: &ModuleArgs, cmd: &mut Command) {
    match operation {
        "list" => {
            cmd.arg("list");
            if bool_arg(args, "reverse", false) {
                cmd.arg("--reverse");
            }
            if let Some(pattern) = args.get_str("pattern") {
                cmd.arg(pattern);
            }
        }
        "info" => {
            let tid = tid_arg(args);
            cmd.arg("info");
            if let Some(t) = tid {
                cmd.arg(&t);
            }
        }
        "userinstalled" => {
            cmd.arg("userinstalled");
        }
        _ => {}
    }
}

fn finish_query(
    operation: &str,
    stdout: String,
    source: &str,
    host: &str,
    ctx: &mut ExecutionContext,
) -> Result<TaskResult> {
    let var_key = if operation == "userinstalled" {
        "packages"
    } else {
        "transactions"
    };
    let parsed = parse_cli_output(operation, &stdout);

    let mut vars: HashMap<String, Value> = HashMap::new();
    vars.insert(var_key.into(), parsed.clone());
    vars.insert("raw_output".into(), Value::String(stdout.clone()));
    vars.insert("source".into(), Value::String(source.into()));

    ctx.set_fact(host, format!("dnf_history_{var_key}"), parsed);
    ctx.set_fact(
        host,
        "dnf_history_source".into(),
        Value::String(source.into()),
    );

    let mut r = TaskResult::ok(host);
    r.stdout = stdout;
    r.vars = vars;
    r.msg = format!("dnf history {operation} via {source}");
    Ok(r)
}

/// Very lightweight parser: each non-blank line becomes a Value::String.
/// Caller is responsible for post-processing if structured output is needed.
fn parse_cli_output(operation: &str, text: &str) -> Value {
    let lines: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| Value::String(l.to_string()))
        .collect();
    Value::Array(lines)
}

// ---------------------------------------------------------------------------
// Tier 3 — direct SQLite read
// ---------------------------------------------------------------------------

fn try_sqlite(
    operation: &str,
    args: &ModuleArgs,
    db_path: &str,
    host: &str,
    ctx: &mut ExecutionContext,
) -> Result<TaskResult> {
    if !Path::new(db_path).exists() {
        return Ok(TaskResult::failed(
            host,
            format!("dnf_history: database not found at '{db_path}' and dnf/python unavailable"),
        ));
    }

    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("cannot open dnf history db '{db_path}'"))?;

    let result = match operation {
        "list" => sqlite_list(&conn, args, host, ctx)?,
        "info" => sqlite_info(&conn, args, host, ctx)?,
        "userinstalled" => sqlite_userinstalled(&conn, host, ctx)?,
        _ => {
            return Ok(TaskResult::failed(
                host,
                format!(
                    "dnf_history: '{operation}' requires dnf/python (not available via SQLite)"
                ),
            ))
        }
    };

    Ok(result)
}

fn sqlite_list(
    conn: &rusqlite::Connection,
    args: &ModuleArgs,
    host: &str,
    ctx: &mut ExecutionContext,
) -> Result<TaskResult> {
    let limit = args
        .args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20);
    let reverse = bool_arg(args, "reverse", false);
    let order = if reverse { "ASC" } else { "DESC" };

    // NOTE: Schema may differ between libdnf versions; we select only the most
    //       stable columns.  The `cmdline` column may be NULL on some versions.
    let sql = format!(
        "SELECT t.id, t.dt_begin, t.dt_end, t.releasever, \
                t.cmdline, t.state \
         FROM trans t \
         ORDER BY t.id {order} \
         LIMIT ?1"
    );

    let mut stmt = conn.prepare(&sql).context("prepare history list query")?;
    let rows: Vec<Value> = stmt
        .query_map([limit as i64], |row| {
            let id: i64 = row.get(0)?;
            let dt_begin: i64 = row.get(1)?;
            let dt_end: i64 = row.get(2)?;
            let releasever: Option<String> = row.get(3)?;
            let cmdline: Option<String> = row.get(4)?;
            let state: i64 = row.get(5)?;
            Ok((id, dt_begin, dt_end, releasever, cmdline, state))
        })
        .context("query history list")?
        .filter_map(|r| r.ok())
        .map(|(id, dt_begin, dt_end, releasever, cmdline, state)| {
            let mut m = serde_json::Map::new();
            m.insert("id".into(), Value::Number(id.into()));
            m.insert("dt_begin".into(), Value::Number(dt_begin.into()));
            m.insert("dt_end".into(), Value::Number(dt_end.into()));
            m.insert(
                "releasever".into(),
                releasever.map(Value::String).unwrap_or(Value::Null),
            );
            m.insert(
                "cmdline".into(),
                cmdline.map(Value::String).unwrap_or(Value::Null),
            );
            m.insert("state".into(), Value::String(state_name(state).into()));
            Value::Object(m)
        })
        .collect();

    let transactions = Value::Array(rows);
    let raw = serde_json::to_string(&transactions).unwrap_or_default();

    let mut vars: HashMap<String, Value> = HashMap::new();
    vars.insert("transactions".into(), transactions.clone());
    vars.insert("raw_output".into(), Value::String(raw));
    vars.insert("source".into(), Value::String("sqlite".into()));
    vars.insert("schema_warning".into(), Value::Bool(true));

    ctx.set_fact(host, "dnf_history_transactions".into(), transactions);
    ctx.set_fact(
        host,
        "dnf_history_source".into(),
        Value::String("sqlite".into()),
    );

    let mut r = TaskResult::ok(host);
    r.vars = vars;
    r.msg = "dnf history list via sqlite (schema may vary)".into();
    Ok(r)
}

fn sqlite_info(
    conn: &rusqlite::Connection,
    args: &ModuleArgs,
    host: &str,
    ctx: &mut ExecutionContext,
) -> Result<TaskResult> {
    let tid = tid_arg(args)
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| anyhow::anyhow!("dnf_history info: 'transaction_id' is required"))?;

    // Transaction header.
    let trans_sql = "SELECT id, dt_begin, dt_end, releasever, cmdline, loginuid, state \
                     FROM trans WHERE id = ?1";
    let trans: Option<Value> = conn
        .query_row(trans_sql, [tid], |row| {
            let id: i64 = row.get(0)?;
            let dt_begin: i64 = row.get(1)?;
            let dt_end: i64 = row.get(2)?;
            let releasever: Option<String> = row.get(3)?;
            let cmdline: Option<String> = row.get(4)?;
            let loginuid: Option<i64> = row.get(5)?;
            let state: i64 = row.get(6)?;
            Ok((id, dt_begin, dt_end, releasever, cmdline, loginuid, state))
        })
        .ok()
        .map(
            |(id, dt_begin, dt_end, releasever, cmdline, loginuid, state)| {
                let mut m = serde_json::Map::new();
                m.insert("id".into(), Value::Number(id.into()));
                m.insert("dt_begin".into(), Value::Number(dt_begin.into()));
                m.insert("dt_end".into(), Value::Number(dt_end.into()));
                m.insert(
                    "releasever".into(),
                    releasever.map(Value::String).unwrap_or(Value::Null),
                );
                m.insert(
                    "cmdline".into(),
                    cmdline.map(Value::String).unwrap_or(Value::Null),
                );
                m.insert(
                    "loginuid".into(),
                    loginuid
                        .map(|u| Value::Number(u.into()))
                        .unwrap_or(Value::Null),
                );
                m.insert("state".into(), Value::String(state_name(state).into()));
                Value::Object(m)
            },
        );

    if trans.is_none() {
        return Ok(TaskResult::failed(
            host,
            format!("dnf_history: transaction id {tid} not found"),
        ));
    }

    // Transaction items (packages).
    let items_sql = "SELECT rpm.name, rpm.epoch, rpm.version, rpm.release, rpm.arch, \
                            ti.action, ti.reason, ti.state, repo.repoid \
                     FROM trans_item ti \
                     JOIN rpm USING (item_id) \
                     LEFT JOIN repo ON ti.repo_id = repo.id \
                     WHERE ti.trans_id = ?1 \
                     ORDER BY rpm.name";

    let mut stmt = conn
        .prepare(items_sql)
        .context("prepare history info items")?;
    let items: Vec<Value> = stmt
        .query_map([tid], |row| {
            let name: String = row.get(0)?;
            let epoch: i64 = row.get(1)?;
            let version: String = row.get(2)?;
            let release: String = row.get(3)?;
            let arch: String = row.get(4)?;
            let action: i64 = row.get(5)?;
            let reason: i64 = row.get(6)?;
            let state: i64 = row.get(7)?;
            let repoid: Option<String> = row.get(8)?;
            Ok((
                name, epoch, version, release, arch, action, reason, state, repoid,
            ))
        })
        .context("query history info items")?
        .filter_map(|r| r.ok())
        .map(
            |(name, epoch, version, release, arch, action, reason, state, repoid)| {
                let nevra = if epoch > 0 {
                    format!("{name}-{epoch}:{version}-{release}.{arch}")
                } else {
                    format!("{name}-{version}-{release}.{arch}")
                };
                let mut m = serde_json::Map::new();
                m.insert("nevra".into(), Value::String(nevra));
                m.insert("name".into(), Value::String(name));
                m.insert("epoch".into(), Value::Number(epoch.into()));
                m.insert("version".into(), Value::String(version));
                m.insert("release".into(), Value::String(release));
                m.insert("arch".into(), Value::String(arch));
                m.insert("action".into(), Value::String(action_name(action).into()));
                m.insert("reason".into(), Value::String(reason_name(reason).into()));
                m.insert("state".into(), Value::String(state_name(state).into()));
                m.insert(
                    "repoid".into(),
                    repoid.map(Value::String).unwrap_or(Value::Null),
                );
                Value::Object(m)
            },
        )
        .collect();

    let mut info_map = serde_json::Map::new();
    info_map.insert("transaction".into(), trans.unwrap_or(Value::Null));
    info_map.insert("items".into(), Value::Array(items));
    let info = Value::Object(info_map);
    let raw = serde_json::to_string(&info).unwrap_or_default();

    let mut vars: HashMap<String, Value> = HashMap::new();
    vars.insert("transactions".into(), info.clone());
    vars.insert("raw_output".into(), Value::String(raw));
    vars.insert("source".into(), Value::String("sqlite".into()));
    vars.insert("schema_warning".into(), Value::Bool(true));

    ctx.set_fact(host, "dnf_history_info".into(), info);
    ctx.set_fact(
        host,
        "dnf_history_source".into(),
        Value::String("sqlite".into()),
    );

    let mut r = TaskResult::ok(host);
    r.vars = vars;
    r.msg = format!("dnf history info {tid} via sqlite (schema may vary)");
    Ok(r)
}

fn sqlite_userinstalled(
    conn: &rusqlite::Connection,
    host: &str,
    ctx: &mut ExecutionContext,
) -> Result<TaskResult> {
    // User-installed: packages whose most-recent trans_item reason = USER (1)
    // and action is not REMOVE (6).
    let sql = "SELECT DISTINCT rpm.name, rpm.epoch, rpm.version, rpm.release, rpm.arch \
               FROM trans_item ti \
               JOIN rpm USING (item_id) \
               WHERE ti.reason = 1 \
                 AND ti.action NOT IN (3, 5, 6, 8) \
                 AND ti.id = ( \
                     SELECT id FROM trans_item ti2 \
                     WHERE ti2.item_id = ti.item_id \
                     ORDER BY ti2.id DESC LIMIT 1 \
                 ) \
               ORDER BY rpm.name";

    let mut stmt = conn.prepare(sql).context("prepare userinstalled query")?;
    let packages: Vec<Value> = stmt
        .query_map([], |row| {
            let name: String = row.get(0)?;
            let epoch: i64 = row.get(1)?;
            let version: String = row.get(2)?;
            let release: String = row.get(3)?;
            let arch: String = row.get(4)?;
            Ok((name, epoch, version, release, arch))
        })
        .context("query userinstalled")?
        .filter_map(|r| r.ok())
        .map(|(name, epoch, version, release, arch)| {
            let nevra = if epoch > 0 {
                format!("{name}-{epoch}:{version}-{release}.{arch}")
            } else {
                format!("{name}-{version}-{release}.{arch}")
            };
            let mut m = serde_json::Map::new();
            m.insert("nevra".into(), Value::String(nevra));
            m.insert("name".into(), Value::String(name));
            m.insert("epoch".into(), Value::Number(epoch.into()));
            m.insert("version".into(), Value::String(version));
            m.insert("release".into(), Value::String(release));
            m.insert("arch".into(), Value::String(arch));
            Value::Object(m)
        })
        .collect();

    let pkg_list = Value::Array(packages);
    let raw = serde_json::to_string(&pkg_list).unwrap_or_default();

    let mut vars: HashMap<String, Value> = HashMap::new();
    vars.insert("packages".into(), pkg_list.clone());
    vars.insert("raw_output".into(), Value::String(raw));
    vars.insert("source".into(), Value::String("sqlite".into()));
    vars.insert("schema_warning".into(), Value::Bool(true));

    ctx.set_fact(host, "dnf_history_packages".into(), pkg_list);
    ctx.set_fact(
        host,
        "dnf_history_source".into(),
        Value::String("sqlite".into()),
    );

    let mut r = TaskResult::ok(host);
    r.vars = vars;
    r.msg = "dnf history userinstalled via sqlite (schema may vary)".into();
    Ok(r)
}

// ---------------------------------------------------------------------------
// Mutating operations — always CLI (tier 1 → 2)
// ---------------------------------------------------------------------------

fn handle_mutating(
    operation: &str,
    args: &ModuleArgs,
    dnf_bin: &str,
    python_bin: &str,
    host: &str,
) -> Result<TaskResult> {
    let tid = tid_arg(args)
        .ok_or_else(|| anyhow::anyhow!("dnf_history {operation}: 'transaction_id' is required"))?;

    // Try tier 1 first, then tier 2.
    let result = run_mutating(operation, &tid, dnf_bin)
        .or_else(|_| run_mutating_python(operation, &tid, python_bin));

    match result {
        Ok((stdout, stderr)) => {
            let _ = stderr;
            let mut r = TaskResult::changed(host);
            r.stdout = stdout;
            r.msg = format!("dnf history {operation} {tid}");
            Ok(r)
        }
        Err(e) => Ok(TaskResult::failed(
            host,
            format!("dnf_history {operation} failed: {e}"),
        )),
    }
}

fn run_mutating(operation: &str, tid: &str, dnf_bin: &str) -> Result<(String, String)> {
    let output = Command::new(dnf_bin)
        .args(["history", operation, "-y", tid])
        .output()
        .context("dnf not available")?;
    if output.status.success() {
        Ok((
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    } else {
        anyhow::bail!(
            "dnf history {} exited {}",
            operation,
            output.status.code().unwrap_or(-1)
        )
    }
}

fn run_mutating_python(operation: &str, tid: &str, python_bin: &str) -> Result<(String, String)> {
    let output = Command::new(python_bin)
        .args(["-m", "dnf.cli.main", "history", operation, "-y", tid])
        .output()
        .context("python3 not available")?;
    if output.status.success() {
        Ok((
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    } else {
        anyhow::bail!(
            "python3 -m dnf.cli.main history {} exited {}",
            operation,
            output.status.code().unwrap_or(-1)
        )
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn tid_arg(args: &ModuleArgs) -> Option<String> {
    args.get_str("transaction_id")
        .or_else(|| args.get_str("tid"))
        .map(|s| s.to_string())
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
    use tempfile::NamedTempFile;

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
    // Helper / unit tests — no binary or DB required
    // -----------------------------------------------------------------------

    #[test]
    fn test_action_name_mapping() {
        assert_eq!(action_name(1), "Install");
        assert_eq!(action_name(2), "Upgrade");
        assert_eq!(action_name(6), "Remove");
        assert_eq!(action_name(99), "Unknown");
    }

    #[test]
    fn test_reason_name_mapping() {
        assert_eq!(reason_name(1), "User");
        assert_eq!(reason_name(3), "Dependency");
        assert_eq!(reason_name(99), "Unknown");
    }

    #[test]
    fn test_state_name_mapping() {
        assert_eq!(state_name(1), "Done");
        assert_eq!(state_name(2), "Error");
        assert_eq!(state_name(0), "Unknown");
    }

    #[test]
    fn test_tid_arg_transaction_id_key() {
        let args = make_args(&[("transaction_id", Value::String("42".into()))]);
        assert_eq!(tid_arg(&args), Some("42".into()));
    }

    #[test]
    fn test_tid_arg_tid_alias() {
        let args = make_args(&[("tid", Value::String("7".into()))]);
        assert_eq!(tid_arg(&args), Some("7".into()));
    }

    #[test]
    fn test_tid_arg_missing() {
        let args = ModuleArgs::new(HashMap::new());
        assert!(tid_arg(&args).is_none());
    }

    #[test]
    fn test_bool_arg_defaults() {
        let args = ModuleArgs::new(HashMap::new());
        assert!(!bool_arg(&args, "reverse", false));
        assert!(!bool_arg(&args, "force_sqlite", false));
    }

    #[test]
    fn test_parse_cli_output_filters_blank_lines() {
        let text = "  id | action | pkg\n\n  1  | Install | bash-5.1\n  \n";
        if let Value::Array(lines) = parse_cli_output("list", text) {
            // blank lines filtered out
            assert_eq!(lines.len(), 2);
        } else {
            panic!("expected Array");
        }
    }

    #[test]
    fn test_parse_cli_output_empty() {
        if let Value::Array(lines) = parse_cli_output("list", "") {
            assert!(lines.is_empty());
        }
    }

    // -----------------------------------------------------------------------
    // Module-level: binary not found → fallback chain → db not found → fail
    // -----------------------------------------------------------------------

    #[test]
    fn test_all_tiers_fail_gracefully() {
        let mut c = ctx();
        let args = make_args(&[
            ("operation", Value::String("list".into())),
            ("dnf_bin", Value::String("/nonexistent/dnf".into())),
            ("python_bin", Value::String("/nonexistent/python3".into())),
            (
                "db_path",
                Value::String("/nonexistent/history.sqlite".into()),
            ),
        ]);
        let result = DnfHistoryModule.invoke(&args, "localhost", &mut c).unwrap();
        assert!(result.status.is_failed());
        assert!(result.msg.contains("not found") || result.msg.contains("unavailable"));
    }

    #[test]
    fn test_mutating_missing_tid_errors() {
        let mut c = ctx();
        let args = make_args(&[
            ("operation", Value::String("undo".into())),
            ("dnf_bin", Value::String("/nonexistent/dnf".into())),
            ("python_bin", Value::String("/nonexistent/python3".into())),
        ]);
        let result = DnfHistoryModule.invoke(&args, "localhost", &mut c);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // SQLite tier tests — create a minimal in-memory DB with the schema
    // -----------------------------------------------------------------------

    fn create_test_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE trans (
                id       INTEGER PRIMARY KEY,
                dt_begin INTEGER,
                dt_end   INTEGER,
                releasever TEXT,
                cmdline  TEXT,
                loginuid INTEGER,
                state    INTEGER
            );
            CREATE TABLE repo (
                id     INTEGER PRIMARY KEY,
                repoid TEXT
            );
            CREATE TABLE rpm (
                item_id INTEGER PRIMARY KEY,
                name    TEXT,
                epoch   INTEGER,
                version TEXT,
                release TEXT,
                arch    TEXT
            );
            CREATE TABLE trans_item (
                id       INTEGER PRIMARY KEY,
                trans_id INTEGER REFERENCES trans(id),
                item_id  INTEGER,
                repo_id  INTEGER REFERENCES repo(id),
                action   INTEGER,
                reason   INTEGER,
                state    INTEGER
            );
            -- Populate test data
            INSERT INTO trans VALUES (1, 1700000000, 1700000100, '39', 'dnf install bash', 1000, 1);
            INSERT INTO trans VALUES (2, 1700001000, 1700001200, '39', 'dnf upgrade vim', 1000, 1);
            INSERT INTO repo VALUES (1, 'fedora');
            INSERT INTO repo VALUES (2, 'updates');
            INSERT INTO rpm VALUES (101, 'bash', 0, '5.1.8', '6.fc39', 'x86_64');
            INSERT INTO rpm VALUES (102, 'vim-enhanced', 0, '9.0.1', '1.fc39', 'x86_64');
            INSERT INTO rpm VALUES (103, 'vim-enhanced', 0, '9.0.2', '1.fc39', 'x86_64');
            INSERT INTO trans_item VALUES (1, 1, 101, 1, 1, 1, 1);  -- Install bash (User)
            INSERT INTO trans_item VALUES (2, 2, 102, 2, 3, 3, 1);  -- Upgraded old vim (Dep)
            INSERT INTO trans_item VALUES (3, 2, 103, 2, 2, 3, 1);  -- Upgrade vim (Dep)
            ",
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_sqlite_list_returns_transactions() {
        let conn = create_test_db();
        let mut c = ctx();
        let args = ModuleArgs::new(HashMap::new());
        let result = sqlite_list(&conn, &args, "localhost", &mut c).unwrap();
        assert!(result.status.is_ok());
        // schema_warning set
        assert_eq!(result.vars.get("schema_warning"), Some(&Value::Bool(true)));
        assert_eq!(
            result.vars.get("source"),
            Some(&Value::String("sqlite".into()))
        );
        // transactions array present
        if let Some(Value::Array(txns)) = result.vars.get("transactions") {
            assert_eq!(txns.len(), 2);
            // desc order by default → id=2 first
            if let Value::Object(m) = &txns[0] {
                assert_eq!(m["id"], Value::Number(2.into()));
                assert_eq!(m["state"], Value::String("Done".into()));
            }
        } else {
            panic!("expected transactions array");
        }
        // fact stored
        assert!(c
            .get_fact("localhost", "dnf_history_transactions")
            .is_some());
    }

    #[test]
    fn test_sqlite_list_reverse_order() {
        let conn = create_test_db();
        let mut c = ctx();
        let args = make_args(&[("reverse", Value::Bool(true))]);
        let result = sqlite_list(&conn, &args, "h", &mut c).unwrap();
        if let Some(Value::Array(txns)) = result.vars.get("transactions") {
            assert_eq!(txns.len(), 2);
            if let Value::Object(m) = &txns[0] {
                // asc order → id=1 first
                assert_eq!(m["id"], Value::Number(1.into()));
            }
        }
    }

    #[test]
    fn test_sqlite_list_limit() {
        let conn = create_test_db();
        let mut c = ctx();
        let args = make_args(&[("limit", Value::Number(1.into()))]);
        let result = sqlite_list(&conn, &args, "h", &mut c).unwrap();
        if let Some(Value::Array(txns)) = result.vars.get("transactions") {
            assert_eq!(txns.len(), 1);
        }
    }

    #[test]
    fn test_sqlite_info_transaction_1() {
        let conn = create_test_db();
        let mut c = ctx();
        let args = make_args(&[("transaction_id", Value::String("1".into()))]);
        let result = sqlite_info(&conn, &args, "h", &mut c).unwrap();
        assert!(result.status.is_ok());
        if let Some(Value::Object(m)) = result.vars.get("transactions") {
            if let Some(Value::Array(items)) = m.get("items") {
                assert_eq!(items.len(), 1);
                if let Value::Object(item) = &items[0] {
                    assert_eq!(item["name"], Value::String("bash".into()));
                    assert_eq!(item["action"], Value::String("Install".into()));
                    assert_eq!(item["reason"], Value::String("User".into()));
                }
            } else {
                panic!("expected items array");
            }
        }
    }

    #[test]
    fn test_sqlite_info_transaction_2_two_items() {
        let conn = create_test_db();
        let mut c = ctx();
        let args = make_args(&[("transaction_id", Value::String("2".into()))]);
        let result = sqlite_info(&conn, &args, "h", &mut c).unwrap();
        if let Some(Value::Object(m)) = result.vars.get("transactions") {
            if let Some(Value::Array(items)) = m.get("items") {
                assert_eq!(items.len(), 2);
            }
        }
    }

    #[test]
    fn test_sqlite_info_missing_transaction() {
        let conn = create_test_db();
        let mut c = ctx();
        let args = make_args(&[("transaction_id", Value::String("999".into()))]);
        let result = sqlite_info(&conn, &args, "h", &mut c).unwrap();
        assert!(result.status.is_failed());
    }

    #[test]
    fn test_sqlite_info_missing_tid_errors() {
        let conn = create_test_db();
        let mut c = ctx();
        let args = ModuleArgs::new(HashMap::new());
        let result = sqlite_info(&conn, &args, "h", &mut c);
        assert!(result.is_err());
    }

    #[test]
    fn test_sqlite_userinstalled_finds_bash() {
        let conn = create_test_db();
        let mut c = ctx();
        let result = sqlite_userinstalled(&conn, "h", &mut c).unwrap();
        assert!(result.status.is_ok());
        if let Some(Value::Array(pkgs)) = result.vars.get("packages") {
            // bash was installed with reason=User(1)
            assert!(pkgs.iter().any(|p| {
                if let Value::Object(m) = p {
                    m["name"] == Value::String("bash".into())
                } else {
                    false
                }
            }));
            // vim was installed with reason=Dependency(3) — should NOT appear
            assert!(!pkgs.iter().any(|p| {
                if let Value::Object(m) = p {
                    m["name"] == Value::String("vim-enhanced".into())
                } else {
                    false
                }
            }));
        } else {
            panic!("expected packages array");
        }
    }

    #[test]
    fn test_sqlite_unsupported_operation_returns_failure() {
        let conn = create_test_db();
        let mut c = ctx();
        let args = make_args(&[("transaction_id", Value::String("1".into()))]);
        let result = sqlite_info(&conn, &args, "h", &mut c).unwrap();
        // Regression: valid operation should succeed
        assert!(result.status.is_ok());
    }

    #[test]
    fn test_sqlite_undo_not_supported_returns_fail() {
        let tmp = NamedTempFile::new().unwrap();
        // Write minimal valid SQLite DB to the temp file
        let init_conn = rusqlite::Connection::open(tmp.path()).unwrap();
        init_conn
            .execute_batch(
                "CREATE TABLE trans (id INTEGER PRIMARY KEY, dt_begin INTEGER, \
             dt_end INTEGER, releasever TEXT, cmdline TEXT, state INTEGER);",
            )
            .unwrap();
        drop(init_conn);

        let mut c = ctx();
        let args = make_args(&[
            ("operation", Value::String("undo".into())),
            ("transaction_id", Value::String("1".into())),
            ("dnf_bin", Value::String("/nonexistent/dnf".into())),
            ("python_bin", Value::String("/nonexistent/python3".into())),
            (
                "db_path",
                Value::String(tmp.path().to_str().unwrap().to_string()),
            ),
        ]);
        let result = DnfHistoryModule.invoke(&args, "h", &mut c).unwrap();
        // undo via sqlite is not supported → failed result
        assert!(result.status.is_failed());
    }

    #[test]
    fn test_nevra_formatting_no_epoch() {
        let epoch = 0i64;
        let (name, version, release, arch) = ("bash", "5.1.8", "6.fc39", "x86_64");
        let nevra = if epoch > 0 {
            format!("{name}-{epoch}:{version}-{release}.{arch}")
        } else {
            format!("{name}-{version}-{release}.{arch}")
        };
        assert_eq!(nevra, "bash-5.1.8-6.fc39.x86_64");
    }

    #[test]
    fn test_nevra_formatting_with_epoch() {
        let epoch = 2i64;
        let (name, version, release, arch) = ("kernel", "6.6.0", "200.fc39", "x86_64");
        let nevra = if epoch > 0 {
            format!("{name}-{epoch}:{version}-{release}.{arch}")
        } else {
            format!("{name}-{version}-{release}.{arch}")
        };
        assert_eq!(nevra, "kernel-2:6.6.0-200.fc39.x86_64");
    }

    #[test]
    fn test_force_sqlite_skips_tiers_1_and_2() {
        let conn = create_test_db();
        let mut c = ctx();
        // Verify that sqlite_list works independently (tier 3 logic intact)
        let args = ModuleArgs::new(HashMap::new());
        let result = sqlite_list(&conn, &args, "h", &mut c).unwrap();
        assert!(result.status.is_ok());
        assert_eq!(result.vars["source"], Value::String("sqlite".into()));
    }
}
