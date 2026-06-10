//! Layer 3: Process-level seccomp — restrict syscalls in the ansiblers coordinator.
//!
//! ## Design rationale
//!
//! The ansiblers coordinator process does NOT need to:
//! - Call `ptrace` (debug other processes)
//! - Load kernel modules (`init_module`, `finit_module`)
//! - Reboot (`reboot`, `kexec_load`)
//! - Manipulate user IDs (`setuid`, `setgid`, `setresuid` — already running as
//!   the playbook user)
//! - Access kernel keyrings (`keyctl`, `add_key`, `request_key`)
//! - Create device files (`mknod`)
//! - Mount filesystems (`mount`, `umount2`) — bwrap does that in a child
//!
//! Applying seccomp to the coordinator before executing any modules provides a
//! last-resort containment: even if a module bypasses its bwrap container via
//! a shared-memory channel or library injection, it cannot escalate via the
//! coordinator's elevated file descriptors.
//!
//! ## Availability
//!
//! This module is compiled when the `seccomp` feature is enabled.
//! It falls back to a no-op when the feature is absent, so callers don't need
//! `#[cfg(feature = "seccomp")]` guards.

use anyhow::Result;

use crate::config::ProcessSeccompLevel;

/// Pre-built seccomp profiles for the ansiblers coordinator process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProcessSeccompProfile {
    /// Deny the most dangerous syscalls while allowing normal coordinator ops.
    ///
    /// Denied: `ptrace`, `process_vm_readv`, `process_vm_writev`,
    /// `kexec_load`, `kexec_file_load`, `init_module`, `finit_module`,
    /// `reboot`, `keyctl`, `add_key`, `request_key`,
    /// `setuid`, `setgid`, `setresuid`, `setresgid`,
    /// `mknod`, `mknodat`, `mount`, `umount2`.
    #[default]
    Coordinator,

    /// Stricter profile — only allow explicitly listed syscalls.
    ///
    /// Suitable for the coordinator when it only performs:
    /// file I/O, socket communication, spawning children, and futex.
    Strict,
}

impl From<ProcessSeccompLevel> for ProcessSeccompProfile {
    fn from(level: ProcessSeccompLevel) -> Self {
        match level {
            ProcessSeccompLevel::Coordinator => Self::Coordinator,
            ProcessSeccompLevel::Strict => Self::Strict,
        }
    }
}

