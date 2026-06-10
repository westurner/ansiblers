//! Pluggable connection providers for task dispatch.
//!
//! A [`ConnectionProvider`] abstracts the mechanism used to run a module on a
//! target host.  The executor selects a provider per host based on the play's
//! `connection:` field (defaulting to `local`).
//!
//! ## Built-in providers
//!
//! | Provider | `connection:` value | Description |
//! |----------|---------------------|-------------|
//! | [`LocalConnectionProvider`] | `"local"` | In-process execution (default) |
//! | [`SshConnectionProvider`]  | `"ssh"` | Execute via `ssh` subprocess |
//!
//! ## Custom providers
//!
//! Implement [`ConnectionProvider`] and register via
//! [`ConnectionRegistry::register`]:
//!
//! ```rust
//! use ansiblers_executor::connection::{ConnectionProvider, ConnectionRegistry, ConnectionContext};
//! use ansiblers_core::{ExecutionContext, TaskResult};
//! use ansiblers_modules::ModuleArgs;
//! use anyhow::Result;
//!
//! struct MyCustomProvider;
//!
//! impl ConnectionProvider for MyCustomProvider {
//!     fn name(&self) -> &str { "my_custom" }
//!
//!     fn invoke_module(
//!         &self,
//!         module: &str,
//!         args: &ModuleArgs,
//!         host: &str,
//!         conn_ctx: &ConnectionContext,
//!         exec_ctx: &mut ExecutionContext,
//!     ) -> Result<TaskResult> {
//!         // custom dispatch logic
//!         todo!()
//!     }
//! }
//!
//! let mut registry = ConnectionRegistry::new();
//! registry.register(std::sync::Arc::new(MyCustomProvider));
//! ```

use std::collections::HashMap;
use std::process::Command;
use std::sync::Arc;

use ansiblers_core::{ExecutionContext, TaskResult, TaskStatus};
use ansiblers_modules::{ModuleArgs, ModuleRegistry};
use anyhow::{anyhow, Context, Result};
use tracing::debug;

// ---------------------------------------------------------------------------
// ConnectionContext — per-host connection parameters
// ---------------------------------------------------------------------------

/// Parameters for connecting to a specific host.
///
/// These mirror the Ansible connection variables (`ansible_host`,
/// `ansible_port`, `ansible_user`, `ansible_ssh_private_key_file`, etc.)
/// and are resolved from the inventory / variable stack before dispatch.
#[derive(Debug, Clone)]
pub struct ConnectionContext {
    /// The connection type to use (e.g. `"local"`, `"ssh"`).
    pub connection_type: ConnectionType,
    /// Resolved hostname or IP address used for the actual connection.
    pub ansible_host: String,
    /// SSH port (default 22).
    pub ansible_port: u16,
    /// Remote user to connect as.
    pub ansible_user: Option<String>,
    /// Path to the SSH private key file.
    pub ansible_ssh_private_key_file: Option<String>,
    /// Extra SSH arguments (split on whitespace).
    pub ansible_ssh_extra_args: Option<String>,
    /// Become / privilege escalation enabled.
    pub become_enabled: bool,
    /// Become method (`sudo`, `su`, …).
    pub become_method: String,
    /// Become user.
    pub become_user: String,
}

impl ConnectionContext {
    /// Create a `local` connection context (controller-side execution).
    pub fn local(host: &str) -> Self {
        Self {
            connection_type: ConnectionType::Local,
            ansible_host: host.to_string(),
            ansible_port: 22,
            ansible_user: None,
            ansible_ssh_private_key_file: None,
            ansible_ssh_extra_args: None,
            become_enabled: false,
            become_method: "sudo".to_string(),
            become_user: "root".to_string(),
        }
    }

    /// Create an SSH connection context.
    pub fn ssh(host: &str) -> Self {
        Self {
            connection_type: ConnectionType::Ssh,
            ansible_host: host.to_string(),
            ansible_port: 22,
            ansible_user: None,
            ansible_ssh_private_key_file: None,
            ansible_ssh_extra_args: None,
            become_enabled: false,
            become_method: "sudo".to_string(),
            become_user: "root".to_string(),
        }
    }

