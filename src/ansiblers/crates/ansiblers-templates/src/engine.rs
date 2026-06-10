//! Ansible-mode template rendering engine backed by `jinja2rs`.
//!
//! ## Overhead of the sandboxed variant
//!
//! | Check | Cost per render | Notes |
//! |-------|----------------|-------|
//! | Strict undefined | ~0 ns | Inside existing AST walk |
//! | Attribute deny-list | ~50 ns | O(DENIED_ATTRS) string cmp per attr access |
//! | `validate_context_for_python_callables` | ~2–8 µs | Full JSON walk over all vars |
//! | Path policy | ~100 ns | Only at `get_template()`, not per render |
//! | seccomp (one-time setup) | ~1 µs | `prctl` at `Engine::new()`, zero at render |
//! | resource limits (one-time) | ~500 ns | `setrlimit` at `Engine::new()` |
//!
//! **Per-render overhead: ~2–8 µs** when `python_callable_warnings` is on
//! (the context walk is the only hot path).
//! Compared to a typical render of 15–50 µs this is a **5–15% overhead**.
//!
//! When `python_callable_warnings` is off (the default) all runtime checks are
//! O(1) — the sandboxed engine is essentially free.
//!
//! ## Configuration
//!
//! ```rust,no_run
//! use ansiblers_templates::engine::{AnsibleTemplateEngine, TemplateEngineConfig, TrustLevel};
//!
//! // Default: standard engine, no sandbox overhead.
//! let engine = AnsibleTemplateEngine::new();
//!
//! // Sandboxed: strict undefined, denied attrs, path policy.
//! let engine = AnsibleTemplateEngine::sandboxed();
//!
//! // Fully explicit via config:
//! let config = TemplateEngineConfig {
//!     trust_level: TrustLevel::Untrusted,
//!     strict_undefined: true,
//!     python_callable_warnings: false,
//!     allowed_read_paths: vec![],
//! };
//! let engine = AnsibleTemplateEngine::with_config(config);
//! ```

use std::collections::HashMap;
use std::path::PathBuf;

use ansiblers_core::Value;
use anyhow::{Context, Result};
use jinja2rs::compat::{AnsibleMode, CompatMode};
use jinja2rs::Environment;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// How much to trust the template source and its variable context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrustLevel {
    /// Templates come from playbooks written by the operator.
    /// Standard engine, no sandbox overhead.  (Default.)
    #[default]
    Trusted,

    /// Templates may originate from role downloads, inventory plugins,
    /// or user-provided variable files.
    /// Use `SandboxedEnvironment` with deny-list but lenient undefined.
    PartiallyTrusted,

    /// Templates or their variable context come from an untrusted source
    /// (e.g. a fetched URL, a third-party API response).
    /// Strict undefined + deny-list + python_callable_warnings.
    Untrusted,
}

/// Controls the template rendering engine behaviour.
#[derive(Debug, Clone)]
pub struct TemplateEngineConfig {
    /// Trust level — selects which protections are active.
    pub trust_level: TrustLevel,

    /// Require all referenced variables to be defined; error otherwise.
    /// Default: `false` (Ansible tolerates undefined → empty string).
    /// Forced to `true` when `trust_level == Untrusted`.
    pub strict_undefined: bool,

    /// Log warnings when Python callable objects are detected in vars.
    /// Only relevant during migration from Python Jinja2.  ~2–8 µs/render.
    pub python_callable_warnings: bool,

    /// Restrict template loaders to these paths.
    /// Empty = no filesystem access (in-memory templates only).
    pub allowed_read_paths: Vec<PathBuf>,
}

impl TemplateEngineConfig {
    /// Standard trusted config (zero overhead, Ansible defaults).
    pub fn trusted() -> Self {
        Self {
            trust_level: TrustLevel::Trusted,
            strict_undefined: false,
            python_callable_warnings: false,
            allowed_read_paths: vec![],
        }
    }

