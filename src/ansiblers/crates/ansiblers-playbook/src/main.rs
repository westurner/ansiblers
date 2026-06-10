//! `ransible-playbook` — drop-in Rust replacement for `ansible-playbook`.
//!
//! Phase 1 supports: local task execution (localhost), shell, command, debug,
//! set_fact, and fail modules, with variable resolution and jinja2rs templating.

use std::collections::HashMap;
use std::process;
use std::sync::Arc;

use ansiblers_core::{ExecutionContext, Value};
use ansiblers_executor::PlayExecutor;
use ansiblers_inventory::load_inventory;
use ansiblers_modules::ModuleRegistry;
use ansiblers_parser::parse_playbook;
use anyhow::{Context, Result};
use clap::Parser;
use tracing::{error, info};

// ---------------------------------------------------------------------------
// CLI definition (mirrors ansible-playbook flags for compatibility)
// ---------------------------------------------------------------------------

/// ransible-playbook — high-performance Ansible playbook runner (Phase 1).
#[derive(Parser, Debug)]
#[command(
    name = "ransible-playbook",
    version,
    about = "Run Ansible playbooks with ransible (Rust Ansible)"
)]
struct Cli {
    /// Playbook file(s) to execute.
    #[arg(required = true, num_args = 1..)]
    playbook: Vec<String>,

    /// Inventory file or comma-separated host list.
    #[arg(short = 'i', long = "inventory")]
    inventory: Option<String>,

    /// Additional variables as key=value or JSON/YAML.
    #[arg(short = 'e', long = "extra-vars", action = clap::ArgAction::Append)]
    extra_vars: Vec<String>,

    /// Limit execution to a subset of hosts.
    #[arg(short = 'l', long = "limit")]
    limit: Option<String>,

    /// Run in check mode (dry run) — not yet implemented.
    #[arg(short = 'C', long = "check")]
    check: bool,

    /// Verbose output (-v / -vv / -vvv).
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
    verbose: u8,

    /// Output results as JSON.
    #[arg(long = "json")]
    json_output: bool,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    let cli = Cli::parse();

    // Initialise structured logging.
    let log_level = match cli.verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| log_level.into()),
        )
        .init();

    if cli.check {
        eprintln!("Warning: --check mode is not yet implemented; running normally.");
    }

    match run(cli) {
        Ok(success) => {
            if !success {
                process::exit(2);
            }
        }
        Err(e) => {
            error!("{e:#}");
            process::exit(1);
        }
    }
}

fn run(cli: Cli) -> Result<bool> {
    // Load inventory.
    let inventory = if let Some(inv_path) = &cli.inventory {
        load_inventory(inv_path).with_context(|| format!("loading inventory '{inv_path}'"))?
    } else {
        // Default: implicit localhost.
        ansiblers_core::Inventory::new()
    };

    // Parse extra vars.
    let extra_vars = parse_extra_vars(&cli.extra_vars)?;

    let mut ctx = ExecutionContext::new(Arc::new(inventory), extra_vars);
    ctx.verbosity = cli.verbose;

    let executor = PlayExecutor::new(ModuleRegistry::with_defaults());
    let mut overall_success = true;

    for playbook_path in &cli.playbook {
        info!(playbook = %playbook_path, "loading playbook");
        let playbook =
            parse_playbook(playbook_path).with_context(|| format!("parsing '{playbook_path}'"))?;

        let result = executor
            .run_playbook(&playbook, &mut ctx)
            .with_context(|| format!("executing '{playbook_path}'"))?;

        if cli.json_output {
            print_json_summary(&result);
        } else {
            print_play_recap(&result);
        }

        if !result.success {
            overall_success = false;
        }
    }

    Ok(overall_success)
}

// ---------------------------------------------------------------------------
// Output helpers
// ---------------------------------------------------------------------------

fn print_play_recap(result: &ansiblers_executor::PlaybookResult) {
    println!("\nPLAY RECAP");
    println!("{}", "─".repeat(60));

    for play_result in &result.play_results {
        for (host, task_results) in &play_result.host_results {
            let ok = task_results
                .iter()
                .filter(|r| r.status == ansiblers_core::TaskStatus::Ok)
                .count();
            let changed = task_results
                .iter()
                .filter(|r| r.status == ansiblers_core::TaskStatus::Changed)
                .count();
            let failed = task_results.iter().filter(|r| r.status.is_failed()).count();
            let skipped = task_results
                .iter()
                .filter(|r| r.status == ansiblers_core::TaskStatus::Skipped)
                .count();

            println!(
                "{host:<30} : ok={ok:<4} changed={changed:<4} failed={failed:<4} skipped={skipped}"
            );
        }
    }
}

fn print_json_summary(result: &ansiblers_executor::PlaybookResult) {
    let summary: serde_json::Value = serde_json::json!({
        "success": result.success,
        "plays": result.play_results.iter().map(|pr| {
            serde_json::json!({
                "name": pr.play_name,
                "success": pr.success,
                "hosts": pr.host_results.keys().collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>()
    });
    println!("{}", serde_json::to_string_pretty(&summary).unwrap());
}

// ---------------------------------------------------------------------------
// Extra-vars parsing
// ---------------------------------------------------------------------------

fn parse_extra_vars(raw: &[String]) -> Result<HashMap<String, Value>> {
    let mut vars = HashMap::new();
    for item in raw {
        // Try JSON object first.
        if item.trim_start().starts_with('{') {
            let map: HashMap<String, Value> = serde_json::from_str(item)
                .with_context(|| format!("parsing --extra-vars JSON: {item}"))?;
            vars.extend(map);
        } else {
            // key=value pair.
            let (k, v) = item
                .split_once('=')
                .with_context(|| format!("extra-vars must be key=value or JSON, got: {item}"))?;
            vars.insert(k.trim().to_string(), Value::String(v.trim().to_string()));
        }
    }
    Ok(vars)
}
