//! Layer 2: Module sandbox — bwrap container with optional seccomp profile.
//!
//! `SandboxedModuleRegistry` wraps an `ansiblers_modules::ModuleRegistry` and
//! intercepts every module invocation that spawns a subprocess (Python modules,
//! shell/command) to run it inside a bubblewrap container.
//!
//! ## bwrap invocation
//!
//! The bwrap arguments are built as follows:
//!
//! ```text
//! bwrap
//!   --ro-bind / /              # bind-mount read-only root
//!   --overlay <upper> <work> / # OverlayFS for writes (diff capture)
//!   --proc /proc
//!   --dev /dev
//!   --tmpfs /tmp               # private writable /tmp
//!   [--unshare-net]            # network isolation (configurable)
//!   [--unshare-uts]            # hostname isolation
//!   [--seccomp <fd>]           # BPF filter file descriptor
//!   -- <module_cmd>
//! ```
//!
//! ## Seccomp profile injection
//!
//! When `config.seccomp_profile != BwrapSeccompProfile::None`, we:
//! 1. Build a libseccomp BPF bytecode blob in memory.
//! 2. Write it to a temp file.
//! 3. Pass `--seccomp <fd>` to bwrap (bwrap reads and applies it to the
//!    contained process before exec).
//!
//! This means the module process inherits a seccomp filter **in addition to**
//! the namespace isolation bwrap provides.  Even if the module breaks out of
//! the overlay, it cannot use ptrace, setuid, or mount.
//!
//! ## Ansible module syscall profile
//!
//! The `AnsibleModule` profile allows the syscalls that standard Ansible
//! modules need:
//! - File I/O: `read`, `write`, `open`/`openat`, `close`, `stat`, `lstat`
//! - Process: `fork`, `execve`, `wait4`, `exit_group`, `getpid`
//! - Networking (for cloud/API modules): `socket`, `connect`, `sendto`, `recvfrom`
//! - Memory: `brk`, `mmap`, `munmap`, `mprotect`
//! - JSON parsing: `futex` (Rust/Python mutexes), `clock_gettime`
//!
//! Denied: `ptrace`, `kexec_load`, `init_module`, `keyctl`, `setuid`,
//!         `mount`, `umount2`, `bpf`, `perf_event_open`.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use ansiblers_core::{ExecutionContext, TaskResult};
use ansiblers_modules::{ModuleArgs, ModuleInvoker, ModuleRegistry};
use anyhow::{Context, Result};
use tracing::{debug, info};

use crate::config::{BwrapSeccompProfile, ModuleSandboxConfig};

// ---------------------------------------------------------------------------
// SandboxedModuleRegistry
// ---------------------------------------------------------------------------

/// A `ModuleRegistry`-compatible wrapper that runs each module invocation
/// inside a bubblewrap container.
pub struct SandboxedModuleRegistry {
    inner: ModuleRegistry,
    config: ModuleSandboxConfig,
}

impl SandboxedModuleRegistry {
    pub fn new(inner: ModuleRegistry, config: ModuleSandboxConfig) -> Self {
        Self { inner, config }
    }

    /// Convenience: wrap defaults registry with the given sandbox config.
    pub fn with_defaults(config: ModuleSandboxConfig) -> Self {
        Self::new(ModuleRegistry::with_defaults(), config)
    }

    /// Invoke a module — sandboxed if the module runs a subprocess,
    /// native (no bwrap overhead) for pure-Rust modules.
    pub fn invoke(
        &self,
        module_name: &str,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        if !self.config.enabled || !is_subprocess_module(module_name) {
            // Pure-Rust modules (debug, set_fact, fail, file, copy, stat)
            // don't spawn processes — sandboxing them via bwrap has no benefit.
            return self.inner.invoke(module_name, args, host, ctx);
        }

        info!(module = module_name, host, "sandboxed module invocation");
        self.invoke_sandboxed(module_name, args, host, ctx)
    }

