//! Test runner — invokes `cargo` subcommands and collects results.

use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result};
use tracing::{debug, info, warn};

use crate::reporter::{CheckResult, CheckStatus, TestReport};

/// Configuration shared across all runner invocations.
#[derive(Debug, Default)]
pub struct RunConfig {
    /// Emit results as JSON.
    pub json: bool,
    /// Crate / target filter list (empty ⇒ all).
    pub crates: Vec<String>,
}

/// Orchestrates `cargo` invocations.
pub struct TestRunner {
    config: RunConfig,
    workspace_root: PathBuf,
}

impl TestRunner {
    /// Create a runner, auto-detecting the workspace root from the environment.
    pub fn new(config: RunConfig) -> Self {
        // Try $CARGO_MANIFEST_DIR first (set when invoked by cargo), otherwise
        // fall back to the current working directory.
        let workspace_root = std::env::var("CARGO_MANIFEST_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default());
        Self { config, workspace_root }
    }

    /// Create a runner with an explicit workspace root (useful in tests).
    pub fn with_root(config: RunConfig, workspace_root: PathBuf) -> Self {
        Self { config, workspace_root }
    }

    // ------------------------------------------------------------------
    // Sanity checks
    // ------------------------------------------------------------------

    /// Run formatting and lint checks.
    pub fn run_sanity(&self) -> Result<TestReport> {
        info!("Running sanity checks…");
        let mut results = Vec::new();

        // cargo fmt --check
        results.push(self.run_check("fmt", &["fmt", "--all", "--", "--check"]));

        // cargo clippy
        results.push(self.run_check(
            "clippy",
            &["clippy", "--all-targets", "--", "-D", "warnings"],
        ));

        Ok(TestReport { suite: "sanity".into(), results })
    }

    // ------------------------------------------------------------------
    // Unit tests
    // ------------------------------------------------------------------

    /// Run `cargo test` for selected crates.
    pub fn run_units(&self, coverage: bool) -> Result<TestReport> {
        info!("Running unit tests (coverage={coverage})…");
        let mut args: Vec<&str> = Vec::new();

        if coverage {
            // cargo +nightly llvm-cov --all
            args.extend_from_slice(&["+nightly", "llvm-cov", "--all"]);
        } else {
            args.push("test");
            if self.config.crates.is_empty() {
                args.push("--all");
            }
        }

        if !self.config.crates.is_empty() && !coverage {
            // Add per-crate -p flags only in the non-coverage path
            // (they get added via env below in coverage mode).
        }

        let result = self.run_check("units", &args);
        Ok(TestReport { suite: "units".into(), results: vec![result] })
    }

    // ------------------------------------------------------------------
    // Integration tests
    // ------------------------------------------------------------------

    /// Run integration tests (the `ansiblers-it` package).
    pub fn run_integration(&self, docker: bool) -> Result<TestReport> {
        info!("Running integration tests (docker={docker})…");
        if docker {
            warn!("Docker mode is not yet implemented; running locally");
        }

        let mut args = vec!["test", "-p", "ansiblers-it", "--test"];

        let target_args: Vec<String> = if self.config.crates.is_empty() {
            // Run all integration test binaries
            vec![
                "test_playbook_parsing".into(),
                "test_inventory".into(),
                "test_playbook_execution".into(),
                "test_phase2_modules".into(),
                "test_molecule".into(),
                "test_phase3_roles".into(),
                "test_phase4_build".into(),
            ]
        } else {
            self.config.crates.clone()
        };

        let mut results = Vec::new();
        for target in &target_args {
            let label = format!("integration::{target}");
            let check_args: Vec<&str> =
                vec!["test", "-p", "ansiblers-it", "--test", target.as_str()];
            results.push(self.run_check(&label, &check_args));
        }

        Ok(TestReport { suite: "integration".into(), results })
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    fn run_check(&self, name: &str, args: &[&str]) -> CheckResult {
        debug!("cargo {}", args.join(" "));
        let start = Instant::now();

        let status = Command::new("cargo")
            .args(args)
            .current_dir(&self.workspace_root)
            .status();

        let elapsed_ms = start.elapsed().as_millis() as u64;

        match status {
            Ok(s) if s.success() => CheckResult {
                name: name.into(),
                status: CheckStatus::Passed,
                elapsed_ms,
                message: None,
            },
            Ok(s) => CheckResult {
                name: name.into(),
                status: CheckStatus::Failed,
                elapsed_ms,
                message: Some(format!("exited with status {s}")),
            },
            Err(e) => CheckResult {
                name: name.into(),
                status: CheckStatus::Error,
                elapsed_ms,
                message: Some(format!("failed to run cargo: {e}")),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runner_new_does_not_panic() {
        let runner = TestRunner::new(RunConfig::default());
        assert!(runner.workspace_root.is_absolute() || !runner.workspace_root.as_os_str().is_empty());
    }

    #[test]
    fn test_run_config_default() {
        let cfg = RunConfig::default();
        assert!(!cfg.json);
        assert!(cfg.crates.is_empty());
    }
}
