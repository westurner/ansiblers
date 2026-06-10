//! Result types and output formatting for ransible-test.

use serde::{Deserialize, Serialize};

/// Whether a single check passed, failed, or errored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    /// The check ran and succeeded.
    Passed,
    /// The check ran and reported failures.
    Failed,
    /// The check could not be run (e.g. tool not installed).
    Error,
}

/// Result for one check within a test suite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    /// Human-readable check name.
    pub name: String,
    /// Outcome of this check.
    pub status: CheckStatus,
    /// Wall-clock time in milliseconds.
    pub elapsed_ms: u64,
    /// Optional detail message.
    pub message: Option<String>,
}

impl CheckResult {
    /// Returns `true` if this check did not pass.
    pub fn is_failure(&self) -> bool {
        matches!(self.status, CheckStatus::Failed | CheckStatus::Error)
    }
}

/// Aggregated results for one `ransible-test` subcommand invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestReport {
    /// Suite name, e.g. `"sanity"`, `"units"`, `"integration"`, `"coverage"`.
    pub suite: String,
    /// Individual check results.
    pub results: Vec<CheckResult>,
}

impl TestReport {
    /// Returns `true` if any check failed or errored.
    pub fn has_failures(&self) -> bool {
        self.results.iter().any(|r| r.is_failure())
    }

    /// Count of passed checks.
    pub fn passed(&self) -> usize {
        self.results
            .iter()
            .filter(|r| r.status == CheckStatus::Passed)
            .count()
    }

    /// Count of failed / errored checks.
    pub fn failed(&self) -> usize {
        self.results.iter().filter(|r| r.is_failure()).count()
    }

    /// Total wall-clock time across all checks.
    pub fn total_elapsed_ms(&self) -> u64 {
        self.results.iter().map(|r| r.elapsed_ms).sum()
    }
}

/// Print the report to stdout in human or JSON format.
pub fn print_report(report: &TestReport, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(report).unwrap_or_default()
        );
        return;
    }

    println!("\nransible-test {} results:", report.suite);
    println!("{:-<50}", "");
    for r in &report.results {
        let icon = match r.status {
            CheckStatus::Passed => "✓",
            CheckStatus::Failed => "✗",
            CheckStatus::Error => "⚠",
        };
        print!("  {icon}  {:40} {:>6}ms", r.name, r.elapsed_ms);
        if let Some(msg) = &r.message {
            print!("  — {msg}");
        }
        println!();
    }
    println!("{:-<50}", "");
    println!(
        "  {} passed, {} failed  ({}ms total)",
        report.passed(),
        report.failed(),
        report.total_elapsed_ms(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_report() -> TestReport {
        TestReport {
            suite: "test".into(),
            results: vec![
                CheckResult {
                    name: "fmt".into(),
                    status: CheckStatus::Passed,
                    elapsed_ms: 100,
                    message: None,
                },
                CheckResult {
                    name: "clippy".into(),
                    status: CheckStatus::Failed,
                    elapsed_ms: 200,
                    message: Some("1 warning".into()),
                },
            ],
        }
    }

    #[test]
    fn test_has_failures() {
        assert!(make_report().has_failures());
    }

    #[test]
    fn test_passed_count() {
        assert_eq!(make_report().passed(), 1);
    }

    #[test]
    fn test_failed_count() {
        assert_eq!(make_report().failed(), 1);
    }

    #[test]
    fn test_total_elapsed() {
        assert_eq!(make_report().total_elapsed_ms(), 300);
    }

    #[test]
    fn test_json_round_trip() {
        let report = make_report();
        let json = serde_json::to_string(&report).unwrap();
        let back: TestReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.suite, report.suite);
        assert_eq!(back.results.len(), 2);
    }

    #[test]
    fn test_no_failures_report() {
        let report = TestReport {
            suite: "sanity".into(),
            results: vec![CheckResult {
                name: "fmt".into(),
                status: CheckStatus::Passed,
                elapsed_ms: 50,
                message: None,
            }],
        };
        assert!(!report.has_failures());
    }
}
