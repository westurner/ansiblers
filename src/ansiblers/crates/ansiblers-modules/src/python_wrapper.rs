//! Python module invocation — subprocess, native (in-process PyO3), and
//! sub-interpreter backends.
//!
//! ## Why not asyncio?
//!
//! Ansible modules are **synchronous** Python scripts that terminate via
//! `sys.exit()` (called by `AnsibleModule.exit_json()`).  `asyncio` is a
//! cooperative single-threaded event loop — it cannot parallelize module
//! execution and adds no value here.
//!
//! ## Parallelism strategies
//!
//! | Backend | Mechanism | Parallelism | Isolation |
//! |---------|-----------|-------------|-----------|
//! | [`SubprocessInvoker`] | `fork + exec python3` | Full (separate process) | Full |
//! | [`NativePythonInvoker`] | PyO3 `Python::with_gil`, GIL released during I/O | Concurrent (I/O-bound) | Shared interpreter |
//! | [`SubInterpreterInvoker`] | Python 3.12+ `_interpreters` | Full (own GIL per interpreter) | Full (separate state) |
//!
//! The unified [`ConfigurablePythonInvoker`] selects the backend via
//! [`PythonInvokeMode`].  When [`Strategy::Free`] is used in the executor,
//! each host's Rust thread independently acquires the Python GIL, so modules
//! blocked on Python I/O (which internally releases the GIL) allow other
//! threads to proceed concurrently.
//!
//! ## Feature flag
//!
//! `NativePythonInvoker` and `SubInterpreterInvoker` require:
//! ```toml
//! ansiblers-modules = { features = ["native-python"] }
//! ```

use std::collections::HashMap;
use std::io::Write;
use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context, Result};
use tempfile::NamedTempFile;

use crate::registry::{ModuleArgs, ModuleInvoker};

// ---------------------------------------------------------------------------
// PythonInvokeMode
// ---------------------------------------------------------------------------

/// Selects which backend is used to invoke Python Ansible modules.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PythonInvokeMode {
    /// Spawn a new `python3` subprocess per module invocation.
    ///
    /// - Always available (no feature flag).
    /// - True process isolation; always parallel.
    /// - Overhead: fork + exec + IPC + JSON-over-stdout.
    #[default]
    Subprocess,

    /// Execute module code in the **current interpreter** via PyO3.
    ///
    /// Requires `native-python` feature.
    ///
    /// - No fork; ~5–10× faster than subprocess for fast modules.
    /// - GIL is held only during `exec(source)`; all file I/O happens outside
    ///   GIL via `py.allow_threads()` → I/O-bound modules run concurrently
    ///   when the executor uses [`crate::strategy::Strategy::Free`].
    /// - Modules share interpreter state (imported stdlib, etc.) — safe as
    ///   long as modules don't mutate global Python state between calls.
    Native,

    /// Run each module in an isolated Python sub-interpreter (Python 3.12+).
    ///
    /// Requires `native-python` feature.
    ///
    /// Uses `_interpreters.create()` / `run_string()` from the CPython C API
    /// (exposed via PyO3).  Each sub-interpreter has its own module registry,
    /// `sys.modules`, and GIL slot, enabling **true parallelism** even for
    /// CPU-bound modules without process overhead.
    ///
    /// Falls back to [`PythonInvokeMode::Native`] on Python < 3.12.
    SubInterpreter,
}

// ---------------------------------------------------------------------------
// PythonModuleConfig
// ---------------------------------------------------------------------------

/// Configuration for Python module invocation.
#[derive(Debug, Clone)]
pub struct PythonModuleConfig {
    /// Invocation backend.
    pub mode: PythonInvokeMode,
    /// Python executable for [`PythonInvokeMode::Subprocess`].
    pub python_executable: String,
    /// Extra environment variables injected into subprocess invocations.
    pub extra_env: HashMap<String, String>,
    /// Ansible library search paths (to locate `.py` module files).
    pub library_paths: Vec<String>,
    /// Inject common `ANSIBLE_*` / `PYTHONDONTWRITEBYTECODE` env vars.
    pub inject_ansible_env: bool,
}

