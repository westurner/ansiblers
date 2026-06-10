//! Contribution report generator (Phase 6, Weeks 43+).
//!
//! Produces a structured report comparing ansiblers vs ansible-playbook
//! performance, identifying hot paths suitable for upstream contribution to
//! Ansible Core.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use ansiblers_compat::contrib::{ContributionReport, HotPathAnalysis};
//!
//! let report = ContributionReport::builder()
//!     .with_hot_path("template_render", 15_000, 2_100, "ansiblers-templates backend switch")
//!     .with_hot_path("variable_resolve", 8_000, 1_200, "VariableResolver precedence cache")
//!     .with_hot_path("inventory_load", 250_000, 35_000, "INI/YAML parser in Rust")
//!     .build();
//!
//! println!("{}", report.markdown());
//! ```
//!
//! ## Python API
//!
//! ```python
//! import ansiblers
//!
//! report = ansiblers.ContributionReport()
//! report.add_hot_path("template_render", python_us=15000, rust_us=2100,
//!                     description="minijinja/jinja2rs rendering")
//! print(report.markdown())
//! ```

use std::fmt::Write as FmtWrite;

use pyo3::prelude::*;

// ---------------------------------------------------------------------------
// HotPathAnalysis
// ---------------------------------------------------------------------------

/// A single measured hot-path comparison between Python Ansible and ansiblers.
#[derive(Debug, Clone)]
pub struct HotPathAnalysis {
    /// Name of the operation (e.g. `"template_render"`).
    pub name: String,
    /// Median time in the Python implementation, in microseconds.
    pub python_us: u64,
    /// Median time in the Rust implementation, in microseconds.
    pub rust_us: u64,
    /// Human-readable description of the optimisation applied.
    pub description: String,
}

impl HotPathAnalysis {
    /// Speedup factor (python_us / rust_us), or 1.0 if rust_us is zero.
    pub fn speedup(&self) -> f64 {
        if self.rust_us == 0 {
            return f64::INFINITY;
        }
        self.python_us as f64 / self.rust_us as f64
    }

    /// Time saved per call in microseconds.
    pub fn saved_us(&self) -> i64 {
        self.python_us as i64 - self.rust_us as i64
    }

    /// `true` if the Rust implementation is strictly faster.
    pub fn is_improvement(&self) -> bool {
        self.rust_us < self.python_us
    }
}

// ---------------------------------------------------------------------------
// ContributionReport
// ---------------------------------------------------------------------------

/// Aggregated report of hot-path analyses, formatted for upstream contribution
/// discussions with the Ansible Core team.
#[derive(Debug, Clone, Default)]
pub struct ContributionReport {
    analyses: Vec<HotPathAnalysis>,
    notes: Vec<String>,
}

impl ContributionReport {
    /// Create an empty report.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start building a report with the fluent builder API.
    pub fn builder() -> ContributionReportBuilder {
        ContributionReportBuilder::new()
    }

    /// Add a hot-path analysis.
    pub fn add_analysis(&mut self, analysis: HotPathAnalysis) {
        self.analyses.push(analysis);
    }

    /// Add a free-text note to the report.
    pub fn add_note(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
    }

    /// Total time saved per call across all hot paths (µs).
    pub fn total_saved_us(&self) -> i64 {
        self.analyses.iter().map(|a| a.saved_us()).sum()
    }

    /// Geometric-mean speedup across all analyses.
    pub fn mean_speedup(&self) -> f64 {
        if self.analyses.is_empty() {
            return 1.0;
        }
        let log_sum: f64 = self
            .analyses
            .iter()
            .map(|a| a.speedup().ln())
            .sum::<f64>();
        (log_sum / self.analyses.len() as f64).exp()
    }