    /// Build a [`ConnectionContext`] from resolved Ansible variables for `host`.
    pub fn from_vars(host: &str, vars: &HashMap<String, ansiblers_core::Value>) -> Self {
        let get_str = |key: &str| -> Option<String> {
            vars.get(key).and_then(|v| v.as_str()).map(str::to_string)
        };
        let get_bool = |key: &str| -> bool {
            vars.get(key)
                .map(|v| matches!(v, ansiblers_core::Value::Bool(true)))
                .unwrap_or(false)
        };

        let ansible_host = get_str("ansible_host").unwrap_or_else(|| host.to_string());
        let connection_str = get_str("ansible_connection").unwrap_or_else(|| "local".to_string());
        let connection_type = ConnectionType::from_str(&connection_str);
        let ansible_port = vars
            .get("ansible_port")
            .and_then(|v| match v {
                ansiblers_core::Value::Number(n) => n.as_u64().map(|n| n as u16),
                ansiblers_core::Value::String(s) => s.parse().ok(),
                _ => None,
            })
            .unwrap_or(22);

        Self {
            connection_type,
            ansible_host,
            ansible_port,
            ansible_user: get_str("ansible_user"),
            ansible_ssh_private_key_file: get_str("ansible_ssh_private_key_file"),
            ansible_ssh_extra_args: get_str("ansible_ssh_extra_args"),
            become_enabled: get_bool("ansible_become"),
            become_method: get_str("ansible_become_method").unwrap_or_else(|| "sudo".to_string()),
            become_user: get_str("ansible_become_user").unwrap_or_else(|| "root".to_string()),
        }
    }
}

/// Connection plugin type.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConnectionType {
    /// Run tasks directly on the controller (no network I/O).
    #[default]
    Local,
    /// Connect via SSH subprocess.
    Ssh,
    /// Smart connection: SSH for remote hosts, local for `localhost`/`127.*`.
    Smart,
    /// Placeholder for future WebRTC/QUIC zero-trust transport.
    WebRtc,
    /// Any other connection plugin name.
    Other(String),
}

impl ConnectionType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "local" => Self::Local,
            "ssh" => Self::Ssh,
            "smart" => Self::Smart,
            "webrtc" | "webrtc_pq" => Self::WebRtc,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Local => "local",
            Self::Ssh => "ssh",
            Self::Smart => "smart",
            Self::WebRtc => "webrtc",
            Self::Other(s) => s.as_str(),
        }
    }
}

// ---------------------------------------------------------------------------
// ConnectionProvider trait
// ---------------------------------------------------------------------------

/// Abstraction over the mechanism used to run a module on a target host.
///
/// Implementors handle the transport and serialisation needed to execute a
/// module on a given host, then return a [`TaskResult`].
pub trait ConnectionProvider: Send + Sync {
    /// Unique name for this connection type (matches `ansible_connection`).
    fn name(&self) -> &str;

    /// Execute `module` with `args` on `host`.
    ///
    /// `conn_ctx` holds the resolved connection parameters for this host.
    /// `exec_ctx` provides access to facts and registered variables.
    fn invoke_module(
        &self,
        module: &str,
        args: &ModuleArgs,
        host: &str,
        conn_ctx: &ConnectionContext,
        exec_ctx: &mut ExecutionContext,
    ) -> Result<TaskResult>;
}

// ---------------------------------------------------------------------------
// LocalConnectionProvider — in-process module invocation (current behaviour)
// ---------------------------------------------------------------------------

/// Executes modules in-process on the controller node.
///
/// This is the default provider for `connection: local` plays and for
/// `localhost` targets.  It delegates directly to the [`ModuleRegistry`],
/// making it identical to the pre-Phase-6 behaviour.
pub struct LocalConnectionProvider {
    registry: Arc<ModuleRegistry>,
}

impl LocalConnectionProvider {
    pub fn new(registry: Arc<ModuleRegistry>) -> Self {
        Self { registry }
    }
}

impl ConnectionProvider for LocalConnectionProvider {
    fn name(&self) -> &str {
        "local"
    }

    fn invoke_module(
        &self,
        module: &str,
        args: &ModuleArgs,
        host: &str,
        _conn_ctx: &ConnectionContext,
        exec_ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        debug!(provider = "local", module, host, "invoke_module");
        self.registry.invoke(module, args, host, exec_ctx)
    }
}

// ---------------------------------------------------------------------------
// SshConnectionProvider — module invocation over SSH
// ---------------------------------------------------------------------------

/// Executes modules on a remote host via an `ssh` subprocess.
///
/// **Phase 6 implementation notes**:
/// - Transfers a pre-built module payload (JSON args) via SSH stdin.
/// - Runs the module binary on the remote side and collects the JSON result.
/// - `russh` / native SSH integration is deferred; this implementation
///   shells out to the system `ssh` binary for broad compatibility.
///
/// # Become / privilege escalation
///
/// When `conn_ctx.become_enabled` is `true`, the remote command is wrapped
/// in `sudo -u <become_user> -- <cmd>`.
pub struct SshConnectionProvider {
    /// Used to serialise module args and deserialise the remote result.
    registry: Arc<ModuleRegistry>,
}

impl SshConnectionProvider {
    pub fn new(registry: Arc<ModuleRegistry>) -> Self {
        Self { registry }
    }

