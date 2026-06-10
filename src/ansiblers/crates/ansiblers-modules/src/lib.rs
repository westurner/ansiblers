//! `ansiblers-modules` — module registry and built-in module implementations.
//!
//! Modules are the units of work in Ansible playbooks.  This crate provides:
//!
//! ## Built-in Rust modules
//!
//! | Module | Phase | Description |
//! |--------|-------|-------------|
//! | `shell` | 1 | Execute commands via `/bin/sh` |
//! | `command` | 1 | Execute commands without shell interpolation |
//! | `debug` | 1 | Print messages or variable values |
//! | `set_fact` | 1 | Store per-host facts in the execution context |
//! | `fail` | 1 | Immediately fail with a custom message |
//! | `file` | 2 | Manage file/directory/symlink state |
//! | `copy` | 2 | Copy files or write string content to a destination |
//! | `stat` | 2 | Gather file/directory metadata |
//! | `apt` | 5 | Manage Debian/Ubuntu packages via `apt-get` |
//! | `yum` / `dnf` | 5 | Manage RPM packages via `yum`/`dnf` |
//! | `find` | 5 | Recursively search a directory tree |
//! | `template` | 5 | Render a Jinja2 template file to a destination |
//! | `lineinfile` | 5 | Ensure a line is present/absent in a file (fancy-regex) |
//! | `setup` | 5 | Gather system facts into `ansible_*` namespace |
//! | `git` | 5 | Manage git repositories (clone, pull, checkout) |
//! | `dnf_history` | 5 | DNF transaction history (CLI → python → SQLite fallback) |
//! | `ostree` | 5 | Manage OSTree repos, refs, remotes, commits, admin ops |
//! | `rpm_ostree` | 5 | Manage packages/deployments on rpm-ostree systems |
//! | `pacman` | 5 | Arch Linux packages via pacman/yay/paru |
//! | `apk` | 5 | Alpine Linux packages via apk |
//! | `zypper` | 5 | SUSE/openSUSE packages via zypper |
//! | `homebrew` | 5 | macOS/Linux packages via brew (formulae + casks) |
//! | `conda` | 5 | conda/mamba/micromamba environments + cross-platform |
//! | `pixi` | 5 | Project environments via pixi (emscripten-wasm32 etc.) |
//! | `pip` | 5 | Python packages via pip/pip3 |
//! | `uv` | 5 | Python packages + tools via uv |
//! | `flatpak` | 5 | Flatpak applications and runtimes |
//! | `snap` | 5 | Snap packages via snapd |
//! | `appimage` | 5 | AppImage bundles (download, verify, integrate) |
//! | `nix` | 5 | Nix packages + flakes (+ GNU Guix via `use_guix`) |
//! | `portage` | 5 | Gentoo Linux packages via emerge |
//! | `slackpkg` | 5 | Slackware pkgtools / slackpkg / slapt-get |
//! | `chocolatey` | 5 | Windows packages via Chocolatey / NuGet |
//! | `pkg_add` | 5 | OpenBSD packages via pkg_add / pkg_delete |
//! | `pkgng` | 5 | FreeBSD packages via pkg (pkgng) |
//! | `pkgsrc` | 5 | NetBSD/MINIX 3 packages via pkgin / pkgsrc |
//! | `macports` | 5 | macOS packages via MacPorts |
//! | `pkg5` | 5 | OpenIndiana / Solaris 11+ IPS (pkg freeze/verify/fix) |
//! | `svr4pkg` | 5 | Oracle Solaris 10 SVR4 packages (pkgadd/pkgrm) |
//! | `zopen` | 5 | z/OS Open Tools packages |
//! | `zos` | 5 | IBM z/OS system resources via z/OSMF REST API |
//! | `poetry` | 5 | Python project + package management via Poetry |
//! | `pip_tools` | 5 | pip-compile + pip-sync (pip-tools) |
//! | `pipenv` | 5 | Python virtualenv + packages via Pipenv |
//!
//! ## Python module invocation
//!
//! [`ConfigurablePythonInvoker`] supports three backends selected via
//! [`PythonInvokeMode`]:
//!
//! | Mode | Mechanism | Parallelism |
//! |------|-----------|-------------|
//! | `Subprocess` (default) | `python3 <module> <args_file>` | Full (separate process) |
//! | `Native` | PyO3 in-process `exec()` | Concurrent (I/O yields GIL) |
//! | `SubInterpreter` | Python 3.12+ `_interpreters` | Full (own GIL per interpreter) |
//!
//! ## Preview Mode sandbox
//!
//! [`PreviewModeWrapper`] wraps any `ModuleInvoker` in a bubblewrap container
//! with OverlayFS diff capture, emulating Ansible `--diff` / `--check` modes.
//!
//! ## Registry
//!
//! ```rust,no_run
//! use ansiblers_modules::ModuleRegistry;
//!
//! // All built-in modules pre-registered:
//! let mut registry = ModuleRegistry::with_defaults();
//!
//! // Fallback to Python for unknown modules:
//! use ansiblers_modules::{ConfigurablePythonInvoker, PythonModuleConfig};
//! use std::sync::Arc;
//! let python = Arc::new(ConfigurablePythonInvoker::default_subprocess());
//! registry.register("my_custom_module", python);
//! ```

pub mod apk;
pub mod appimage;
pub mod apt;
pub mod chocolatey;
pub mod command;
pub mod conda;
pub mod copy;
pub mod debug;
pub mod dnf_history;
pub mod fail;
pub mod file;
pub mod find;
pub mod flatpak;
pub mod git;
pub mod homebrew;
pub mod lineinfile;
pub mod macports;
pub mod module_cache;
pub mod nix;
pub mod ostree;
pub mod pacman;
pub mod pip;
pub mod pip_tools;
pub mod pipenv;
pub mod pixi;
pub mod pkg5;
pub mod pkg_add;
pub mod pkgng;
pub mod pkgsrc;
pub mod poetry;
pub mod portage;
pub mod preview;
pub mod python_wrapper;
pub mod registry;
pub mod rpm_ostree;
pub mod set_fact;
pub mod setup;
pub mod shell;
pub mod slackpkg;
pub mod snap;
pub mod stat;
pub mod svr4pkg;
pub mod template;
pub mod uv;
pub mod yum;
pub mod zopen;
pub mod zos;
pub mod zypper;

pub use module_cache::{
    CacheKey, CachingModuleRegistry, InMemoryCache, ModuleResultCache, SqliteCache,
    CACHEABLE_MODULES,
};
pub use preview::{is_bwrap_available, PreviewModeWrapper};
pub use python_wrapper::{
    discover_ansible_library_paths, AnsiblePythonModuleInvoker, ConfigurablePythonInvoker,
    PythonInvokeMode, PythonModuleConfig, PythonModuleWrapper, SubprocessInvoker,
};
#[cfg(feature = "native-python")]
pub use python_wrapper::{NativePythonInvoker, SubInterpreterInvoker};
pub use registry::{ModuleArgs, ModuleInvoker, ModuleRegistry};
