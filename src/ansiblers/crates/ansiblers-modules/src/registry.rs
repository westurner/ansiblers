//! Module registry — maps module names to their implementations.

use std::collections::HashMap;
use std::sync::Arc;

use ansiblers_core::{ExecutionContext, TaskResult};
use anyhow::Result;

use crate::apk::ApkModule;
use crate::appimage::AppimageModule;
use crate::apt::AptModule;
use crate::chocolatey::ChocolateyModule;
use crate::command::CommandModule;
use crate::conda::CondaModule;
use crate::copy::CopyModule;
use crate::debug::DebugModule;
use crate::dnf_history::DnfHistoryModule;
use crate::fail::FailModule;
use crate::file::FileModule;
use crate::find::FindModule;
use crate::flatpak::FlatpakModule;
use crate::git::GitModule;
use crate::homebrew::HomebrewModule;
use crate::lineinfile::LineinfileModule;
use crate::macports::MacPortsModule;
use crate::nix::NixModule;
use crate::ostree::OstreeModule;
use crate::pacman::PacmanModule;
use crate::pip::PipModule;
use crate::pip_tools::PipToolsModule;
use crate::pipenv::PipenvModule;
use crate::pixi::PixiModule;
use crate::pkg5::Pkg5Module;
use crate::pkg_add::PkgAddModule;
use crate::pkgng::PkgngModule;
use crate::pkgsrc::PkgsrcModule;
use crate::poetry::PoetryModule;
use crate::portage::PortageModule;
use crate::rpm_ostree::RpmOstreeModule;
use crate::set_fact::SetFactModule;
use crate::setup::SetupModule;
use crate::shell::ShellModule;
use crate::slackpkg::SlackpkgModule;
use crate::snap::SnapModule;
use crate::stat::StatModule;
use crate::svr4pkg::Svr4PkgModule;
use crate::template::TemplateModule;
use crate::uv::UvModule;
use crate::yum::YumModule;
use crate::zopen::ZopenModule;
use crate::zos::ZosModule;
use crate::zypper::ZypperModule;

/// Arguments passed to a module at invocation time.
///
/// `args` holds the rendered key/value parameters from the task YAML.
/// `task_name` is populated from `task.name` for use in error messages.
#[derive(Debug, Clone)]
pub struct ModuleArgs {
    pub args: HashMap<String, ansiblers_core::Value>,
    pub task_name: Option<String>,
}

impl ModuleArgs {
    pub fn new(args: HashMap<String, ansiblers_core::Value>) -> Self {
        Self {
            args,
            task_name: None,
        }
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.args.get(key).and_then(|v| v.as_str())
    }

    pub fn get_raw_params(&self) -> Option<&str> {
        self.get_str("_raw_params")
    }
}

/// Trait implemented by every module.
///
/// Each implementation corresponds to one Ansible module name (e.g. `"shell"`,
/// `"copy"`, `"apt"`).  Modules receive rendered arguments, the target
/// hostname, and mutable access to the [`ExecutionContext`] (to write facts
/// via `set_fact` or read registered variables).
///
/// All built-in modules are synchronous and blocking.  Async support is planned
/// for Phase 6 (WebRTC transport).
pub trait ModuleInvoker: Send + Sync {
    /// Execute the module synchronously for a single host.
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult>;
}

/// Registry that maps module names → [`ModuleInvoker`] implementations.
///
/// Use [`ModuleRegistry::with_defaults`] to get a registry pre-populated with
/// all Phase 1+2 built-in Rust modules.  Additional modules (Python wrappers,
/// custom Rust modules) can be registered at runtime via [`register`](Self::register).
///
/// # Fallback strategy
///
/// Unregistered modules return a failed [`TaskResult`] with `"module not found"`
/// rather than panicking.  To support arbitrary Python modules transparently,
/// wrap the registry with a [`ConfigurablePythonInvoker`](crate::ConfigurablePythonInvoker)
/// as the default invoker.
pub struct ModuleRegistry {
    modules: HashMap<String, Arc<dyn ModuleInvoker>>,
}