/// Apply a seccomp filter to the **current process**.
///
/// Must be called **before** any module execution begins.
/// Once applied, the filter cannot be removed.
///
/// When the `seccomp` feature is not compiled in, this function is a no-op
/// that logs a warning.
pub fn apply_process_seccomp(profile: ProcessSeccompProfile) -> Result<()> {
    #[cfg(feature = "seccomp")]
    {
        apply_with_libseccomp(profile)
    }
    #[cfg(not(feature = "seccomp"))]
    {
        let _ = profile;
        tracing::warn!(
            "process seccomp requested but 'seccomp' feature is not compiled in; \
             rebuild with --features ansiblers-sandbox/seccomp to enable"
        );
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// libseccomp implementation
// ---------------------------------------------------------------------------

#[cfg(feature = "seccomp")]
fn apply_with_libseccomp(profile: ProcessSeccompProfile) -> Result<()> {
    use libseccomp::{ScmpAction, ScmpFilterContext, ScmpSyscall};

    // Default action: ALLOW (coordinator needs many syscalls; we deny specific ones).
    // For Strict profile we invert: default DENY with explicit allow-list.
    let filter = match profile {
        ProcessSeccompProfile::Coordinator => build_coordinator_filter()?,
        ProcessSeccompProfile::Strict => build_strict_filter()?,
    };

    filter
        .load()
        .map_err(|e| anyhow::anyhow!("failed to load seccomp filter: {e}"))?;

    tracing::info!(profile = ?profile, "process seccomp filter applied");
    Ok(())
}

/// Build a deny-list filter (default ALLOW, deny dangerous syscalls).
#[cfg(feature = "seccomp")]
fn build_coordinator_filter() -> Result<libseccomp::ScmpFilterContext> {
    use libseccomp::{ScmpAction, ScmpFilterContext, ScmpSyscall};

    let mut filter = ScmpFilterContext::new_filter(ScmpAction::Allow)
        .map_err(|e| anyhow::anyhow!("seccomp filter create: {e}"))?;

    // Syscalls that could lead to privilege escalation or system compromise.
    let denied: &[&str] = &[
        // Debugging/injection
        "ptrace",
        "process_vm_readv",
        "process_vm_writev",
        // Kernel module loading
        "init_module",
        "finit_module",
        "delete_module",
        // System restart / kexec
        "reboot",
        "kexec_load",
        "kexec_file_load",
        // Privilege escalation via UID/GID changes
        "setuid",
        "setuid32",
        "setgid",
        "setgid32",
        "setresuid",
        "setresuid32",
        "setresgid",
        "setresgid32",
        "setfsuid",
        "setfsuid32",
        "setfsgid",
        "setfsgid32",
        // Kernel keyring — can leak credentials
        "keyctl",
        "add_key",
        "request_key",
        // Device node creation
        "mknod",
        "mknodat",
        // Mount/unmount (bwrap handles these in child namespaces)
        "mount",
        "umount2",
        "unshare",
        "pivot_root",
        // Dangerous socket operations
        "create_module",
        "query_module",
        // perf events (can be used for side-channel attacks)
        "perf_event_open",
        // eBPF — powerful and dangerous
        "bpf",
    ];

    for name in denied {
        match ScmpSyscall::from_name(name) {
            Ok(syscall) => {
                filter
                    .add_rule(ScmpAction::KillProcess, syscall)
                    .map_err(|e| anyhow::anyhow!("add seccomp rule for {name}: {e}"))?;
            }
            Err(_) => {
                // Syscall doesn't exist on this architecture — skip silently.
                tracing::debug!(syscall = name, "seccomp: syscall not found on this arch, skipping");
            }
        }
    }

    Ok(filter)
}

/// Build an allow-list filter (default KILL, allow specific syscalls).
#[cfg(feature = "seccomp")]
fn build_strict_filter() -> Result<libseccomp::ScmpFilterContext> {
    use libseccomp::{ScmpAction, ScmpFilterContext, ScmpSyscall};

    let mut filter = ScmpFilterContext::new_filter(ScmpAction::KillProcess)
        .map_err(|e| anyhow::anyhow!("seccomp strict filter create: {e}"))?;

    // Minimal syscalls for the coordinator's work:
    // file I/O, process management, sockets, memory, synchronization.
    let allowed: &[&str] = &[
        // Memory management
        "brk", "mmap", "mmap2", "mprotect", "munmap", "mremap",
        "madvise", "mincore",
        // File I/O
        "read", "readv", "pread64", "write", "writev", "pwrite64",
        "open", "openat", "openat2", "close", "stat", "fstat", "lstat",
        "stat64", "fstat64", "lstat64", "newfstatat", "fstatat64",
        "lseek", "llseek", "_llseek",
        "access", "faccessat", "faccessat2",
        "ioctl", "fcntl", "fcntl64",
        "dup", "dup2", "dup3",
        "pipe", "pipe2",
        "mkdir", "mkdirat", "rmdir",
        "unlink", "unlinkat",
        "rename", "renameat", "renameat2",
        "chmod", "fchmod", "fchmodat",
        "chown", "fchown", "lchown", "fchownat",
        "symlink", "symlinkat", "readlink", "readlinkat",
        "link", "linkat",
        "getdents", "getdents64",
        "select", "pselect6", "poll", "ppoll",
        "epoll_create", "epoll_create1", "epoll_ctl", "epoll_wait", "epoll_pwait",
        "sendfile", "sendfile64",
        "truncate", "ftruncate", "truncate64", "ftruncate64",
        "utimes", "utime", "futimesat", "utimensat",
        // Process management
        "fork", "vfork", "clone", "clone3",
        "execve", "execveat",
        "wait4", "waitid", "waitpid",
        "exit", "exit_group",
        "getpid", "getppid", "gettid",
        "getuid", "getuid32", "getgid", "getgid32",
        "geteuid", "geteuid32", "getegid", "getegid32",
        "getgroups", "getgroups32",
        "getpgid", "setpgid", "getsid", "setsid",
        "kill", "tgkill", "tkill",
        "prctl",
        // Signals
        "rt_sigaction", "rt_sigprocmask", "rt_sigreturn", "rt_sigsuspend",
        "rt_sigpending", "rt_sigqueueinfo", "rt_tgsigqueueinfo",
        "sigaltstack", "signalfd", "signalfd4",
        // Networking (modules may need DNS/TCP)
        "socket", "connect", "bind", "listen", "accept", "accept4",
        "sendto", "recvfrom", "sendmsg", "recvmsg",
        "setsockopt", "getsockopt", "getsockname", "getpeername",
        "shutdown", "socketpair",
        // Time
        "clock_gettime", "clock_gettime64", "clock_nanosleep", "clock_nanosleep_time64",
        "gettimeofday", "time", "times", "nanosleep",
        // Synchronization
        "futex", "futex_time64",
        // Memory locking (some modules may need this)
        "mlock", "munlock", "mlockall", "munlockall",
        // Misc
        "getcwd", "chdir", "fchdir",
        "umask",
        "getrlimit", "ugetrlimit", "setrlimit", "prlimit64",
        "getrusage",
        "sysinfo",
        "uname",
        "getrandom",
        "arch_prctl",
        "set_tid_address", "set_robust_list", "get_robust_list",
        "rseq",
    ];

    for name in allowed {
        match ScmpSyscall::from_name(name) {
            Ok(syscall) => {
                filter
                    .add_rule(ScmpAction::Allow, syscall)
                    .map_err(|e| anyhow::anyhow!("add seccomp allow rule for {name}: {e}"))?;
            }
            Err(_) => {
                tracing::debug!(syscall = name, "seccomp strict: syscall not found, skipping");
            }
        }
    }

    Ok(filter)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_from_level() {
        assert_eq!(
            ProcessSeccompProfile::from(crate::config::ProcessSeccompLevel::Coordinator),
            ProcessSeccompProfile::Coordinator
        );
        assert_eq!(
            ProcessSeccompProfile::from(crate::config::ProcessSeccompLevel::Strict),
            ProcessSeccompProfile::Strict
        );
    }

    #[test]
    fn test_apply_noop_without_feature() {
        // When seccomp feature is absent this should return Ok(()) immediately.
        // When present, applying a Coordinator profile to the test process
        // must also succeed (we only deny truly dangerous syscalls).
        let result = apply_process_seccomp(ProcessSeccompProfile::Coordinator);
        assert!(result.is_ok(), "apply_process_seccomp failed: {:?}", result);
    }
}
