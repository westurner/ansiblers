//! Coverage integration via `cargo-llvm-cov`.

use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use anyhow::Result;
use tracing::info;

use crate::reporter::{CheckResult, CheckStatus, TestReport};

/// Configuration for a coverage run.
#[derive(Debug)]
pub struct CoverageConfig {
    /// Emit an HTML report to `target/llvm-cov/html/`.
    pub html: bool,
    /// Minimum acceptable line coverage percentage (0–100).
    pub fail_under_lines: u8,
    /// Minimum acceptable branch coverage percentage (0–100).
    pub fail_under_branches: u8,
    /// Path for the lcov output file.
    pub lcov_output: String,
}

impl Default for CoverageConfig {
    fn default() -> Self {
        Self {
            html: false,
            fail_under_lines: 75,
            fail_under_branches: 60,
            lcov_output: "lcov.info".into(),
        }
    }
}

/// Run `cargo +nightly llvm-cov` and collect results.
pub fn run_coverage(cfg: CoverageConfig) -> Result<TestReport> {
    info!(
        "Generating coverage report (lines≥{}%, branches≥{}%)…",
        cfg.fail_under_lines, cfg.fail_under_branches
    );

    let mut results = Vec::new();

    // Step 1: generate lcov data
    results.push(run_llvm_cov_lcov(&cfg));

    // Step 2: generate HTML if requested
    if cfg.html {
        results.push(run_llvm_cov_html());
    }

    // Step 3: check thresholds
    results.push(check_thresholds(&cfg));

    Ok(TestReport { suite: "coverage".into(), results })
}

fn run_llvm_cov_lcov(cfg: &CoverageConfig) -> CheckResult {
    let start = Instant::now();
    let status = Command::new("cargo")
        .args([
            "+nightly",
            "llvm-cov",
            "--all",
            "--lcov",
            "--output-path",
            &cfg.lcov_output,
        ])
        .status();

    let elapsed_ms = start.elapsed().as_millis() as u64;
    match status {
        Ok(s) if s.success() => CheckResult {
            name: "llvm-cov/lcov".into(),
            status: CheckStatus::Passed,
            elapsed_ms,
            message: Some(format!("written to {}", cfg.lcov_output)),
        },
        Ok(s) => CheckResult {
            name: "llvm-cov/lcov".into(),
            status: CheckStatus::Failed,
            elapsed_ms,
            message: Some(format!("cargo +nightly llvm-cov exited with {s}")),
        },
        Err(e) => CheckResult {
            name: "llvm-cov/lcov".into(),
            status: CheckStatus::Error,
            elapsed_ms,
            message: Some(format!("failed to run: {e}")),
        },
    }
}

fn run_llvm_cov_html() -> CheckResult {
    let start = Instant::now();
    let status = Command::new("cargo")
        .args(["+nightly", "llvm-cov", "report", "--html"])
        .status();
    let elapsed_ms = start.elapsed().as_millis() as u64;
    match status {
        Ok(s) if s.success() => CheckResult {
            name: "llvm-cov/html".into(),
            status: CheckStatus::Passed,
            elapsed_ms,
            message: Some("report at target/llvm-cov/html/index.html".into()),
        },
        Ok(s) => CheckResult {
            name: "llvm-cov/html".into(),
            status: CheckStatus::Failed,
            elapsed_ms,
            message: Some(format!("exited with {s}")),
        },
        Err(e) => CheckResult {
            name: "llvm-cov/html".into(),
            status: CheckStatus::Error,
            elapsed_ms,
            message: Some(format!("{e}")),
        },
    }
}

fn check_thresholds(cfg: &CoverageConfig) -> CheckResult {
    let start = Instant::now();
    let status = Command::new("cargo")
        .args([
            "+nightly",
            "llvm-cov",
            "report",
            &format!("--fail-under-lines={}", cfg.fail_under_lines),
            &format!("--fail-under-branches={}", cfg.fail_under_branches),
        ])
        .status();
    let elapsed_ms = start.elapsed().as_millis() as u64;
    match status {
        Ok(s) if s.success() => CheckResult {
            name: "coverage/thresholds".into(),
            status: CheckStatus::Passed,
            elapsed_ms,
            message: Some(format!(
                "lines≥{}% branches≥{}% — OK",
                cfg.fail_under_lines, cfg.fail_under_branches
            )),
        },
        Ok(s) => CheckResult {
            name: "coverage/thresholds".into(),
            status: CheckStatus::Failed,
            elapsed_ms,
            message: Some(format!("coverage below threshold (exit {s})")),
        },
        Err(e) => CheckResult {
            name: "coverage/thresholds".into(),
            status: CheckStatus::Error,
            elapsed_ms,
            message: Some(format!("{e}")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_thresholds() {
        let cfg = CoverageConfig::default();
        assert_eq!(cfg.fail_under_lines, 75);
        assert_eq!(cfg.fail_under_branches, 60);
    }
}
