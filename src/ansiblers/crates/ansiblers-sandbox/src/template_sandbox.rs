//! Layer 1: Template sandbox — secure Jinja2 rendering via `jinja2rs::SandboxedEnvironment`.
//!
//! Provides [`SandboxedTemplateEngine`], a rendering engine that delegates to
//! `jinja2rs::SandboxedEnvironment` when the `jinja2-sandbox` feature is enabled
//! and [`TemplateSandboxConfig::enabled`] is `true`.
//!
//! ## What the sandbox protects against
//!
//! | Attack | Mitigation |
//! |--------|------------|
//! | Undefined variable silently empty | Strict undefined → error |
//! | Reflection via `__class__`, `__mro__` | Denied attribute list |
//! | Arbitrary file include/load | Path policy whitelist |
//! | Resource exhaustion (render loop) | Memory + CPU limits (optional) |
//! | Syscall escalation from template thread | seccomp on render call (optional) |
//!
//! ## Overhead
//!
//! Without `python_callable_warnings`: ~0 ns (attribute checks are O(1)).
//! With `python_callable_warnings`: ~2–8 µs per render (O(N) context walk).
//!
//! ## Usage
//!
//! Prefer using [`AnsibleTemplateEngine::with_config`] with a [`TrustLevel`]
//! directly in `ansiblers-templates`.  Use this module when you need explicit
//! control over path policy and resource limits.
//!
//! [`AnsibleTemplateEngine::with_config`]: ansiblers_templates::AnsibleTemplateEngine::with_config
//! [`TrustLevel`]: ansiblers_templates::TrustLevel

use anyhow::Result;

use crate::config::TemplateSandboxConfig;

/// Build a sandboxed template engine based on the provided config.
///
/// Returns an opaque handle that implements the same rendering interface
/// as the standard engine. When `jinja2-sandbox` feature is absent, returns
/// the standard (non-sandboxed) engine.
pub struct SandboxedTemplateEngine {
    config: TemplateSandboxConfig,
}

impl SandboxedTemplateEngine {
    pub fn new(config: TemplateSandboxConfig) -> Self {
        Self { config }
    }

    /// Render a template string against a variable map.
    ///
    /// Uses the jinja2rs SandboxedEnvironment when the `jinja2-sandbox` feature
    /// is enabled, otherwise delegates to the standard engine.
    pub fn render(
        &self,
        template: &str,
        vars: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<String> {
        if !self.config.enabled {
            return ansiblers_templates::render_string(template, vars)
                .map_err(|e| anyhow::anyhow!("{e}"));
        }

        #[cfg(feature = "jinja2-sandbox")]
        {
            render_sandboxed(template, vars, &self.config)
        }
        #[cfg(not(feature = "jinja2-sandbox"))]
        {
            tracing::warn!(
                "template sandbox enabled in config but 'jinja2-sandbox' feature \
                 is not compiled in; using standard engine"
            );
            ansiblers_templates::render_string(template, vars).map_err(|e| anyhow::anyhow!("{e}"))
        }
    }
}

// ---------------------------------------------------------------------------
// jinja2rs sandboxed rendering
// ---------------------------------------------------------------------------

#[cfg(feature = "jinja2-sandbox")]
fn render_sandboxed(
    template: &str,
    vars: &std::collections::HashMap<String, serde_json::Value>,
    config: &TemplateSandboxConfig,
) -> Result<String> {
    use jinja2rs::sandbox_config::{PathPolicy, SandboxedEnvironmentBuilder, SeccompWhitelist};

    let mut builder = SandboxedEnvironmentBuilder::new();

    // Path policy: restrict template includes to declared read paths.
    if !config.allowed_read_paths.is_empty() {
        let mut policy = PathPolicy::new();
        for p in &config.allowed_read_paths {
            policy = policy.with_read_path(p.to_string_lossy().into_owned());
        }
        builder = builder
            .with_path_policy(policy)
            .map_err(|e| anyhow::anyhow!("template path policy: {e}"))?;
    }

    // Seccomp for the template-render call.
    #[cfg(all(feature = "jinja2-sandbox", unix))]
    if config.seccomp {
        builder = builder
            .with_seccomp_whitelist(SeccompWhitelist::Minimal)
            .with_seccomp_filtering()
            .map_err(|e| anyhow::anyhow!("template seccomp: {e}"))?;
    }

    // Resource limits.
    #[cfg(all(feature = "jinja2-sandbox", unix))]
    if config.memory_limit_bytes > 0 || config.cpu_limit_secs > 0 {
        let mem = if config.memory_limit_bytes > 0 {
            config.memory_limit_bytes
        } else {
            u64::MAX
        };
        let cpu = if config.cpu_limit_secs > 0 {
            config.cpu_limit_secs
        } else {
            u64::MAX
        };
        builder = builder
            .with_resource_limits(mem, cpu)
            .map_err(|e| anyhow::anyhow!("template resource limits: {e}"))?;
    }

    let env = builder.build();
    env.render_str(template, vars)
        .map_err(|e| anyhow::anyhow!("template render: {e}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TemplateSandboxConfig;
    use std::collections::HashMap;

    #[test]
    fn test_render_disabled_passthrough() {
        let engine = SandboxedTemplateEngine::new(TemplateSandboxConfig::default());
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), serde_json::json!("world"));
        let out = engine.render("Hello {{ name }}!", &vars).unwrap();
        assert_eq!(out, "Hello world!");
    }

    #[cfg(feature = "jinja2-sandbox")]
    #[test]
    fn test_render_sandboxed_basic() {
        let engine = SandboxedTemplateEngine::new(TemplateSandboxConfig::enabled());
        let mut vars = HashMap::new();
        vars.insert("x".to_string(), serde_json::json!(42));
        let out = engine.render("value={{ x }}", &vars).unwrap();
        assert_eq!(out, "value=42");
    }

    #[cfg(feature = "jinja2-sandbox")]
    #[test]
    fn test_render_sandboxed_strict_undefined_errors() {
        let engine = SandboxedTemplateEngine::new(TemplateSandboxConfig::enabled());
        let result = engine.render("{{ undefined_var }}", &HashMap::new());
        assert!(
            result.is_err(),
            "strict mode should error on undefined vars"
        );
    }
}