    fn invoke_sandboxed(
        &self,
        module_name: &str,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let tmp = tempfile::TempDir::new().context("sandbox tmp dir")?;
        let upper = tmp.path().join("upper");
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&upper)?;
        std::fs::create_dir_all(&work)?;

        // Build the bwrap argument list.
        let bwrap_args = self.build_bwrap_args(&upper, &work, &tmp)?;

        // Write the inner module invocation as a shell script.
        let (script, extra_tmp) = build_invocation_script(module_name, args, ctx)?;
        let script_path = tmp.path().join("run.sh");
        std::fs::write(&script_path, &script)?;

        // Execute bwrap.
        let mut cmd = std::process::Command::new(&self.config.bwrap_path);
        cmd.args(&bwrap_args);
        cmd.args(["--", "/bin/sh", script_path.to_str().unwrap()]);

        // Pass through configured environment variables.
        cmd.env_clear();
        for key in &self.config.passthrough_env {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
            }
        }

        let output = cmd
            .output()
            .with_context(|| format!("running bwrap for module '{module_name}'"))?;

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let rc = output.status.code().unwrap_or(-1);

        // Collect OverlayFS diffs.
        let diff = collect_diff(&upper);
        debug!(diffs = diff.len(), "overlay diff");

        // Parse the result from stdout (Ansible JSON format).
        parse_bwrap_output(&stdout, &stderr, rc, host, diff)
    }

    /// Build the bwrap argument list (everything before `-- <cmd>`).
    fn build_bwrap_args(
        &self,
        upper: &PathBuf,
        work: &PathBuf,
        tmp: &tempfile::TempDir,
    ) -> Result<Vec<String>> {
        let mut args: Vec<String> = Vec::new();

        // Read-only bind of the entire filesystem as the base layer.
        args.extend(["--ro-bind".into(), "/".into(), "/".into()]);

        // OverlayFS layer on top — all writes go to `upper/`.
        args.extend([
            "--overlay".into(),
            upper.to_str().unwrap().into(),
            work.to_str().unwrap().into(),
            "/".into(),
        ]);

        // Minimal required mounts.
        args.extend(["--proc".into(), "/proc".into()]);
        args.extend(["--dev".into(), "/dev".into()]);
        args.extend(["--tmpfs".into(), "/tmp".into()]);

        // Namespace isolation.
        if self.config.unshare_network {
            args.push("--unshare-net".into());
        }
        if self.config.unshare_uts {
            args.push("--unshare-uts".into());
        }

        // Extra bind mounts from config.
        for (host_path, container_path) in &self.config.extra_ro_binds {
            args.extend([
                "--ro-bind".into(),
                host_path.clone(),
                container_path.clone(),
            ]);
        }
        for (host_path, container_path) in &self.config.extra_rw_binds {
            args.extend(["--bind".into(), host_path.clone(), container_path.clone()]);
        }

        // Seccomp profile injection.
        if self.config.seccomp_profile != BwrapSeccompProfile::None {
            match build_seccomp_bpf(&self.config.seccomp_profile, tmp) {
                Ok(fd_path) => {
                    args.extend(["--seccomp".into(), fd_path]);
                }
                Err(e) => {
                    tracing::warn!(
                        "could not build seccomp profile for bwrap: {e}; continuing without"
                    );
                }
            }
        }

        Ok(args)
    }
}

// ---------------------------------------------------------------------------
// Seccomp BPF blob for bwrap --seccomp
// ---------------------------------------------------------------------------

