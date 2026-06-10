//! `ansiblers-sandbox` — defense-in-depth security for ansiblers.
//!
//! ## Architecture: Three Complementary Layers
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │  Layer 1: Template Sandbox  (jinja2rs::SandboxedEnvironment)│
//! │  • Strict undefined-variable errors                         │
//! │  • Denied attribute list (__class__, __mro__, etc.)         │
//! │  • Path policy: restrict template include/loader paths      │
//! │  • Optional seccomp for template-render thread              │
//! │  • Optional resource limits (memory, CPU time)              │
//! ├─────────────────────────────────────────────────────────────┤
//! │  Layer 2: Module Sandbox  (bwrap + seccomp profile)         │
//! │  • Read-only bind mount of /                                │
//! │  • OverlayFS upperdir captures all filesystem writes        │
//! │  • Seccomp profile injected as bwrap --seccomp fd           │
//! │  • Unprivileged user namespace (no root escalation)         │
//! │  • No network namespace by default (--unshare-net)          │
//! ├─────────────────────────────────────────────────────────────┤
//! │  Layer 3: Process Seccomp  (libseccomp on the Rust process) │
//! │  • Restricts ansiblers coordinator process syscalls         │
//! │  • Denies: ptrace, kexec_load, keyctl, setuid, …           │
//! │  • Applied once at startup before any module execution      │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Why not asyncio / single-layer?
//!
//! Each layer addresses a different threat model:
//! - Layer 1 stops template injection (attacker controls template text)
//! - Layer 2 stops module escape (malicious module binary)
//! - Layer 3 stops privilege escalation (compromised ansiblers process)
//!
//! ## Feature flags
//!
//! | Feature | What it enables |
//! |---------|----------------|
//! | `seccomp` | Layer 3: process-level seccomp via libseccomp |
//! | `bwrap` | Layer 2: per-module bubblewrap isolation |
//! | `jinja2-sandbox` | Layer 1: `jinja2rs::SandboxedEnvironment` |
//! | `resource-limits` | RLIMIT_AS + RLIMIT_CPU for subprocesses |
//! | `full` | All of the above |

pub mod config;
pub mod layers;
pub mod process_seccomp;
pub mod template_sandbox;

pub use config::{SandboxConfig, SandboxLayer};
pub use layers::SandboxedModuleRegistry;
pub use process_seccomp::{apply_process_seccomp, ProcessSeccompProfile};
