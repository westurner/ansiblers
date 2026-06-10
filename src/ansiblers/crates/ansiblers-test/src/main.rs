//! `ransible-test` — test runner and CI integration for the ansiblers workspace.
//!
//! Mirrors the `ansible-test` subcommand interface so CI pipelines written for
//! Ansible's test runner work unchanged with ansiblers.
//!
//! ## Subcommands
//!
//! | Subcommand    | Description                                           |
//! |---------------|-------------------------------------------------------|
//! | `sanity`      | Run formatting, lint, and static analysis checks      |
//! | `units`       | Run `cargo test` for all or selected crates           |
//! | `integration` | Run integration tests (optionally inside Docker)      |
//! | `coverage`    | Generate coverage reports via `cargo-llvm-cov`        |
//!
//! ## Usage
//!
//! ```bash
//! ransible-test sanity
//! ransible-test units --coverage
//! ransible-test integration --docker
//! ransible-test coverage --html --fail-under-lines 75 --fail-under-branches 60
//! ```
//!
//! ## Exit codes
//!
//! | Code | Meaning |
//! |------|---------|
//! | 0    | All checks passed |
//! | 1    | One or more checks failed |
//! | 2    | Configuration / usage error |

use std::process;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::info;

mod coverage;
mod reporter;
mod runner;

pub use coverage::CoverageConfig;
pub use reporter::{CheckResult, TestReport};
pub use runner::{RunConfig, TestRunner};

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

/// ransible-test — run tests and CI checks for ansiblers.
#[derive(Parser, Debug)]
#[command(
    name = "ransible-test",
    version,
    about = "Test runner and CI integration for the ansiblers workspace"
)]
struct Cli {
    /// Emit results as machine-readable JSON.
    #[arg(long)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run formatting, lint, and static analysis.
    Sanity {
        /// Limit to specific crates (default: all).
        #[arg(long = "crate")]
        crates: Vec<String>,
    },

    /// Run unit tests via `cargo test`.
    Units {
        /// Limit to specific crates (default: all).
        #[arg(long = "crate")]
        crates: Vec<String>,
        /// Capture coverage while running tests.
        #[arg(long)]
        coverage: bool,
    },

    /// Run integration tests.
    Integration {
        /// Limit to specific test targets (default: all).
        #[arg(long = "target")]
        targets: Vec<String>,
        /// Run inside a Docker container.
        #[arg(long)]
        docker: bool,
    },

    /// Generate a coverage report.
    Coverage {
        /// Emit HTML report.
        #[arg(long)]
        html: bool,
        /// Fail if line coverage drops below N percent.
        #[arg(long, default_value = "75")]
        fail_under_lines: u8,
        /// Fail if branch coverage drops below N percent.
        #[arg(long, default_value = "60")]
        fail_under_branches: u8,
        /// Path to write the lcov file (default: `lcov.info`).
        #[arg(long, default_value = "lcov.info")]
        lcov_output: String,
    },
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    match run(cli) {
        Ok(report) => {
            if report.has_failures() {
                process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("ransible-test: error: {e:#}");
            process::exit(2);
        }
    }
}

fn run(cli: Cli) -> Result<TestReport> {
    match cli.command {
        Command::Sanity { crates } => {
            let runner = TestRunner::new(RunConfig { json: cli.json, crates });
            let report = runner.run_sanity()?;
            reporter::print_report(&report, cli.json);
            Ok(report)
        }
        Command::Units { crates, coverage } => {
            let runner = TestRunner::new(RunConfig { json: cli.json, crates });
            let report = runner.run_units(coverage)?;
            reporter::print_report(&report, cli.json);
            Ok(report)
        }
        Command::Integration { targets, docker } => {
            let runner = TestRunner::new(RunConfig { json: cli.json, crates: targets });
            let report = runner.run_integration(docker)?;
            reporter::print_report(&report, cli.json);
            Ok(report)
        }
        Command::Coverage { html, fail_under_lines, fail_under_branches, lcov_output } => {
            let cfg = CoverageConfig {
                html,
                fail_under_lines,
                fail_under_branches,
                lcov_output,
            };
            let report = coverage::run_coverage(cfg)?;
            reporter::print_report(&report, cli.json);
            Ok(report)
        }
    }
}