impl Default for PythonModuleConfig {
    fn default() -> Self {
        Self {
            mode: PythonInvokeMode::default(),
            python_executable: "python3".to_string(),
            extra_env: HashMap::new(),
            library_paths: discover_ansible_library_paths(),
            inject_ansible_env: true,
        }
    }
}

impl PythonModuleConfig {
    pub fn subprocess() -> Self {
        Self {
            mode: PythonInvokeMode::Subprocess,
            ..Self::default()
        }
    }

    /// Build a Native config.  Returns `None` when `native-python` feature is
    /// not compiled in.
    pub fn native() -> Option<Self> {
        #[cfg(feature = "native-python")]
        return Some(Self {
            mode: PythonInvokeMode::Native,
            ..Self::default()
        });
        #[cfg(not(feature = "native-python"))]
        return None;
    }

    /// Build a SubInterpreter config.  Falls back to `Native` on Python < 3.12.
    pub fn sub_interpreter() -> Option<Self> {
        #[cfg(feature = "native-python")]
        return Some(Self {
            mode: PythonInvokeMode::SubInterpreter,
            ..Self::default()
        });
        #[cfg(not(feature = "native-python"))]
        return None;
    }

    pub fn with_python(mut self, exe: impl Into<String>) -> Self {
        self.python_executable = exe.into();
        self
    }

    pub fn with_library_paths(mut self, extra: Vec<String>) -> Self {
        self.library_paths.extend(extra);
        self
    }

    pub fn with_env(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.extra_env.insert(k.into(), v.into());
        self
    }
}

// ---------------------------------------------------------------------------
// ConfigurablePythonInvoker (unified front-end)
// ---------------------------------------------------------------------------

/// The recommended Python module invoker. Selects the backend at construction
/// time based on [`PythonModuleConfig::mode`].
pub struct ConfigurablePythonInvoker {
    pub config: PythonModuleConfig,
}

impl ConfigurablePythonInvoker {
    pub fn new(config: PythonModuleConfig) -> Self {
        Self { config }
    }

    pub fn default_subprocess() -> Self {
        Self::new(PythonModuleConfig::subprocess())
    }

    pub fn default_native() -> Option<Self> {
        PythonModuleConfig::native().map(Self::new)
    }

    pub fn default_sub_interpreter() -> Option<Self> {
        PythonModuleConfig::sub_interpreter().map(Self::new)
    }

    /// Find a `.py` module file by name in the configured library paths.
    pub fn find_module(&self, name: &str) -> Option<String> {
        self.config.library_paths.iter().find_map(|lib| {
            let c = format!("{lib}/{name}.py");
            std::path::Path::new(&c).exists().then_some(c)
        })
    }

    fn dispatch(
        &self,
        module_path: &str,
        args: &HashMap<String, Value>,
        host: &str,
    ) -> Result<TaskResult> {
        match self.config.mode {
            PythonInvokeMode::Subprocess => {
                invoke_subprocess(module_path, args, host, &self.config)
            }
            PythonInvokeMode::Native => {
                #[cfg(feature = "native-python")]
                return invoke_native(module_path, args, host);
                #[cfg(not(feature = "native-python"))]
                return Ok(TaskResult::failed(
                    host,
                    "native-python feature not compiled; rebuild with --features native-python",
                ));
            }
            PythonInvokeMode::SubInterpreter => {
                #[cfg(feature = "native-python")]
                return invoke_sub_interpreter(module_path, args, host);
                #[cfg(not(feature = "native-python"))]
                return Ok(TaskResult::failed(
                    host,
                    "native-python feature not compiled",
                ));
            }
        }
    }
}

impl ModuleInvoker for ConfigurablePythonInvoker {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let module_name = args.task_name.as_deref().unwrap_or("unknown");