impl ModuleRegistry {
    /// Create a registry pre-populated with Phase 1+2 built-in modules.
    pub fn with_defaults() -> Self {
        let mut r = Self {
            modules: HashMap::new(),
        };
        // Phase 1
        r.register("shell", Arc::new(ShellModule));
        r.register("command", Arc::new(CommandModule));
        r.register("debug", Arc::new(DebugModule));
        r.register("set_fact", Arc::new(SetFactModule));
        r.register("fail", Arc::new(FailModule));
        // Phase 2
        r.register("file", Arc::new(FileModule));
        r.register("copy", Arc::new(CopyModule));
        r.register("stat", Arc::new(StatModule));
        // Phase 5
        r.register("apt", Arc::new(AptModule));
        r.register("yum", Arc::new(YumModule));
        r.register("dnf", Arc::new(YumModule));
        r.register("find", Arc::new(FindModule));
        r.register("template", Arc::new(TemplateModule));
        r.register("lineinfile", Arc::new(LineinfileModule));
        r.register("setup", Arc::new(SetupModule));
        r.register("gather_facts", Arc::new(SetupModule));
        r.register("git", Arc::new(GitModule));
        // ostree / rpm-ostree
        r.register("ostree", Arc::new(OstreeModule));
        r.register("rpm_ostree", Arc::new(RpmOstreeModule));
        // dnf history
        r.register("dnf_history", Arc::new(DnfHistoryModule));
        // cross-platform / other package managers
        r.register("pacman", Arc::new(PacmanModule));
        r.register("apk", Arc::new(ApkModule));
        r.register("zypper", Arc::new(ZypperModule));
        r.register("homebrew", Arc::new(HomebrewModule));
        r.register("brew", Arc::new(HomebrewModule));
        r.register("conda", Arc::new(CondaModule));
        r.register("mamba", Arc::new(CondaModule));
        r.register("micromamba", Arc::new(CondaModule));
        r.register("pixi", Arc::new(PixiModule));
        r.register("pip", Arc::new(PipModule));
        r.register("pip3", Arc::new(PipModule));
        r.register("uv", Arc::new(UvModule));
        // more Python package managers
        r.register("poetry", Arc::new(PoetryModule));
        r.register("pip_tools", Arc::new(PipToolsModule));
        r.register("pipenv", Arc::new(PipenvModule));
        // universal / container / app packaging
        r.register("flatpak", Arc::new(FlatpakModule));
        r.register("snap", Arc::new(SnapModule));
        r.register("appimage", Arc::new(AppimageModule));
        // functional / source-based
        r.register("nix", Arc::new(NixModule));
        r.register("guix", Arc::new(NixModule));
        r.register("portage", Arc::new(PortageModule));
        r.register("emerge", Arc::new(PortageModule));
        // Slackware
        r.register("slackpkg", Arc::new(SlackpkgModule));
        r.register("pkgtools", Arc::new(SlackpkgModule));
        // Windows
        r.register("chocolatey", Arc::new(ChocolateyModule));
        r.register("choco", Arc::new(ChocolateyModule));
        // BSD / macOS
        r.register("pkg_add", Arc::new(PkgAddModule));
        r.register("pkgng", Arc::new(PkgngModule));
        r.register("pkg", Arc::new(PkgngModule));
        r.register("pkgsrc", Arc::new(PkgsrcModule));
        r.register("pkgin", Arc::new(PkgsrcModule));
        r.register("macports", Arc::new(MacPortsModule));
        r.register("port", Arc::new(MacPortsModule));
        // Solaris / OpenIndiana / Illumos
        r.register("pkg5", Arc::new(Pkg5Module));
        r.register("pkg6", Arc::new(Pkg5Module)); // IPS alias for newer Solaris 11.4+
        r.register("ips", Arc::new(Pkg5Module));
        r.register("svr4pkg", Arc::new(Svr4PkgModule));
        r.register("pkgadd", Arc::new(Svr4PkgModule));
        // z/OS
        r.register("zopen", Arc::new(ZopenModule));
        r.register("zos", Arc::new(ZosModule));
        r
    }

    pub fn register(&mut self, name: &str, invoker: Arc<dyn ModuleInvoker>) {
        self.modules.insert(name.to_string(), invoker);
    }