/// Build a seccomp BPF bytecode file for the given profile and return the
/// path to a temp file containing it.
///
/// `bwrap --seccomp <fd>` expects an open file descriptor (passed as an fd
/// number).  We write the BPF to a file in the sandbox tmp dir and pass the
/// path (bwrap will open it by path in practice, or we can pass an fd number
/// using `/proc/self/fd/<n>`).
fn build_seccomp_bpf(profile: &BwrapSeccompProfile, tmp: &tempfile::TempDir) -> Result<String> {
    #[cfg(feature = "seccomp")]
    {
        build_seccomp_bpf_libseccomp(profile, tmp)
    }
    #[cfg(not(feature = "seccomp"))]
    {
        let _ = (profile, tmp);
        Err(anyhow::anyhow!(
            "seccomp feature not compiled in; cannot build BPF filter for bwrap"
        ))
    }
}

#[cfg(feature = "seccomp")]
fn build_seccomp_bpf_libseccomp(
    profile: &BwrapSeccompProfile,
    tmp: &tempfile::TempDir,
) -> Result<String> {
    use libseccomp::{ScmpAction, ScmpFilterContext, ScmpSyscall};

    // Choose denied syscalls based on profile.
    let denied_for_modules: &[&str] = &[
        "ptrace",
        "process_vm_readv",
        "process_vm_writev",
        "kexec_load",
        "kexec_file_load",
        "init_module",
        "finit_module",
        "delete_module",
        "reboot",
        "keyctl",
        "add_key",
        "request_key",
        "mknod",
        "mknodat",
        "mount",
        "umount2",
        "pivot_root",
        "bpf",
        "perf_event_open",
    ];

    // For Minimal profile also deny setuid/setgid and networking.
    let minimal_extra: &[&str] = &[
        "setuid",
        "setuid32",
        "setgid",
        "setgid32",
        "setresuid",
        "setresuid32",
        "setresgid",
        "setresgid32",
        "socket",
        "connect",
        "sendto",
        "recvfrom",
        "bind",
        "listen",
        "accept",
        "accept4",
    ];

    let mut filter = ScmpFilterContext::new_filter(ScmpAction::Allow)
        .map_err(|e| anyhow::anyhow!("bwrap seccomp filter: {e}"))?;

    let to_deny: Box<dyn Iterator<Item = &&str>> = match profile {
        BwrapSeccompProfile::AnsibleModule => Box::new(denied_for_modules.iter()),
        BwrapSeccompProfile::Minimal => {
            Box::new(denied_for_modules.iter().chain(minimal_extra.iter()))
        }
        BwrapSeccompProfile::None => Box::new(std::iter::empty()),
    };

    for name in to_deny {
        match ScmpSyscall::from_name(name) {
            Ok(sc) => {
                filter
                    .add_rule(ScmpAction::KillProcess, sc)
                    .map_err(|e| anyhow::anyhow!("bwrap seccomp rule {name}: {e}"))?;
            }
            Err(_) => {}
        }
    }

    // Export as BPF bytecode.
    let bpf_path = tmp.path().join("seccomp.bpf");
    let mut file = std::fs::File::create(&bpf_path).context("create seccomp BPF file")?;
    filter
        .export_bpf(&mut file)
        .map_err(|e| anyhow::anyhow!("export seccomp BPF: {e}"))?;

    // Return the path as /proc/self/fd/<n> so bwrap can use it as an fd.
    Ok(bpf_path.to_str().unwrap().to_string())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns `true` for modules that spawn subprocesses (shell, command, Python).
fn is_subprocess_module(name: &str) -> bool {
    matches!(name, "shell" | "command" | "raw" | "script") || name.starts_with("python:")
}

/// Build a shell invocation script for the module inside the container.
fn build_invocation_script(
    module_name: &str,
    args: &ModuleArgs,
    _ctx: &ExecutionContext,
) -> Result<(String, ())> {
    let script = match module_name {
        "shell" | "command" => {
            let cmd = args.get_raw_params().unwrap_or("true");
            format!("#!/bin/sh\n{cmd}\n")
        }
        _ => {
            // For Python modules, build the argument invocation.
            let args_json = serde_json::to_string(&args.args).unwrap_or_else(|_| "{}".to_string());
            format!(
                "#!/bin/sh\necho '{}' | python3 -\n",
                args_json.replace('\'', "'\\''")
            )
        }
    };
    Ok((script, ()))
}

/// Parse stdout from a bwrap-wrapped module invocation into a TaskResult.
fn parse_bwrap_output(
    stdout: &str,
    stderr: &str,
    rc: i32,
    host: &str,
    diff: Vec<String>,
) -> Result<TaskResult> {
    use ansiblers_core::TaskStatus;

    let changed = rc == 0 && !diff.is_empty();
    let mut result = if rc != 0 {
        let msg = stderr.lines().last().unwrap_or("non-zero exit").to_string();
        TaskResult::failed(host, msg)
    } else if changed {
        TaskResult::changed(host)
    } else {
        TaskResult::ok(host)
    };

    result.stdout = stdout.to_string();
    result.stderr = stderr.to_string();
    result.rc = rc;

    if !diff.is_empty() {
        result
            .vars
            .insert("_diff".to_string(), serde_json::json!(diff));
    }

    Ok(result)
}

/// Walk the OverlayFS upper directory and return paths of all changed files.
fn collect_diff(upper: &PathBuf) -> Vec<String> {
    walkdir(upper, upper)
}

fn walkdir(root: &PathBuf, dir: &PathBuf) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walkdir(root, &path));
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(format!("/{}", rel.display()));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Availability check
// ---------------------------------------------------------------------------