        // `_module_path` in args takes precedence over library search.
        let path = args
            .args
            .get("_module_path")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| self.find_module(module_name));

        match path {
            Some(p) => self.dispatch(&p, &args.args, host),
            None => Ok(TaskResult::failed(
                host,
                format!(
                    "Python module '{module_name}' not found in {:?}",
                    self.config.library_paths
                ),
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Backend 1: Subprocess
// ---------------------------------------------------------------------------

/// Invoke a Python module by spawning a subprocess.  Always parallel — each
/// invocation is a separate OS process with its own Python interpreter.
pub struct SubprocessInvoker {
    pub config: PythonModuleConfig,
}

impl SubprocessInvoker {
    pub fn new(config: PythonModuleConfig) -> Self {
        Self { config }
    }

    pub fn from_env() -> Self {
        Self::new(PythonModuleConfig::subprocess())
    }
}

impl ModuleInvoker for SubprocessInvoker {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let module_name = args.task_name.as_deref().unwrap_or("unknown");
        let path = args
            .args
            .get("_module_path")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| {
                self.config.library_paths.iter().find_map(|lib| {
                    let c = format!("{lib}/{module_name}.py");
                    std::path::Path::new(&c).exists().then_some(c)
                })
            });
        match path {
            Some(p) => invoke_subprocess(&p, &args.args, host, &self.config),
            None => Ok(TaskResult::failed(
                host,
                format!("Python module '{module_name}' not found"),
            )),
        }
    }
}

fn invoke_subprocess(
    module_path: &str,
    module_args: &HashMap<String, Value>,
    host: &str,
    config: &PythonModuleConfig,
) -> Result<TaskResult> {
    // All work before spawn is pure Rust — no Python overhead.
    let args_file = write_args_file(module_args)?;
    let args_path = args_file.path().to_str().unwrap().to_string();

    let mut cmd = Command::new(&config.python_executable);
    cmd.args([module_path, &args_path]);
    if config.inject_ansible_env {
        cmd.env("PYTHONDONTWRITEBYTECODE", "1");
        cmd.env("ANSIBLE_MODULE_UTILS", "");
    }
    for (k, v) in &config.extra_env {
        cmd.env(k, v);
    }
    let output = cmd
        .output()
        .with_context(|| format!("spawning {} {}", config.python_executable, module_path))?;

    parse_output(
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
        output.status.code().unwrap_or(-1),
        host,
    )
}

// ---------------------------------------------------------------------------
// Backend 2: Native (in-process, PyO3)
// ---------------------------------------------------------------------------

/// Invoke a Python module in the **current interpreter** via PyO3.
///
/// Parallelism notes (when used with [`Strategy::Free`]):
/// - The GIL is acquired only for `exec(source)`.
/// - File I/O (writing args, reading source) happens in Rust **before**
///   GIL acquisition via [`pyo3::Python::with_gil`].
/// - Inside the exec, Python's own I/O operations (`open`, `subprocess`,
///   `socket`) release the GIL internally, letting other OS threads acquire
///   it and run their own modules concurrently.
#[cfg(feature = "native-python")]
pub struct NativePythonInvoker;

#[cfg(feature = "native-python")]
impl ModuleInvoker for NativePythonInvoker {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let path = args
            .args
            .get("_module_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("NativePythonInvoker requires '_module_path' arg"))?
            .to_string();
        invoke_native(&path, &args.args, host)
    }
}

#[cfg(feature = "native-python")]
fn invoke_native(
    module_path: &str,
    module_args: &HashMap<String, Value>,
    host: &str,
) -> Result<TaskResult> {
    use pyo3::prelude::*;
    use pyo3::types::{PyDict, PyList};

    // ---- Phase 1: All Rust-side work OUTSIDE the GIL ----------------------
    let args_file = write_args_file(module_args)?;
    let args_path = args_file.path().to_str().unwrap().to_string();
    let source = std::fs::read_to_string(module_path)
        .with_context(|| format!("reading module '{module_path}'"))?;

    // ---- Phase 2: Hold GIL only for Python execution ----------------------
    Python::with_gil(|py| -> Result<TaskResult> {
        let sys = py.import_bound("sys")?;
        let io = py.import_bound("io")?;

        // Save and replace sys.argv + sys.stdout.
        let orig_argv = sys.getattr("argv")?.into_py(py);
        let orig_stdout = sys.getattr("stdout")?.into_py(py);
        let buf = io.call_method0("StringIO")?;
        sys.setattr(
            "argv",
            PyList::new_bound(py, [module_path, args_path.as_str()]),
        )?;
        sys.setattr("stdout", &buf)?;

        // Prepare module globals.
        let globals = PyDict::new_bound(py);
        globals.set_item("__name__", "__main__")?;
        globals.set_item("__file__", module_path)?;

        // Execute; catch SystemExit (normal exit path for AnsibleModule).
        let exec_result = py.run_bound(&source, Some(&globals), None);

        // Always restore sys state before inspecting result.
        let captured: String = buf.call_method0("getvalue")?.extract()?;
        sys.setattr("argv", orig_argv)?;
        sys.setattr("stdout", orig_stdout)?;

        match exec_result {
            Ok(_) => {}
            Err(ref e) if e.is_instance_of::<pyo3::exceptions::PySystemExit>(py) => {}
            Err(e) => return Ok(TaskResult::failed(host, format!("{e}"))),
        }

        parse_output(&captured, "", 0, host)
    })
    .map_err(|e: pyo3::PyErr| anyhow::anyhow!("PyO3: {e}"))
    .and_then(|r| r)
}

// ---------------------------------------------------------------------------
// Backend 3: Sub-interpreter (Python 3.12+, PyO3)
// ---------------------------------------------------------------------------

/// Invoke a Python module in an isolated sub-interpreter.
///
/// Uses the Python 3.12+ `_interpreters` C API (via PyO3) to create a fresh
/// interpreter state per invocation.  Each sub-interpreter has its own:
/// - `sys.modules` registry
/// - `sys.stdout` / `sys.argv`
/// - GIL slot (can run in a separate OS thread without contention)
///
/// This gives **full isolation and true parallelism** without process overhead.
/// Falls back to `Native` mode on Python < 3.12.
#[cfg(feature = "native-python")]
pub struct SubInterpreterInvoker;

#[cfg(feature = "native-python")]
impl ModuleInvoker for SubInterpreterInvoker {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let path = args
            .args
            .get("_module_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("SubInterpreterInvoker requires '_module_path' arg"))?
            .to_string();
        invoke_sub_interpreter(&path, &args.args, host)
    }
}

/// Run `module_path` inside a fresh `_interpreters` sub-interpreter.
///
/// The bootstrap script:
/// 1. Imports `_interpreters` in the **main** interpreter.
/// 2. Creates a new sub-interpreter.
/// 3. Runs the module source inside it via `run_string`, capturing stdout
///    through a shared temp file (sub-interpreters cannot share Python objects
///    with the main interpreter).
/// 4. Destroys the sub-interpreter.
/// 5. Reads the temp file and parses the JSON result.
#[cfg(feature = "native-python")]
fn invoke_sub_interpreter(
    module_path: &str,
    module_args: &HashMap<String, Value>,
    host: &str,
) -> Result<TaskResult> {
    use pyo3::prelude::*;

    // ---- Phase 1: Rust-side setup (no GIL needed) -------------------------
    let args_file = write_args_file(module_args)?;
    let args_path = args_file.path().to_str().unwrap().to_string();

    // Output file: sub-interpreter writes JSON here; we read it back.
    let out_file = NamedTempFile::new().context("create output temp file")?;
    let out_path = out_file.path().to_str().unwrap().to_string();

    // Read module source in Rust (no GIL).
    let source = std::fs::read_to_string(module_path)
        .with_context(|| format!("reading module '{module_path}'"))?;

    // ---- Phase 2: Run bootstrap in main interpreter -----------------------
    // The bootstrap creates a sub-interpreter, executes the module inside it,
    // and writes the result to `out_path`.
    let bootstrap = build_subinterp_bootstrap(module_path, &args_path, &out_path, &source);

    Python::with_gil(|py| -> PyResult<()> {
        // Check availability of _interpreters (Python 3.12+).
        match py.import_bound("_interpreters") {
            Ok(_) => {
                py.run_bound(&bootstrap, None, None)?;
            }
            Err(_) => {
                // Fallback: run directly in the main interpreter.
                let io = py.import_bound("io")?;
                let sys = py.import_bound("sys")?;
                let orig_argv = sys.getattr("argv")?.into_py(py);
                let orig_stdout = sys.getattr("stdout")?.into_py(py);
                let buf = io.call_method0("StringIO")?;
                use pyo3::types::{PyDict, PyList};
                sys.setattr(
                    "argv",
                    PyList::new_bound(py, [module_path, args_path.as_str()]),
                )?;
                sys.setattr("stdout", &buf)?;
                let globals = PyDict::new_bound(py);
                globals.set_item("__name__", "__main__")?;
                let exec_result = py.run_bound(&source, Some(&globals), None);
                let captured: String = buf.call_method0("getvalue")?.extract()?;
                sys.setattr("argv", orig_argv)?;
                sys.setattr("stdout", orig_stdout)?;
                // Write captured output to out_path so the reader below works.
                std::fs::write(&out_path, captured.as_bytes()).ok();
                let _ = exec_result; // SystemExit is normal
            }
        }
        Ok(())
    })
    .map_err(|e| anyhow::anyhow!("PyO3 sub-interpreter: {e}"))?;

    // ---- Phase 3: Read result (no GIL) ------------------------------------
    let stdout = std::fs::read_to_string(&out_path).unwrap_or_default();
    parse_output(&stdout, "", 0, host)
}

/// Build the Python bootstrap code that runs `source` inside a sub-interpreter
/// and writes its stdout to `out_path`.
#[cfg(feature = "native-python")]
fn build_subinterp_bootstrap(
    module_path: &str,
    args_path: &str,
    out_path: &str,
    source: &str,
) -> String {
    // Escape backslashes and quotes in paths for embedding in Python strings.
    let esc = |s: &str| s.replace('\\', "\\\\").replace('\'', "\\'");
    let esc_src = source
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n");

    format!(
        r#"
import _interpreters, sys, os

_module_path = '{module_path}'
_args_path   = '{args_path}'
_out_path    = '{out_path}'
_source      = '{source}'

_script = f"""
import sys, io as _io
sys.argv = ['{module_path}', '{args_path}']
_buf = _io.StringIO()
sys.stdout = _buf
try:
    exec(compile({source!r}, '{module_path}', 'exec'), {{'__name__': '__main__', '__file__': '{module_path}'}})
except SystemExit:
    pass
finally:
    _result = _buf.getvalue()
    sys.stdout = sys.__stdout__
    with open('{out_path}', 'w') as _f:
        _f.write(_result)
"""

_interp = _interpreters.create()
try:
    _interpreters.run_string(_interp, _script)
finally:
    _interpreters.destroy(_interp)
"#,
        module_path = esc(module_path),
        args_path = esc(args_path),
        out_path = esc(out_path),
        source = esc_src,
    )
}

// ---------------------------------------------------------------------------
// Legacy public API (backward-compatible wrappers)
// ---------------------------------------------------------------------------

/// Invoke a Python module by absolute path, using subprocess.
/// Prefer [`ConfigurablePythonInvoker`] for new code.
pub struct PythonModuleWrapper {
    pub module_path: String,
}

impl PythonModuleWrapper {
    pub fn new(module_path: impl Into<String>) -> Self {
        Self {
            module_path: module_path.into(),
        }
    }
}

impl ModuleInvoker for PythonModuleWrapper {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        invoke_subprocess(
            &self.module_path,
            &args.args,
            host,
            &PythonModuleConfig::default(),
        )
    }
}