    /// Clone the registry by re-creating it with defaults.
    /// Used by the free-strategy multi-thread executor.
    pub fn clone_defaults() -> Self {
        Self::with_defaults()
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn ModuleInvoker>> {
        self.modules.get(name).cloned()
    }

    pub fn invoke(
        &self,
        module: &str,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        match self.modules.get(module) {
            Some(invoker) => invoker.invoke(args, host, ctx),
            None => Ok(TaskResult::failed(
                host,
                format!("module not found: {module}"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ansiblers_core::{ExecutionContext, Inventory, Value};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn ctx() -> ExecutionContext {
        ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new())
    }

    fn empty_args() -> ModuleArgs {
        ModuleArgs::new(HashMap::new())
    }

    #[test]
    fn test_with_defaults_registers_shell() {
        let r = ModuleRegistry::with_defaults();
        assert!(r.get("shell").is_some());
    }

    #[test]
    fn test_with_defaults_registers_all_phase1() {
        let r = ModuleRegistry::with_defaults();
        for name in &["shell", "command", "debug", "set_fact", "fail"] {
            assert!(r.get(name).is_some(), "missing: {name}");
        }
    }

    #[test]
    fn test_with_defaults_registers_phase2() {
        let r = ModuleRegistry::with_defaults();
        for name in &["file", "copy", "stat"] {
            assert!(r.get(name).is_some(), "missing: {name}");
        }
    }

    #[test]
    fn test_with_defaults_registers_package_managers() {
        let r = ModuleRegistry::with_defaults();
        for name in &[
            "apt", "yum", "dnf", "pacman", "apk", "zypper", "brew", "conda", "pip", "pip3", "uv",
            "pixi", "poetry", "pipenv",
        ] {
            assert!(r.get(name).is_some(), "missing: {name}");
        }
    }

    #[test]
    fn test_with_defaults_registers_bsd_modules() {
        let r = ModuleRegistry::with_defaults();
        for name in &["pkg_add", "pkg", "pkgin", "port"] {
            assert!(r.get(name).is_some(), "missing: {name}");
        }
    }

    #[test]
    fn test_with_defaults_registers_universal_packaging() {
        let r = ModuleRegistry::with_defaults();
        for name in &[
            "flatpak", "snap", "appimage", "nix", "guix", "emerge", "portage",
        ] {
            assert!(r.get(name).is_some(), "missing: {name}");
        }
    }

    #[test]
    fn test_with_defaults_registers_zos() {
        let r = ModuleRegistry::with_defaults();
        assert!(r.get("zopen").is_some());
        assert!(r.get("zos").is_some());
    }

    #[test]
    fn test_unknown_module_returns_failed() {
        let r = ModuleRegistry::with_defaults();
        let mut c = ctx();
        let result = r
            .invoke("no_such_module", &empty_args(), "h", &mut c)
            .unwrap();
        assert!(result.status.is_failed());
        assert!(result.msg.contains("module not found"));
    }

    #[test]
    fn test_custom_register_and_get() {
        let mut r = ModuleRegistry::with_defaults();
        // Re-register "debug" under a custom name.
        let debug_invoker = r.get("debug").unwrap();
        r.register("my_debug", debug_invoker);
        assert!(r.get("my_debug").is_some());
    }

    #[test]
    fn test_clone_defaults_has_same_modules() {
        let r = ModuleRegistry::clone_defaults();
        assert!(r.get("shell").is_some());
        assert!(r.get("apt").is_some());
    }

    #[test]
    fn test_module_args_get_str() {
        let mut m = HashMap::new();
        m.insert("key".to_string(), Value::String("val".to_string()));
        let args = ModuleArgs::new(m);
        assert_eq!(args.get_str("key"), Some("val"));
        assert_eq!(args.get_str("missing"), None);
    }

    #[test]
    fn test_module_args_get_raw_params() {
        let mut m = HashMap::new();
        m.insert(
            "_raw_params".to_string(),
            Value::String("echo hi".to_string()),
        );
        let args = ModuleArgs::new(m);
        assert_eq!(args.get_raw_params(), Some("echo hi"));
    }

    #[test]
    fn test_invoke_shell_echo_succeeds() {
        let r = ModuleRegistry::with_defaults();
        let mut c = ctx();
        let mut m = HashMap::new();
        m.insert(
            "_raw_params".to_string(),
            Value::String("echo registry_test".to_string()),
        );
        let result = r.invoke("shell", &ModuleArgs::new(m), "h", &mut c).unwrap();
        assert!(result.status.is_ok());
        assert!(result.stdout.contains("registry_test"));
    }
}
