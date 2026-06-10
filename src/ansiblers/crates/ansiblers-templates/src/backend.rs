//! Template rendering backend selection (Phase 6, Weeks 43+).
//!
//! Allows switching the Jinja2 rendering backend at runtime via the
//! `ANSIBLE_TEMPLATE_BACKEND` environment variable or by constructing a
//! [`BackendSelector`] directly.
//!
//! ## Backends
//!
//! | Variant | `ANSIBLE_TEMPLATE_BACKEND` | Description | Compat | Speed |
//! |---------|---------------------------|-------------|--------|-------|
//! | [`TemplateBackend::Jinja2rs`] | `jinja2rs` (default) | jinja2rs with Ansible compat layer | ~95% + Ansible filters | ★★★★ |
//! | [`TemplateBackend::Minijinja`] | `minijinja` | Raw minijinja, no Ansible compat | ~95% Jinja2 | ★★★★★ |
//! | [`TemplateBackend::PythonJinja2`] | `python` | CPython Jinja2 via subprocess | 100% | ★ |
//!
//! ## Choosing a backend
//!
//! - **`jinja2rs`** (default): production use — Ansible filters (`combine`,
//!   `regex_replace`, `to_nice_json`, `quote`, etc.) are fully available.
//! - **`minijinja`**: fastest raw render; use for benchmarking or when Ansible
//!   filters are not needed.
//! - **`python`**: guaranteed 100% Jinja2 compatibility; use to diagnose
//!   minijinja/jinja2rs rendering differences.  Requires `python3` + `jinja2`
//!   on the control node.
//!
//! ## Environment variable override
//!
//! ```bash
//! # Use jinja2rs with Ansible compat (default):
//! ANSIBLE_TEMPLATE_BACKEND=jinja2rs ransible-playbook site.yml
//!
//! # Raw minijinja (no Ansible filters):
//! ANSIBLE_TEMPLATE_BACKEND=minijinja ransible-playbook site.yml
//!
//! # CPython Jinja2 (compatibility testing):
//! ANSIBLE_TEMPLATE_BACKEND=python ransible-playbook site.yml
//! ```
//!
//! ## Programmatic use
//!
//! ```rust,no_run
//! use ansiblers_templates::backend::{BackendSelector, TemplateBackend};
//! use std::collections::HashMap;
//! use serde_json::Value;
//!
//! let vars: HashMap<String, Value> = [("name".into(), Value::String("Alice".into()))].into();
//!
//! // Read backend from ANSIBLE_TEMPLATE_BACKEND env var (defaults to Jinja2rs):
//! let selector = BackendSelector::from_env();
//! let out = selector.render("Hello {{ name }}!", &vars).unwrap();
//! assert_eq!(out, "Hello Alice!");
//!
//! // Force a specific backend:
//! let selector = BackendSelector::new(TemplateBackend::Minijinja);
//! let out = selector.render("{{ name | upper }}", &vars).unwrap();
//! assert_eq!(out, "ALICE");
//! ```

use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};

use ansiblers_core::Value;
use anyhow::{bail, Context, Result};
use tracing::debug;

use crate::engine::AnsibleTemplateEngine;

// ---------------------------------------------------------------------------
// TemplateBackend
// ---------------------------------------------------------------------------

/// Selects the Jinja2 rendering implementation.
///
/// See the [module-level docs](self) for a comparison table and guidance on
/// when to use each backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TemplateBackend {
    /// `jinja2rs` with Ansible compatibility mode (default).
    ///
    /// Wraps minijinja with the `jinja2rs` compat layer which provides
    /// Ansible-standard filters (`combine`, `regex_replace`, `to_nice_json`,
    /// `quote`, `path_join`, etc.) and the `AnsibleMode` undefined behaviour.
    ///
    /// **This is the correct backend for running real Ansible playbooks.**
    #[default]
    Jinja2rs,

    /// Raw minijinja — no Ansible compat layer.
    ///
    /// Fastest render path.  Built-in minijinja filters (`upper`, `lower`,
    /// `join`, `split`, `items`, etc.) are available but Ansible-specific
    /// filters are **not** registered.
    ///
    /// Use for:
    /// - Benchmarking the raw render cost
    /// - Templates that only use standard Jinja2 built-ins
    /// - Comparing minijinja vs jinja2rs rendering for compat debugging
    Minijinja,

    /// CPython Jinja2 rendered via a `python3` subprocess.
    ///
    /// Provides 100% compatibility with the Python Jinja2 reference
    /// implementation.  Requires `python3` and the `jinja2` package to be
    /// installed on the control node.
    ///
    /// **Performance**: ~5–20 ms per render (subprocess overhead).
    /// Use for compatibility testing only, not production hot paths.
    PythonJinja2,
}