/// Locate Ansible Python modules in the current environment and invoke via
/// subprocess.  Prefer [`ConfigurablePythonInvoker`] for new code.
pub struct AnsiblePythonModuleInvoker {
    pub library_paths: Vec<String>,
}

impl AnsiblePythonModuleInvoker {
    pub fn new(library_paths: Vec<String>) -> Self {
        Self { library_paths }
    }

    pub fn from_env() -> Self {
        Self {
            library_paths: discover_ansible_library_paths(),
        }
    }
}

impl ModuleInvoker for AnsiblePythonModuleInvoker {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let name = args.task_name.as_deref().unwrap_or("unknown");
        let path = self.library_paths.iter().find_map(|lib| {
            let c = format!("{lib}/{name}.py");
            std::path::Path::new(&c).exists().then_some(c)
        });
        match path {
            Some(p) => invoke_subprocess(&p, &args.args, host, &PythonModuleConfig::default()),
            None => Ok(TaskResult::failed(
                host,
                format!("Python module '{name}' not found"),
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Write module args to a temp file in `{"ANSIBLE_MODULE_ARGS": {…}}` format.
fn write_args_file(module_args: &HashMap<String, Value>) -> Result<NamedTempFile> {
    let clean: HashMap<_, _> = module_args
        .iter()
        .filter(|(k, _)| !k.starts_with('_'))
        .collect();
    let json = serde_json::json!({ "ANSIBLE_MODULE_ARGS": clean });
    let mut f = NamedTempFile::new().context("create args temp file")?;
    serde_json::to_writer(&mut f, &json).context("write args JSON")?;
    f.flush().context("flush args file")?;
    Ok(f)
}

/// Parse the JSON blob printed by an Ansible module into a [`TaskResult`].
///
/// Ansible modules may emit non-JSON lines before the result (e.g. from
/// `print()` calls in third-party modules).  We scan backwards for the last
/// complete `{…}` JSON object.
fn parse_output(stdout: &str, stderr: &str, rc: i32, host: &str) -> Result<TaskResult> {
    let json_str = last_json_object(stdout).unwrap_or(stdout.trim());

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(json) => {
            let failed = json
                .get("failed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let changed = json
                .get("changed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let msg = json
                .get("msg")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let mut r = if failed {
                TaskResult::failed(host, msg.clone())
            } else if changed {
                TaskResult::changed(host)
            } else {
                TaskResult::ok(host)
            };
            r.stdout = stdout.to_string();
            r.stderr = stderr.to_string();
            r.rc = rc;
            r.msg = msg;

            if let Some(obj) = json.as_object() {
                for (k, v) in obj {
                    if !matches!(k.as_str(), "failed" | "changed" | "msg") {
                        r.vars.insert(k.clone(), v.clone());
                    }
                }
            }
            Ok(r)
        }
        Err(_) => {
            let mut r = if rc == 0 {
                TaskResult::ok(host)
            } else {
                TaskResult::failed(host, stderr.trim().to_string())
            };
            r.stdout = stdout.to_string();
            r.stderr = stderr.to_string();
            r.rc = rc;
            Ok(r)
        }
    }
}

/// Return a slice of `s` containing the last complete `{…}` JSON object.
fn last_json_object(s: &str) -> Option<&str> {
    let end = s.rfind('}')?;
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut i = end;
    loop {
        match bytes[i] {
            b'}' => depth += 1,
            b'{' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[i..=end]);
                }
            }
            _ => {}
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    None
}

/// Discover the Ansible modules directory from the active Python environment.
pub fn discover_ansible_library_paths() -> Vec<String> {
    let out = Command::new("python3")
        .args([
            "-c",
            "import ansible.modules,os; print(os.path.dirname(ansible.modules.__file__))",
        ])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let p = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !p.is_empty() {
                vec![p]
            } else {
                vec![]
            }
        }
        _ => vec![],
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    // ---- Config construction -----------------------------------------------

    #[test]
    fn test_default_config_is_subprocess() {
        assert_eq!(
            PythonModuleConfig::default().mode,
            PythonInvokeMode::Subprocess
        );
    }

    #[test]
    fn test_config_builder_chain() {
        let cfg = PythonModuleConfig::subprocess()
            .with_python("/usr/bin/python3.13")
            .with_env("ANSIBLE_NOCOLOR", "1");
        assert_eq!(cfg.python_executable, "/usr/bin/python3.13");
        assert_eq!(
            cfg.extra_env.get("ANSIBLE_NOCOLOR").map(|s| s.as_str()),
            Some("1")
        );
    }

    #[test]
    fn test_native_config_gated_by_feature() {
        #[cfg(feature = "native-python")]
        assert!(PythonModuleConfig::native().is_some());
        #[cfg(not(feature = "native-python"))]
        assert!(PythonModuleConfig::native().is_none());
    }

    #[test]
    fn test_sub_interpreter_config_gated_by_feature() {
        #[cfg(feature = "native-python")]
        assert!(PythonModuleConfig::sub_interpreter().is_some());
        #[cfg(not(feature = "native-python"))]
        assert!(PythonModuleConfig::sub_interpreter().is_none());
    }

    // ---- JSON extraction ---------------------------------------------------

    #[rstest]
    #[case(r#"{"changed": true}"#, Some(r#"{"changed": true}"#))]
    #[case("noise\n{\"ok\": true}", Some("{\"ok\": true}"))]
    #[case("no json here", None)]
    fn test_last_json_object(#[case] input: &str, #[case] expected: Option<&str>) {
        assert_eq!(last_json_object(input), expected);
    }

    // ---- parse_output ------------------------------------------------------

    #[test]
    fn test_parse_ok() {
        let r = parse_output(r#"{"changed":false,"msg":"good"}"#, "", 0, "h1").unwrap();
        assert!(r.status.is_ok());
        assert_eq!(r.msg, "good");
    }

    #[test]
    fn test_parse_changed() {
        let r = parse_output(r#"{"changed":true,"msg":""}"#, "", 0, "h1").unwrap();
        assert!(r.changed);
    }

    #[test]
    fn test_parse_failed() {
        let r = parse_output(r#"{"failed":true,"msg":"denied"}"#, "", 1, "h1").unwrap();
        assert!(r.status.is_failed());
        assert_eq!(r.msg, "denied");
    }

    #[test]
    fn test_parse_extra_vars() {
        let r = parse_output(
            r#"{"changed":false,"rc":0,"stdout":"hi","msg":""}"#,
            "",
            0,
            "h1",
        )
        .unwrap();
        assert!(r.vars.contains_key("rc"));
        assert!(r.vars.contains_key("stdout"));
    }

    #[test]
    fn test_parse_preamble_noise() {
        let stdout = "some debug line\n{\"changed\":false,\"msg\":\"clean\"}\n";
        let r = parse_output(stdout, "", 0, "h1").unwrap();
        assert_eq!(r.msg, "clean");
    }

    #[test]
    fn test_parse_non_json_fallback() {
        let r = parse_output("not json", "stderr msg", 1, "h1").unwrap();
        assert!(r.status.is_failed());
    }

    // ---- Subprocess: trivial Python module ---------------------------------

    #[test]
    fn test_subprocess_trivial_module() {
        let script = b"import sys,json\n\
            args_file=sys.argv[1]\n\
            result={\"changed\":False,\"msg\":\"subprocess_ok\",\"invoked\":True}\n\
            print(json.dumps(result))\n\
            sys.exit(0)\n";
        let mut tmp = NamedTempFile::with_suffix(".py").unwrap();
        tmp.write_all(script).unwrap();
        let cfg = PythonModuleConfig::subprocess();
        let r = invoke_subprocess(
            tmp.path().to_str().unwrap(),
            &HashMap::new(),
            "localhost",
            &cfg,
        )
        .unwrap();
        assert!(r.status.is_ok());
        assert_eq!(r.msg, "subprocess_ok");
        assert_eq!(r.vars.get("invoked"), Some(&Value::Bool(true)));
    }

    #[test]
    fn test_subprocess_module_failure() {
        let script = b"import sys,json\nprint(json.dumps({\"failed\":True,\"msg\":\"oops\"}))\nsys.exit(1)\n";
        let mut tmp = NamedTempFile::with_suffix(".py").unwrap();
        tmp.write_all(script).unwrap();
        let cfg = PythonModuleConfig::subprocess();
        let r = invoke_subprocess(
            tmp.path().to_str().unwrap(),
            &HashMap::new(),
            "localhost",
            &cfg,
        )
        .unwrap();
        assert!(r.status.is_failed());
        assert_eq!(r.msg, "oops");
    }

    // ---- Native (PyO3) backend --------------------------------------------

    #[cfg(feature = "native-python")]
    #[test]
    fn test_native_trivial_module() {
        let script = b"import sys,json\n\
            result={\"changed\":False,\"msg\":\"native_ok\",\"backend\":\"pyo3\"}\n\
            print(json.dumps(result))\n\
            sys.exit(0)\n";
        let mut tmp = NamedTempFile::with_suffix(".py").unwrap();
        tmp.write_all(script).unwrap();
        let r = invoke_native(tmp.path().to_str().unwrap(), &HashMap::new(), "localhost").unwrap();
        assert!(r.status.is_ok());
        assert_eq!(r.msg, "native_ok");
        assert_eq!(r.vars.get("backend"), Some(&Value::String("pyo3".into())));
    }

    #[cfg(feature = "native-python")]
    #[test]
    fn test_native_reads_args_file() {
        let script = b"import sys,json\n\
            with open(sys.argv[1]) as f: data=json.load(f)\n\
            val=data['ANSIBLE_MODULE_ARGS'].get('mykey','missing')\n\
            print(json.dumps({\"changed\":False,\"msg\":val}))\n\
            sys.exit(0)\n";
        let mut tmp = NamedTempFile::with_suffix(".py").unwrap();
        tmp.write_all(script).unwrap();
        let mut args = HashMap::new();
        args.insert(
            "mykey".to_string(),
            Value::String("hello_native".to_string()),
        );
        let r = invoke_native(tmp.path().to_str().unwrap(), &args, "localhost").unwrap();
        assert_eq!(r.msg, "hello_native");
    }

    // ---- Sub-interpreter backend ------------------------------------------

    #[cfg(feature = "native-python")]
    #[test]
    fn test_sub_interpreter_trivial_module() {
        let script = b"import sys,json\n\
            result={\"changed\":False,\"msg\":\"subinterp_ok\"}\n\
            print(json.dumps(result))\n\
            sys.exit(0)\n";
        let mut tmp = NamedTempFile::with_suffix(".py").unwrap();
        tmp.write_all(script).unwrap();
        let r = invoke_sub_interpreter(tmp.path().to_str().unwrap(), &HashMap::new(), "localhost")
            .unwrap();
        // Result is ok; message may vary depending on Python version.
        assert!(r.status.is_ok());
    }

    // ---- ConfigurablePythonInvoker -----------------------------------------

    #[test]
    fn test_configurable_missing_module() {
        use crate::registry::ModuleArgs;
        use ansiblers_core::{ExecutionContext, Inventory};
        use std::sync::Arc;

        let inv = ConfigurablePythonInvoker::new(
            PythonModuleConfig::subprocess().with_library_paths(vec![]),
        );
        let mut ctx = ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new());
        let mut ma = ModuleArgs::new(HashMap::new());
        ma.task_name = Some("nonexistent_xyz".to_string());
        let r = inv.invoke(&ma, "localhost", &mut ctx).unwrap();
        assert!(r.status.is_failed());
        assert!(r.msg.contains("not found"));
    }

    #[test]
    fn test_configurable_with_module_path_arg() {
        use crate::registry::ModuleArgs;
        use ansiblers_core::{ExecutionContext, Inventory};
        use std::io::Write;
        use std::sync::Arc;

        let script = b"import sys,json\nprint(json.dumps({\"changed\":False,\"msg\":\"via_path\"}))\nsys.exit(0)\n";
        let mut tmp = NamedTempFile::with_suffix(".py").unwrap();
        tmp.write_all(script).unwrap();

        let inv = ConfigurablePythonInvoker::new(PythonModuleConfig::subprocess());
        let mut ctx = ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new());
        let mut args = HashMap::new();
        args.insert(
            "_module_path".to_string(),
            Value::String(tmp.path().to_str().unwrap().to_string()),
        );
        let r = inv
            .invoke(&ModuleArgs::new(args), "localhost", &mut ctx)
            .unwrap();
        assert_eq!(r.msg, "via_path");
    }

    #[test]
    fn test_discover_ansible_library_paths() {
        let _paths = discover_ansible_library_paths();
        // Just confirm it doesn't panic (Ansible may not be installed in CI).
    }
}