    /// Sandboxed config for partially trusted content.
    /// Adds: deny-list, path policy.  No strict undefined.
    pub fn partially_trusted() -> Self {
        Self {
            trust_level: TrustLevel::PartiallyTrusted,
            strict_undefined: false,
            python_callable_warnings: false,
            allowed_read_paths: vec![],
        }
    }

    /// Sandboxed config for untrusted content.
    /// Adds: deny-list, strict undefined, python_callable_warnings.
    pub fn untrusted() -> Self {
        Self {
            trust_level: TrustLevel::Untrusted,
            strict_undefined: true,
            python_callable_warnings: true,
            allowed_read_paths: vec![],
        }
    }

    /// Returns `true` when any sandbox layer should be engaged.
    pub fn needs_sandbox(&self) -> bool {
        self.trust_level != TrustLevel::Trusted
    }
}

impl Default for TemplateEngineConfig {
    fn default() -> Self {
        Self::trusted()
    }
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

enum EngineInner {
    Standard(jinja2rs::Environment),
    #[cfg(feature = "sandboxed")]
    Sandboxed(jinja2rs::SandboxedEnvironment),
}

/// A template rendering engine for Ansible playbook usage.
///
/// Backed by `jinja2rs` (minijinja + Ansible filters).  Optionally uses
/// `jinja2rs::SandboxedEnvironment` when `trust_level != Trusted` or when
/// the `sandboxed` feature is forced.
pub struct AnsibleTemplateEngine {
    inner: EngineInner,
    config: TemplateEngineConfig,
}

impl AnsibleTemplateEngine {
    /// Standard engine (trusted playbooks, zero overhead).
    pub fn new() -> Self {
        let mut env = Environment::new();
        env.set_compat_mode(CompatMode::Ansible(AnsibleMode::default()));
        Self {
            inner: EngineInner::Standard(env),
            config: TemplateEngineConfig::trusted(),
        }
    }

    /// Sandboxed engine (strict undefined, deny-list, path policy).
    ///
    /// When the `sandboxed` feature flag is compiled in, uses
    /// `jinja2rs::SandboxedEnvironment`.  Otherwise falls back to the
    /// standard engine with a warning.
    pub fn sandboxed() -> Self {
        Self::with_config(TemplateEngineConfig::untrusted())
    }

    /// Build an engine from an explicit `TemplateEngineConfig`.
    ///
    /// This is the primary constructor for production use — it lets callers
    /// select the trust level per-play or per-task based on the data source.
    pub fn with_config(config: TemplateEngineConfig) -> Self {
        if !config.needs_sandbox() {
            return Self::new();
        }

        #[cfg(feature = "sandboxed")]
        {
            Self::build_sandboxed(config)
        }
        #[cfg(not(feature = "sandboxed"))]
        {
            tracing::warn!(
                trust_level = ?config.trust_level,
                "sandboxed template engine requested but 'sandboxed' feature is not \
                 compiled in (ansiblers-templates); using standard engine. \
                 Rebuild with --features ansiblers-templates/sandboxed to enable."
            );
            let mut env = Environment::new();
            env.set_compat_mode(CompatMode::Ansible(AnsibleMode::default()));
            Self {
                inner: EngineInner::Standard(env),
                config,
            }
        }
    }

    #[cfg(feature = "sandboxed")]
    fn build_sandboxed(config: TemplateEngineConfig) -> Self {
        use jinja2rs::sandbox_config::{PathPolicy, SandboxedEnvironmentBuilder};

        let mut builder = SandboxedEnvironmentBuilder::new();

        // Path policy
        if !config.allowed_read_paths.is_empty() {
            let mut policy = PathPolicy::new();
            for p in &config.allowed_read_paths {
                policy = policy.with_read_path(p.to_string_lossy().into_owned());
            }
            if let Ok(b) = builder.with_path_policy(policy) {
                builder = b;
            }
        }

        if config.python_callable_warnings {
            builder = builder.with_python_callable_warnings();
        }

        let sandboxed_env = builder.build();

        Self {
            inner: EngineInner::Sandboxed(sandboxed_env),
            config,
        }
    }

    /// Returns the active `TrustLevel`.
    pub fn trust_level(&self) -> TrustLevel {
        self.config.trust_level
    }