impl TemplateBackend {
    /// Parse from a string (case-insensitive).
    ///
    /// | Input | Backend |
    /// |-------|---------|
    /// | `"jinja2rs"`, `"jinja2r2"`, `"compat"` | [`Jinja2rs`](Self::Jinja2rs) |
    /// | `"minijinja"`, `"rust"` | [`Minijinja`](Self::Minijinja) |
    /// | `"python"`, `"python_jinja2"`, `"cpython"` | [`PythonJinja2`](Self::PythonJinja2) |
    /// | anything else | [`Jinja2rs`](Self::Jinja2rs) (default) |
    pub fn from_str(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "jinja2rs" | "jinja2r2" | "compat" => Self::Jinja2rs,
            "minijinja" | "rust" => Self::Minijinja,
            "python" | "python_jinja2" | "cpython" => Self::PythonJinja2,
            _ => Self::Jinja2rs,
        }
    }

    /// Read the backend from the `ANSIBLE_TEMPLATE_BACKEND` environment
    /// variable.  Falls back to [`Jinja2rs`](Self::Jinja2rs) if unset.
    pub fn from_env() -> Self {
        match std::env::var("ANSIBLE_TEMPLATE_BACKEND") {
            Ok(val) => {
                let backend = Self::from_str(&val);
                debug!(
                    backend = backend.name(),
                    env = %val,
                    "ANSIBLE_TEMPLATE_BACKEND set"
                );
                backend
            }
            Err(_) => Self::Jinja2rs,
        }
    }

    /// Canonical short name for this backend (matches the env-var token).
    pub fn name(&self) -> &'static str {
        match self {
            Self::Jinja2rs => "jinja2rs",
            Self::Minijinja => "minijinja",
            Self::PythonJinja2 => "python_jinja2",
        }
    }
}

// ---------------------------------------------------------------------------
// BackendSelector
// ---------------------------------------------------------------------------

/// Wraps a chosen [`TemplateBackend`] and provides a uniform `render` API.
///
/// The standard entrypoint is [`BackendSelector::from_env()`], which reads
/// the `ANSIBLE_TEMPLATE_BACKEND` environment variable and defaults to
/// [`Jinja2rs`](TemplateBackend::Jinja2rs).
///
/// For tests or benchmarks, construct with [`BackendSelector::new`] to pin
/// a specific backend.
#[derive(Debug, Clone, Copy, Default)]
pub struct BackendSelector {
    backend: TemplateBackend,
}

impl BackendSelector {
    /// Create a selector with the given backend.
    pub fn new(backend: TemplateBackend) -> Self {
        Self { backend }
    }

    /// Create a selector, reading the backend from `ANSIBLE_TEMPLATE_BACKEND`.
    pub fn from_env() -> Self {
        Self::new(TemplateBackend::from_env())
    }

    /// The active backend.
    pub fn backend(&self) -> TemplateBackend {
        self.backend
    }

    /// Render `template_str` against `vars` using the selected backend.
    pub fn render(&self, template_str: &str, vars: &HashMap<String, Value>) -> Result<String> {
        match self.backend {
            TemplateBackend::Jinja2rs => {
                // jinja2rs AnsibleMode: Ansible filters + compat undefined behaviour.
                AnsibleTemplateEngine::new().render(template_str, vars)
            }
            TemplateBackend::Minijinja => render_via_minijinja(template_str, vars),
            TemplateBackend::PythonJinja2 => render_via_python(template_str, vars),
        }
    }
}

// ---------------------------------------------------------------------------
// Minijinja direct renderer (no jinja2rs compat layer)
// ---------------------------------------------------------------------------

/// Render a template using raw minijinja — no Ansible compat layer.
///
/// Only standard minijinja built-in filters are available.  Ansible-specific
/// filters (`combine`, `regex_replace`, `to_nice_json`, etc.) are **not**
/// registered.
///
/// Use for benchmarking or when only standard Jinja2 built-ins are needed.
pub fn render_via_minijinja(template_str: &str, vars: &HashMap<String, Value>) -> Result<String> {
    let env = minijinja::Environment::new();
    env.render_str(template_str, vars)
        .with_context(|| format!("minijinja rendering: {template_str}"))
}

// ---------------------------------------------------------------------------
// Python Jinja2 subprocess renderer
// ---------------------------------------------------------------------------

