//! Unified sandbox configuration — controls which layers are active.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Top-level sandbox configuration; compose which layers are active.
///
/// Each layer is independently opt-in.  Use [`SandboxConfig::full`] for maximum
/// isolation or one of the convenience constructors for common scenarios:
///
/// | Constructor | Layers active | Use case |
/// |-------------|--------------|----------|
/// | [`disabled`](Self::disabled) | none | Trusted environment (default) |
/// | [`template_only`](Self::template_only) | Layer 1 | CI, no root needed |
/// | [`module_isolation`](Self::module_isolation) | Layer 2 | Untrusted modules, no kernel seccomp |
/// | [`full`](Self::full) | 1 + 2 + 3 | Maximum isolation |
///
/// # Example
///
/// ```rust,no_run
/// use ansiblers_sandbox::config::SandboxConfig;
///
/// // Enable all three layers:
/// let cfg = SandboxConfig::full();
/// assert!(cfg.template.enabled && cfg.module.enabled && cfg.process.enabled);
///
/// // Just wrap modules in bwrap:
/// let cfg = SandboxConfig::module_isolation();
/// assert!(cfg.module.enabled);
/// assert!(!cfg.process.enabled);
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// Layer 1: restrict template rendering via jinja2rs SandboxedEnvironment.
    pub template: TemplateSandboxConfig,

    /// Layer 2: wrap each module invocation in bubblewrap.
    pub module: ModuleSandboxConfig,

    /// Layer 3: apply seccomp to the ansiblers coordinator process at startup.
    pub process: ProcessSandboxConfig,
}

impl SandboxConfig {
    /// All sandbox layers disabled (default, production-safe for trusted environments).
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Maximum isolation: all layers enabled.
    pub fn full() -> Self {
        Self {
            template: TemplateSandboxConfig::enabled(),
            module: ModuleSandboxConfig::enabled(),
            process: ProcessSandboxConfig::enabled(),
        }
    }

    /// Template-only sandbox (safe for CI, no root/kernel privileges needed).
    pub fn template_only() -> Self {
        Self {
            template: TemplateSandboxConfig::enabled(),
            ..Self::default()
        }
    }

    /// Module isolation via bwrap (no process seccomp, no template restrictions).
    pub fn module_isolation() -> Self {
        Self {
            module: ModuleSandboxConfig::enabled(),
            ..Self::default()
        }
    }
}

/// Controls which sandbox layers are considered "enabled" for display/reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxLayer {
    Template,
    Module,
    Process,
}

// ---------------------------------------------------------------------------
// Layer 1: Template sandbox
// ---------------------------------------------------------------------------

/// Configuration for jinja2rs SandboxedEnvironment (template rendering).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateSandboxConfig {
    /// Enable the sandboxed environment (strict undefined, denied attrs).
    pub enabled: bool,

    /// Filesystem paths templates may load includes/partials from.
    /// Empty = no filesystem access (templates must be pre-loaded in-memory).
    pub allowed_read_paths: Vec<PathBuf>,

    /// Whether to apply seccomp to the template-render call.
    /// Requires `jinja2rs` to be compiled with the `seccomp` feature.
    pub seccomp: bool,

    /// Memory limit for template rendering in bytes (0 = unlimited).
    pub memory_limit_bytes: u64,

    /// CPU time limit in seconds (0 = unlimited).
    pub cpu_limit_secs: u64,
}

impl TemplateSandboxConfig {
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }
}

impl Default for TemplateSandboxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_read_paths: Vec::new(),
            seccomp: false,
            memory_limit_bytes: 0,
            cpu_limit_secs: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Layer 2: Module sandbox
// ---------------------------------------------------------------------------

/// Configuration for per-module bubblewrap isolation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleSandboxConfig {
    /// Enable bwrap wrapping for module invocations.
    pub enabled: bool,

    /// Path to the bwrap binary. Defaults to `"bwrap"` (resolved via PATH).
    pub bwrap_path: String,

    /// Attach a seccomp BPF profile to the bwrap container.
    /// When `true`, a pre-built profile for Ansible modules is loaded.
    pub seccomp_profile: BwrapSeccompProfile,

    /// Unshare the network namespace (default: true).
    /// Set to `false` for modules that need network access (e.g. cloud modules).
    pub unshare_network: bool,

    /// Unshare the UTS namespace (hostname isolation).
    pub unshare_uts: bool,

    /// Additional read-only bind mounts inside the container.
    /// Format: `(host_path, container_path)`.
    pub extra_ro_binds: Vec<(String, String)>,

    /// Additional read-write bind mounts (e.g. /tmp for output files).
    pub extra_rw_binds: Vec<(String, String)>,

    /// Environment variables passed through to the sandboxed module.
    pub passthrough_env: Vec<String>,
}

/// Which seccomp profile to attach to bwrap containers.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum BwrapSeccompProfile {
    /// No seccomp filter (bwrap alone provides namespace isolation).
    #[default]
    None,
    /// Ansible-module profile: syscalls needed for common Ansible operations.
    AnsibleModule,
    /// Minimal profile: only syscalls for file I/O + JSON + exit.
    Minimal,
}

impl ModuleSandboxConfig {
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }
}

impl Default for ModuleSandboxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bwrap_path: "bwrap".to_string(),
            seccomp_profile: BwrapSeccompProfile::None,
            unshare_network: true,
            unshare_uts: true,
            extra_ro_binds: Vec::new(),
            extra_rw_binds: Vec::new(),
            passthrough_env: vec![
                "HOME".to_string(),
                "PATH".to_string(),
                "LANG".to_string(),
                "LC_ALL".to_string(),
                "PYTHONDONTWRITEBYTECODE".to_string(),
            ],
        }
    }
}

// ---------------------------------------------------------------------------
// Layer 3: Process seccomp
// ---------------------------------------------------------------------------

/// Configuration for process-level seccomp (applied to ansiblers itself).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessSandboxConfig {
    /// Apply seccomp to the ansiblers coordinator process.
    /// Must be called before any module execution begins.
    pub enabled: bool,

    /// Seccomp profile strictness.
    pub profile: ProcessSeccompLevel,
}

/// How aggressively to restrict the coordinator process.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProcessSeccompLevel {
    /// Reasonable defaults: deny ptrace, kexec, setuid, keyctl, etc.
    #[default]
    Coordinator,
    /// Strict: only syscalls for file I/O, process management, and networking.
    Strict,
}

impl ProcessSandboxConfig {
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }
}

impl Default for ProcessSandboxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            profile: ProcessSeccompLevel::Coordinator,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disabled_is_all_false() {
        let cfg = SandboxConfig::disabled();
        assert!(!cfg.template.enabled);
        assert!(!cfg.module.enabled);
        assert!(!cfg.process.enabled);
    }

    #[test]
    fn test_full_is_all_enabled() {
        let cfg = SandboxConfig::full();
        assert!(cfg.template.enabled);
        assert!(cfg.module.enabled);
        assert!(cfg.process.enabled);
    }

    #[test]
    fn test_template_only() {
        let cfg = SandboxConfig::template_only();
        assert!(cfg.template.enabled);
        assert!(!cfg.module.enabled);
        assert!(!cfg.process.enabled);
    }

    #[test]
    fn test_module_isolation() {
        let cfg = SandboxConfig::module_isolation();
        assert!(!cfg.template.enabled);
        assert!(cfg.module.enabled);
    }

    #[test]
    fn test_bwrap_seccomp_profile_default() {
        let cfg = ModuleSandboxConfig::default();
        assert_eq!(cfg.seccomp_profile, BwrapSeccompProfile::None);
        assert!(cfg.unshare_network);
    }
}