    /// Build the SSH command vector for `conn_ctx`.
    fn build_ssh_args(&self, conn_ctx: &ConnectionContext, remote_cmd: &str) -> Vec<String> {
        let mut args: Vec<String> = vec!["ssh".to_string()];

        args.push("-o".to_string());
        args.push("BatchMode=yes".to_string());
        args.push("-o".to_string());
        args.push("StrictHostKeyChecking=accept-new".to_string());
        args.push("-p".to_string());
        args.push(conn_ctx.ansible_port.to_string());

        if let Some(key) = &conn_ctx.ansible_ssh_private_key_file {
            args.push("-i".to_string());
            args.push(key.clone());
        }

        if let Some(extra) = &conn_ctx.ansible_ssh_extra_args {
            for part in extra.split_whitespace() {
                args.push(part.to_string());
            }
        }

        let target = if let Some(user) = &conn_ctx.ansible_user {
            format!("{}@{}", user, conn_ctx.ansible_host)
        } else {
            conn_ctx.ansible_host.clone()
        };
        args.push(target);

        // Wrap with become if requested.
        if conn_ctx.become_enabled {
            let method = conn_ctx.become_method.as_str();
            let user = conn_ctx.become_user.as_str();
            match method {
                "su" => {
                    args.push(format!("su - {} -c '{}'", user, remote_cmd));
                }
                _ => {
                    // Default: sudo
                    args.push(format!("sudo -u {} -- {}", user, remote_cmd));
                }
            }
        } else {
            args.push(remote_cmd.to_string());
        }

        args
    }
}

impl ConnectionProvider for SshConnectionProvider {
    fn name(&self) -> &str {
        "ssh"
    }

    fn invoke_module(
        &self,
        module: &str,
        args: &ModuleArgs,
        host: &str,
        conn_ctx: &ConnectionContext,
        _exec_ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        debug!(
            provider = "ssh",
            module,
            host = conn_ctx.ansible_host,
            "invoke_module"
        );

        // Serialise arguments to JSON so they can be passed to a remote runner.
        let args_json =
            serde_json::to_string(&args.args).context("failed to serialise module args to JSON")?;

        // Remote command: `ransible-module-runner <module> '<json>'`
        // This requires the ransible module runner to be on the remote PATH.
        // For Phase 6, fall back gracefully with a descriptive error when the
        // remote runner is not available.
        let remote_cmd = format!(
            "ransible-module-runner {} {}",
            module,
            shell_escape(&args_json),
        );

        let ssh_args = self.build_ssh_args(conn_ctx, &remote_cmd);
        debug!(cmd = ?ssh_args, "running SSH module");

        let output = Command::new(&ssh_args[0])
            .args(&ssh_args[1..])
            .output()
            .with_context(|| format!("failed to spawn ssh for host {}", host))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let mut r = TaskResult::failed(
                host,
                format!(
                    "ssh module failed\nstdout: {}\nstderr: {}",
                    String::from_utf8_lossy(&output.stdout),
                    stderr,
                ),
            );
            r.task_name = args.task_name.clone();
            return Ok(r);
        }

        // Parse the JSON result returned by the remote runner.
        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: TaskResult = serde_json::from_str(stdout.trim()).unwrap_or_else(|_| {
            // If parsing fails, treat the raw stdout as success output.
            let mut r = TaskResult::ok(host);
            r.stdout = stdout.into_owned();
            r.task_name = args.task_name.clone();
            r
        });

        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// ConnectionRegistry — maps connection type names → providers
// ---------------------------------------------------------------------------

/// Registry mapping connection type names to [`ConnectionProvider`] implementations.
///
/// The executor uses this to look up the correct provider for each host based
/// on the resolved `ansible_connection` variable.
pub struct ConnectionRegistry {
    providers: HashMap<String, Arc<dyn ConnectionProvider>>,
}

impl ConnectionRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    /// Create a registry pre-populated with `local` and `ssh` providers.
    pub fn with_defaults(registry: Arc<ModuleRegistry>) -> Self {
        let mut r = Self::new();
        r.register(Arc::new(LocalConnectionProvider::new(Arc::clone(
            &registry,
        ))));
        r.register(Arc::new(SshConnectionProvider::new(Arc::clone(&registry))));
        r
    }

    /// Register a connection provider.  Overwrites any existing provider with
    /// the same [`ConnectionProvider::name`].
    pub fn register(&mut self, provider: Arc<dyn ConnectionProvider>) {
        self.providers.insert(provider.name().to_string(), provider);
    }

    /// Look up the provider for a connection type name.
    ///
    /// Falls back to `local` when the requested type is not registered.
    pub fn get(&self, connection_type: &str) -> Option<&Arc<dyn ConnectionProvider>> {
        self.providers
            .get(connection_type)
            .or_else(|| self.providers.get("local"))
    }