/// Render a template via a `python3` subprocess using the CPython Jinja2 library.
///
/// Variables are serialised to JSON, passed to Python on stdin, and the
/// rendered string is returned from stdout.
///
/// # Errors
///
/// Returns an error if `python3` cannot be spawned, `jinja2` is not installed,
/// or if the template rendering fails.
pub fn render_via_python(template_str: &str, vars: &HashMap<String, Value>) -> Result<String> {
    let python_script = r#"
import sys, json, jinja2

payload = json.load(sys.stdin)
template_str = payload["template"]
variables = payload["vars"]

env = jinja2.Environment(undefined=jinja2.Undefined)
result = env.from_string(template_str).render(variables)
sys.stdout.write(result)
"#;

    let payload = serde_json::json!({
        "template": template_str,
        "vars": serde_json::to_value(vars).context("serialising vars for Python Jinja2")?
    });
    let payload_str = serde_json::to_string(&payload)?;

    let mut child = Command::new("python3")
        .args(["-c", python_script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning python3 for Jinja2 rendering (is python3 + jinja2 installed?)")?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(payload_str.as_bytes())
            .context("writing template payload to python3 stdin")?;
    }

    let output = child
        .wait_with_output()
        .context("waiting for python3 Jinja2 subprocess")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "python3 Jinja2 rendering failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        );
    }

    String::from_utf8(output.stdout).context("decoding python3 Jinja2 output as UTF-8")
}

// ---------------------------------------------------------------------------
// Convenience function
// ---------------------------------------------------------------------------