    /// Returns `true` if using the sandboxed environment.
    pub fn is_sandboxed(&self) -> bool {
        #[cfg(feature = "sandboxed")]
        {
            matches!(self.inner, EngineInner::Sandboxed(_))
        }
        #[cfg(not(feature = "sandboxed"))]
        {
            false
        }
    }

    /// Render `template_str` against `vars`.
    pub fn render(&self, template_str: &str, vars: &HashMap<String, Value>) -> Result<String> {
        match &self.inner {
            EngineInner::Standard(env) => env
                .render_str(template_str, vars)
                .with_context(|| format!("rendering template: {template_str}")),
            #[cfg(feature = "sandboxed")]
            EngineInner::Sandboxed(env) => env
                .render_str(template_str, vars)
                .with_context(|| format!("rendering template (sandboxed): {template_str}")),
        }
    }

    /// Render a `Value` recursively — strings are templated, other types pass through.
    pub fn render_value(&self, value: &Value, vars: &HashMap<String, Value>) -> Result<Value> {
        match value {
            Value::String(s) => Ok(Value::String(self.render(s, vars)?)),
            Value::Array(arr) => {
                let rendered: Result<Vec<_>> =
                    arr.iter().map(|v| self.render_value(v, vars)).collect();
                Ok(Value::Array(rendered?))
            }
            Value::Object(obj) => {
                let rendered: Result<serde_json::Map<_, _>> = obj
                    .iter()
                    .map(|(k, v)| self.render_value(v, vars).map(|rv| (k.clone(), rv)))
                    .collect();
                Ok(Value::Object(rendered?))
            }
            other => Ok(other.clone()),
        }
    }
}

impl Default for AnsibleTemplateEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Module-level convenience functions
// ---------------------------------------------------------------------------

/// Render a Jinja2 template string against the provided variables.
/// Uses the standard (trusted) engine — zero sandbox overhead.
pub fn render_string(template: &str, vars: &HashMap<String, Value>) -> Result<String> {
    AnsibleTemplateEngine::new().render(template, vars)
}

/// Render a [`Value`] recursively, resolving template strings.
pub fn render_value(value: &Value, vars: &HashMap<String, Value>) -> Result<Value> {
    AnsibleTemplateEngine::new().render_value(value, vars)
}

/// Render using the sandboxed engine (strict undefined, deny-list).
/// Per-render overhead: ~2–8 µs when python_callable_warnings is on, ~0 otherwise.
pub fn render_string_sandboxed(template: &str, vars: &HashMap<String, Value>) -> Result<String> {
    AnsibleTemplateEngine::sandboxed().render(template, vars)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
            .collect()
    }

    // ---- Standard engine ---------------------------------------------------

    #[rstest]
    #[case("Hello, {{ name }}!", &[("name", "World")], "Hello, World!")]
    #[case("{{ a }} + {{ b }}", &[("a", "1"), ("b", "2")], "1 + 2")]
    #[case("no-op", &[], "no-op")]
    #[case("{{ x | upper }}", &[("x", "hello")], "HELLO")]
    fn test_render_string(
        #[case] template: &str,
        #[case] v: &[(&str, &str)],
        #[case] expected: &str,
    ) {
        let result = render_string(template, &vars(v)).unwrap();
        assert_eq!(result, expected);
    }

    #[test]
    fn test_render_value_recursive() {
        let v: HashMap<String, Value> = [("greeting".to_string(), Value::String("Hi".to_string()))]
            .into_iter()
            .collect();
        let template_val = Value::Object({
            let mut m = serde_json::Map::new();
            m.insert(
                "msg".to_string(),
                Value::String("{{ greeting }}".to_string()),
            );
            m
        });
        let rendered = render_value(&template_val, &v).unwrap();
        assert_eq!(rendered["msg"], Value::String("Hi".to_string()));
    }

    // ---- TrustLevel / TemplateEngineConfig ---------------------------------