    /// Invoke a module using the appropriate provider for `conn_ctx`.
    pub fn invoke(
        &self,
        module: &str,
        args: &ModuleArgs,
        host: &str,
        conn_ctx: &ConnectionContext,
        exec_ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let type_name = conn_ctx.connection_type.as_str();
        let provider = self
            .get(type_name)
            .ok_or_else(|| anyhow!("no connection provider registered for '{}'", type_name))?;
        provider.invoke_module(module, args, host, conn_ctx, exec_ctx)
    }
}

impl Default for ConnectionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Minimally escape a string for use as a single shell argument.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[test]
    fn connection_type_roundtrip() {
        for s in &["local", "ssh", "smart", "webrtc"] {
            let ct = ConnectionType::from_str(s);
            assert_eq!(ct.as_str(), *s);
        }
    }

    #[test]
    fn connection_type_other() {
        let ct = ConnectionType::from_str("docker");
        assert_eq!(ct.as_str(), "docker");
        assert!(matches!(ct, ConnectionType::Other(_)));
    }

    #[rstest]
    #[case("local", ConnectionType::Local)]
    #[case("ssh", ConnectionType::Ssh)]
    #[case("smart", ConnectionType::Smart)]
    fn connection_type_from_str(#[case] input: &str, #[case] expected: ConnectionType) {
        assert_eq!(ConnectionType::from_str(input), expected);
    }

    #[test]
    fn connection_context_local() {
        let ctx = ConnectionContext::local("localhost");
        assert_eq!(ctx.connection_type, ConnectionType::Local);
        assert_eq!(ctx.ansible_host, "localhost");
        assert_eq!(ctx.ansible_port, 22);
        assert!(!ctx.become_enabled);
    }

    #[test]
    fn connection_context_ssh() {
        let ctx = ConnectionContext::ssh("192.168.1.10");
        assert_eq!(ctx.connection_type, ConnectionType::Ssh);
        assert_eq!(ctx.ansible_host, "192.168.1.10");
    }

    #[test]
    fn connection_context_from_vars() {
        let mut vars = HashMap::new();
        vars.insert(
            "ansible_connection".to_string(),
            ansiblers_core::Value::String("ssh".to_string()),
        );
        vars.insert(
            "ansible_host".to_string(),
            ansiblers_core::Value::String("10.0.0.1".to_string()),
        );
        vars.insert(
            "ansible_port".to_string(),
            ansiblers_core::Value::Number(serde_json::Number::from(2222_u16)),
        );
        vars.insert(
            "ansible_user".to_string(),
            ansiblers_core::Value::String("deploy".to_string()),
        );
        vars.insert(
            "ansible_become".to_string(),
            ansiblers_core::Value::Bool(true),
        );

        let ctx = ConnectionContext::from_vars("host1", &vars);
        assert_eq!(ctx.connection_type, ConnectionType::Ssh);
        assert_eq!(ctx.ansible_host, "10.0.0.1");
        assert_eq!(ctx.ansible_port, 2222);
        assert_eq!(ctx.ansible_user.as_deref(), Some("deploy"));
        assert!(ctx.become_enabled);
    }

    #[test]
    fn connection_registry_with_defaults() {
        let module_registry = Arc::new(ModuleRegistry::with_defaults());
        let conn_registry = ConnectionRegistry::with_defaults(module_registry);
        assert!(conn_registry.get("local").is_some());
        assert!(conn_registry.get("ssh").is_some());
        // Falls back to local for unknown type.
        assert!(conn_registry.get("unknown").is_some());
    }

    #[test]
    fn shell_escape_basic() {
        assert_eq!(shell_escape("hello"), "'hello'");
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
        assert_eq!(shell_escape(r#"{"key":"val"}"#), r#"'{"key":"val"}'"#);
    }

    #[test]
    fn ssh_provider_builds_args() {
        let registry = Arc::new(ModuleRegistry::with_defaults());
        let provider = SshConnectionProvider::new(registry);
        let mut ctx = ConnectionContext::ssh("10.0.0.1");
        ctx.ansible_user = Some("ubuntu".to_string());
        ctx.ansible_port = 2222;

        let args = provider.build_ssh_args(&ctx, "echo hi");
        assert!(args.contains(&"ubuntu@10.0.0.1".to_string()));
        assert!(args.contains(&"2222".to_string()));
    }

    #[test]
    fn ssh_provider_with_become() {
        let registry = Arc::new(ModuleRegistry::with_defaults());
        let provider = SshConnectionProvider::new(registry);
        let mut ctx = ConnectionContext::ssh("10.0.0.2");
        ctx.become_enabled = true;
        ctx.become_user = "root".to_string();

        let args = provider.build_ssh_args(&ctx, "whoami");
        let last = args.last().unwrap();
        assert!(last.contains("sudo"), "expected sudo in: {last}");
    }
}