    /// Render the report as a Markdown table.
    pub fn markdown(&self) -> String {
        let mut out = String::new();
        writeln!(out, "# ansiblers Hot-Path Contribution Report").ok();
        writeln!(out).ok();
        writeln!(out, "## Summary").ok();
        writeln!(
            out,
            "- Total savings: **{:.1} µs/call** across {} hot paths",
            self.total_saved_us(),
            self.analyses.len()
        )
        .ok();
        writeln!(
            out,
            "- Geometric-mean speedup: **{:.1}×**",
            self.mean_speedup()
        )
        .ok();
        writeln!(out).ok();
        writeln!(
            out,
            "| Hot Path | Python (µs) | Rust (µs) | Speedup | Description |"
        )
        .ok();
        writeln!(
            out,
            "|----------|-------------|-----------|---------|-------------|"
        )
        .ok();
        for a in &self.analyses {
            writeln!(
                out,
                "| `{}` | {:>11} | {:>9} | {:>6.1}× | {} |",
                a.name,
                a.python_us,
                a.rust_us,
                a.speedup(),
                a.description,
            )
            .ok();
        }
        if !self.notes.is_empty() {
            writeln!(out).ok();
            writeln!(out, "## Notes").ok();
            for note in &self.notes {
                writeln!(out, "- {note}").ok();
            }
        }
        out
    }

    /// Render as JSON (for CI artifact upload / dashboards).
    pub fn json(&self) -> String {
        let analyses: Vec<serde_json::Value> = self
            .analyses
            .iter()
            .map(|a| {
                serde_json::json!({
                    "name": a.name,
                    "python_us": a.python_us,
                    "rust_us": a.rust_us,
                    "speedup": a.speedup(),
                    "saved_us": a.saved_us(),
                    "description": a.description,
                })
            })
            .collect();
        serde_json::json!({
            "total_saved_us": self.total_saved_us(),
            "mean_speedup": self.mean_speedup(),
            "analyses": analyses,
            "notes": self.notes,
        })
        .to_string()
    }
}

// ---------------------------------------------------------------------------
// ContributionReportBuilder (fluent)
// ---------------------------------------------------------------------------

/// Fluent builder for [`ContributionReport`].
#[derive(Default)]
pub struct ContributionReportBuilder {
    inner: ContributionReport,
}

impl ContributionReportBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a hot-path measurement.
    pub fn with_hot_path(
        mut self,
        name: impl Into<String>,
        python_us: u64,
        rust_us: u64,
        description: impl Into<String>,
    ) -> Self {
        self.inner.add_analysis(HotPathAnalysis {
            name: name.into(),
            python_us,
            rust_us,
            description: description.into(),
        });
        self
    }

    /// Add a free-text note.
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.inner.add_note(note);
        self
    }

    pub fn build(self) -> ContributionReport {
        self.inner
    }
}

// ---------------------------------------------------------------------------
// PyContributionReport — Python-facing wrapper
// ---------------------------------------------------------------------------

/// Python-accessible contribution report.
///
/// ```python
/// import ansiblers
///
/// report = ansiblers.ContributionReport()
/// report.add_hot_path("template_render", python_us=15000, rust_us=2100,
///                     description="jinja2rs replaces Python Jinja2 on hot render path")
/// report.add_hot_path("variable_resolve", python_us=8000, rust_us=1200,
///                     description="VariableResolver precedence cache in Rust")
/// print(report.markdown())
/// # Output: Markdown table with speedup column
/// ```
#[pyclass(name = "ContributionReport")]
pub struct PyContributionReport {
    inner: ContributionReport,
}

#[pymethods]
impl PyContributionReport {
    #[new]
    fn new() -> Self {
        Self { inner: ContributionReport::new() }
    }

    /// Add a hot-path measurement.
    ///
    /// Args:
    ///     name: Short identifier (e.g. `"template_render"`).
    ///     python_us: Median Python execution time in microseconds.
    ///     rust_us: Median Rust execution time in microseconds.
    ///     description: Human-readable description of the optimisation.
    #[pyo3(signature = (name, python_us, rust_us, description=""))]
    fn add_hot_path(
        &mut self,
        name: &str,
        python_us: u64,
        rust_us: u64,
        description: &str,
    ) {
        self.inner.add_analysis(HotPathAnalysis {
            name: name.to_string(),
            python_us,
            rust_us,
            description: description.to_string(),
        });
    }