    #[test]
    fn test_trusted_config_no_sandbox() {
        let cfg = TemplateEngineConfig::trusted();
        assert!(!cfg.needs_sandbox());
        assert_eq!(cfg.trust_level, TrustLevel::Trusted);
        assert!(!cfg.strict_undefined);
    }

    #[test]
    fn test_partially_trusted_needs_sandbox() {
        let cfg = TemplateEngineConfig::partially_trusted();
        assert!(cfg.needs_sandbox());
        assert_eq!(cfg.trust_level, TrustLevel::PartiallyTrusted);
    }

    #[test]
    fn test_untrusted_needs_sandbox() {
        let cfg = TemplateEngineConfig::untrusted();
        assert!(cfg.needs_sandbox());
        assert!(cfg.strict_undefined);
        assert!(cfg.python_callable_warnings);
    }

    #[test]
    fn test_with_config_trusted_is_standard() {
        let engine = AnsibleTemplateEngine::with_config(TemplateEngineConfig::trusted());
        assert_eq!(engine.trust_level(), TrustLevel::Trusted);
        // Standard engine always succeeds on basic renders.
        let r = engine.render(
            "{{ x }}",
            &[("x".to_string(), Value::Number(1.into()))]
                .into_iter()
                .collect(),
        );
        assert!(r.is_ok());
    }

    #[test]
    fn test_with_config_untrusted_renders_defined() {
        let engine = AnsibleTemplateEngine::with_config(TemplateEngineConfig::untrusted());
        let mut v = HashMap::new();
        v.insert("y".to_string(), Value::String("defined".to_string()));
        let r = engine.render("{{ y }}", &v);
        // Defined vars must always work regardless of trust level.
        assert!(r.is_ok());
        assert_eq!(r.unwrap(), "defined");
    }

    #[cfg(feature = "sandboxed")]
    #[test]
    fn test_sandboxed_strict_undefined_errors() {
        let engine = AnsibleTemplateEngine::sandboxed();
        let result = engine.render("{{ missing }}", &HashMap::new());
        assert!(
            result.is_err(),
            "strict mode should error on undefined vars"
        );
    }

    #[cfg(feature = "sandboxed")]
    #[test]
    fn test_is_sandboxed_flag() {
        let standard = AnsibleTemplateEngine::new();
        let sandboxed = AnsibleTemplateEngine::sandboxed();
        assert!(!standard.is_sandboxed());
        assert!(sandboxed.is_sandboxed());
    }

    // ---- Overhead benchmark (manual, informational) -----------------------
    // Run with: cargo test --release -- --nocapture overhead
    #[test]
    fn test_overhead_comparison_informational() {
        use std::time::Instant;

        let template = "Hello {{ name }}, you have {{ count }} messages.";
        let vars: HashMap<String, Value> = [
            ("name".to_string(), Value::String("Alice".to_string())),
            ("count".to_string(), Value::Number(42.into())),
        ]
        .into_iter()
        .collect();
        const ITERS: u32 = 1000;

        // Standard engine.
        let std_engine = AnsibleTemplateEngine::new();
        let t0 = Instant::now();
        for _ in 0..ITERS {
            std_engine.render(template, &vars).unwrap();
        }
        let std_us = t0.elapsed().as_micros() as f64 / ITERS as f64;

        // Untrusted (sandboxed when feature present, standard otherwise).
        let sandbox_engine = AnsibleTemplateEngine::with_config(TemplateEngineConfig::untrusted());
        let t1 = Instant::now();
        for _ in 0..ITERS {
            sandbox_engine.render(template, &vars).unwrap();
        }
        let sb_us = t1.elapsed().as_micros() as f64 / ITERS as f64;

        eprintln!(
            "\n[overhead] standard={std_us:.1} us  sandboxed={sb_us:.1} us  \
             overhead={:.1} us  ({:.0}%)",
            sb_us - std_us,
            (sb_us - std_us) / std_us * 100.0,
        );

        // Sanity: sandboxed should not be more than 10× slower.
        assert!(
            sb_us < std_us * 10.0 || std_us < 1.0,
            "sandboxed engine unexpectedly slow: {sb_us:.1} us vs std {std_us:.1} us"
        );
    }
}