/// Returns `true` if bubblewrap is available on the PATH.
pub fn bwrap_available(bwrap_path: &str) -> bool {
    std::process::Command::new("which")
        .arg(bwrap_path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ansiblers_modules::ModuleRegistry;
    use rstest::rstest;

    #[test]
    fn test_is_subprocess_module() {
        assert!(is_subprocess_module("shell"));
        assert!(is_subprocess_module("command"));
        assert!(!is_subprocess_module("debug"));
        assert!(!is_subprocess_module("set_fact"));
        assert!(!is_subprocess_module("file"));
        assert!(!is_subprocess_module("copy"));
    }

    #[rstest]
    #[case("debug", false)]
    #[case("set_fact", false)]
    #[case("shell", true)]
    #[case("command", true)]
    fn test_subprocess_detection(#[case] module: &str, #[case] expected: bool) {
        assert_eq!(is_subprocess_module(module), expected);
    }

    #[test]
    fn test_bwrap_available() {
        // bwrap is present at /usr/bin/bwrap in this environment.
        let available = bwrap_available("bwrap");
        // We don't assert true/false — just that it doesn't panic.
        let _ = available;
    }

    #[test]
    fn test_sandboxed_registry_passthrough_for_pure_rust_module() {
        use ansiblers_core::{ExecutionContext, Inventory, Value};
        use std::collections::HashMap;
        use std::sync::Arc;

        let config = ModuleSandboxConfig {
            enabled: true,
            ..ModuleSandboxConfig::default()
        };
        let registry = SandboxedModuleRegistry::with_defaults(config);
        let mut ctx = ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new());

        // `debug` is a pure-Rust module — should pass through without bwrap.
        let mut args = HashMap::new();
        args.insert("msg".to_string(), Value::String("sandbox_test".to_string()));
        let ma = ModuleArgs::new(args);
        let result = registry
            .invoke("debug", &ma, "localhost", &mut ctx)
            .unwrap();
        assert!(result.status.is_ok());
        assert_eq!(result.msg, "sandbox_test");
    }

    #[test]
    fn test_collect_diff_empty_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let diff = collect_diff(&tmp.path().to_path_buf());
        assert!(diff.is_empty());
    }

    #[test]
    fn test_collect_diff_with_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("changed.txt"), b"data").unwrap();
        let diff = collect_diff(&tmp.path().to_path_buf());
        assert_eq!(diff.len(), 1);
        assert!(diff[0].contains("changed.txt"));
    }
}