    /// Add a free-text note appended to the report.
    fn add_note(&mut self, note: &str) {
        self.inner.add_note(note);
    }

    /// Return the report as a Markdown string.
    fn markdown(&self) -> String {
        self.inner.markdown()
    }

    /// Return the report as a JSON string.
    fn json(&self) -> String {
        self.inner.json()
    }

    /// Total time saved per call across all hot paths, in microseconds.
    fn total_saved_us(&self) -> i64 {
        self.inner.total_saved_us()
    }

    /// Geometric-mean speedup across all analyses.
    fn mean_speedup(&self) -> f64 {
        self.inner.mean_speedup()
    }

    fn __repr__(&self) -> String {
        format!(
            "ContributionReport(paths={}, mean_speedup={:.1}x)",
            self.inner.analyses.len(),
            self.inner.mean_speedup()
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_report() -> ContributionReport {
        ContributionReport::builder()
            .with_hot_path("template_render",  15_000, 2_100, "jinja2rs")
            .with_hot_path("variable_resolve",  8_000, 1_200, "VariableResolver")
            .with_hot_path("inventory_load",  250_000, 35_000, "Rust INI/YAML parser")
            .build()
    }

    #[test]
    fn test_speedup_calculation() {
        let a = HotPathAnalysis {
            name: "x".into(),
            python_us: 10_000,
            rust_us: 2_000,
            description: "5x".into(),
        };
        assert!((a.speedup() - 5.0).abs() < 1e-9);
        assert_eq!(a.saved_us(), 8_000);
        assert!(a.is_improvement());
    }

    #[test]
    fn test_speedup_zero_rust() {
        let a = HotPathAnalysis {
            name: "x".into(),
            python_us: 1_000,
            rust_us: 0,
            description: "".into(),
        };
        assert!(a.speedup().is_infinite());
    }

    #[test]
    fn test_total_saved() {
        let r = sample_report();
        // (15000-2100) + (8000-1200) + (250000-35000) = 12900 + 6800 + 215000 = 234700
        assert_eq!(r.total_saved_us(), 234_700);
    }

    #[test]
    fn test_mean_speedup_positive() {
        let r = sample_report();
        assert!(r.mean_speedup() > 1.0);
    }

    #[test]
    fn test_empty_report_mean_speedup() {
        let r = ContributionReport::new();
        assert!((r.mean_speedup() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_markdown_contains_table() {
        let r = sample_report();
        let md = r.markdown();
        assert!(md.contains("template_render"));
        assert!(md.contains("variable_resolve"));
        assert!(md.contains("inventory_load"));
        assert!(md.contains("Speedup"));
        assert!(md.contains("Geometric-mean speedup"));
    }

    #[test]
    fn test_json_roundtrip() {
        let r = sample_report();
        let json_str = r.json();
        let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(v["analyses"].is_array());
        assert_eq!(v["analyses"].as_array().unwrap().len(), 3);
        assert!(v["mean_speedup"].as_f64().unwrap() > 1.0);
    }

    #[test]
    fn test_builder_with_note() {
        let r = ContributionReport::builder()
            .with_hot_path("x", 100, 10, "desc")
            .with_note("See issue #1234")
            .build();
        let md = r.markdown();
        assert!(md.contains("See issue #1234"));
    }

    #[test]
    fn test_py_contribution_report_struct() {
        let mut pr = PyContributionReport::new();
        pr.add_hot_path("template_render", 15_000, 2_100, "jinja2rs");
        assert!((pr.mean_speedup() - 15_000.0 / 2_100.0).abs() < 0.01);
        assert_eq!(pr.total_saved_us(), 12_900);
        assert!(pr.markdown().contains("template_render"));
    }

    #[test]
    fn test_py_repr() {
        let pr = PyContributionReport::new();
        assert_eq!(pr.__repr__(), "ContributionReport(paths=0, mean_speedup=1.0x)");
    }
}