/// Render `template_str` using the backend selected by `ANSIBLE_TEMPLATE_BACKEND`.
///
/// Defaults to [`Jinja2rs`](TemplateBackend::Jinja2rs) when the environment
/// variable is unset.
pub fn render_with_selected_backend(
    template_str: &str,
    vars: &HashMap<String, Value>,
) -> Result<String> {
    BackendSelector::from_env().render(template_str, vars)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn v(pairs: &[(&str, &str)]) -> HashMap<String, Value> {
        pairs
            .iter()
            .map(|(k, val)| (k.to_string(), Value::String(val.to_string())))
            .collect()
    }

    fn python3_jinja2_available() -> bool {
        Command::new("python3")
            .args(["-c", "import jinja2"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    // ---- TemplateBackend::from_str ----------------------------------------

    #[rstest]
    #[case("jinja2rs",       TemplateBackend::Jinja2rs)]
    #[case("jinja2r2",       TemplateBackend::Jinja2rs)]
    #[case("compat",         TemplateBackend::Jinja2rs)]
    #[case("Jinja2rs",       TemplateBackend::Jinja2rs)]
    #[case("JINJA2RS",       TemplateBackend::Jinja2rs)]
    #[case("minijinja",      TemplateBackend::Minijinja)]
    #[case("rust",           TemplateBackend::Minijinja)]
    #[case("MINIJINJA",      TemplateBackend::Minijinja)]
    #[case("python",         TemplateBackend::PythonJinja2)]
    #[case("Python",         TemplateBackend::PythonJinja2)]
    #[case("PYTHON",         TemplateBackend::PythonJinja2)]
    #[case("python_jinja2",  TemplateBackend::PythonJinja2)]
    #[case("cpython",        TemplateBackend::PythonJinja2)]
    #[case("",               TemplateBackend::Jinja2rs)]
    #[case("unknown",        TemplateBackend::Jinja2rs)]
    fn test_backend_from_str(#[case] input: &str, #[case] expected: TemplateBackend) {
        assert_eq!(TemplateBackend::from_str(input), expected);
    }

    // ---- backend name() ---------------------------------------------------

    #[test]
    fn test_backend_name() {
        assert_eq!(TemplateBackend::Jinja2rs.name(), "jinja2rs");
        assert_eq!(TemplateBackend::Minijinja.name(), "minijinja");
        assert_eq!(TemplateBackend::PythonJinja2.name(), "python_jinja2");
    }

    // ---- Jinja2rs backend ------------------------------------------------

    #[rstest]
    #[case("Hello {{ name }}!", &[("name", "World")], "Hello World!")]
    #[case("{{ x | upper }}", &[("x", "hello")], "HELLO")]
    #[case("static", &[], "static")]
    fn test_jinja2rs_render(
        #[case] tmpl: &str,
        #[case] pairs: &[(&str, &str)],
        #[case] expected: &str,
    ) {
        let sel = BackendSelector::new(TemplateBackend::Jinja2rs);
        assert_eq!(sel.render(tmpl, &v(pairs)).unwrap(), expected);
    }

    // ---- Minijinja (raw) backend ------------------------------------------

    #[rstest]
    #[case("Hello {{ name }}!", &[("name", "World")], "Hello World!")]
    #[case("{{ x | upper }}", &[("x", "hello")], "HELLO")]
    #[case("static", &[], "static")]
    fn test_minijinja_render(
        #[case] tmpl: &str,
        #[case] pairs: &[(&str, &str)],
        #[case] expected: &str,
    ) {
        let sel = BackendSelector::new(TemplateBackend::Minijinja);
        assert_eq!(sel.render(tmpl, &v(pairs)).unwrap(), expected);
    }

    #[test]
    fn test_minijinja_no_vars() {
        let sel = BackendSelector::new(TemplateBackend::Minijinja);
        assert_eq!(sel.render("no vars", &HashMap::new()).unwrap(), "no vars");
    }

    // ---- BackendSelector::from_env ----------------------------------------

    #[test]
    fn test_from_env_default_is_jinja2rs() {
        std::env::remove_var("ANSIBLE_TEMPLATE_BACKEND");
        assert_eq!(BackendSelector::from_env().backend(), TemplateBackend::Jinja2rs);
    }

    #[test]
    fn test_from_env_minijinja_override() {
        std::env::set_var("ANSIBLE_TEMPLATE_BACKEND", "minijinja");
        assert_eq!(BackendSelector::from_env().backend(), TemplateBackend::Minijinja);
        std::env::remove_var("ANSIBLE_TEMPLATE_BACKEND");
    }

    #[test]
    fn test_from_env_python_override() {
        std::env::set_var("ANSIBLE_TEMPLATE_BACKEND", "python");
        assert_eq!(BackendSelector::from_env().backend(), TemplateBackend::PythonJinja2);
        std::env::remove_var("ANSIBLE_TEMPLATE_BACKEND");
    }

    #[test]
    fn test_from_env_jinja2r2_alias() {
        std::env::set_var("ANSIBLE_TEMPLATE_BACKEND", "jinja2r2");
        assert_eq!(BackendSelector::from_env().backend(), TemplateBackend::Jinja2rs);
        std::env::remove_var("ANSIBLE_TEMPLATE_BACKEND");
    }

    // ---- render_with_selected_backend default ----------------------------

    #[test]
    fn test_render_with_selected_backend_default() {
        std::env::remove_var("ANSIBLE_TEMPLATE_BACKEND");
        let out = render_with_selected_backend("{{ k }}", &v(&[("k", "v")])).unwrap();
        assert_eq!(out, "v");
    }

    // ---- Python Jinja2 backend (skipped if python3/jinja2 unavailable) ----

    #[test]
    fn test_python_backend_simple() {
        if !python3_jinja2_available() {
            return;
        }
        let sel = BackendSelector::new(TemplateBackend::PythonJinja2);
        let out = sel.render("{{ greeting }} world", &v(&[("greeting", "hi")])).unwrap();
        assert_eq!(out, "hi world");
    }

    #[test]
    fn test_python_backend_filter_upper() {
        if !python3_jinja2_available() {
            return;
        }
        let sel = BackendSelector::new(TemplateBackend::PythonJinja2);
        let out = sel.render("{{ x | upper }}", &v(&[("x", "hello")])).unwrap();
        assert_eq!(out, "HELLO");
    }

    // ---- render_via_minijinja direct function ----------------------------

    #[test]
    fn test_render_via_minijinja_direct() {
        let out = render_via_minijinja("{{ a }}-{{ b }}", &v(&[("a", "foo"), ("b", "bar")])).unwrap();
        assert_eq!(out, "foo-bar");
    }

    // ---- Cross-backend output parity (jinja2rs vs minijinja) -------------

    #[rstest]
    #[case("Hello {{ name }}", &[("name", "Alice")])]
    #[case("{{ x | upper }}", &[("x", "test")])]
    #[case("{% for i in items %}{{ i }}{% endfor %}", &[])]
    fn test_jinja2rs_vs_minijinja_parity(#[case] tmpl: &str, #[case] pairs: &[(&str, &str)]) {
        let vars = v(pairs);
        let r1 = BackendSelector::new(TemplateBackend::Jinja2rs).render(tmpl, &vars);
        let r2 = BackendSelector::new(TemplateBackend::Minijinja).render(tmpl, &vars);
        // Both succeed or both fail; when both succeed they must agree.
        match (r1, r2) {
            (Ok(a), Ok(b)) => assert_eq!(a, b, "jinja2rs vs minijinja output differs for: {tmpl}"),
            (Err(_), Err(_)) => {} // both failed — acceptable for edge cases
            (Ok(a), Err(e)) => panic!("jinja2rs succeeded ({a:?}) but minijinja failed: {e}"),
            (Err(e), Ok(b)) => panic!("minijinja succeeded ({b:?}) but jinja2rs failed: {e}"),
        }
    }
}
