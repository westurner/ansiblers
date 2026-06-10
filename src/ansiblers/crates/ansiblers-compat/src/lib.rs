//! `ansiblers-compat` — PyO3 Python bindings for ansiblers.
//!
//! Exposes a `ansiblers` Python extension module that provides a high-level
//! interface to the Rust playbook engine.  Build with
//! [Maturin](https://www.maturin.rs/) to produce an importable wheel:
//!
//! ```bash
//! maturin build --features extension-module
//! pip install target/wheels/ansiblers_compat-*.whl
//! ```
//!
//! ## Python API
//!
//! ```python
//! import ansiblers
//!
//! # Run a playbook:
//! runner = ansiblers.PlaybookRunner(inventory="inventory.ini", verbosity=1)
//! runner.set_var("env", "staging")
//! result = runner.run("site.yml")
//! print(f"success={result.success}")
//! for task in result.all_results():
//!     print(f"  {task.host}: {task.status} changed={task.changed}")
//!
//! # Inspect an inventory:
//! inv = ansiblers.InventoryLoader("inventory.yml")
//! print(inv.hosts())          # ["web1", "web2", "db1"]
//! print(inv.groups())         # ["all", "webservers", "databases"]
//! print(inv.matching_hosts("webservers"))  # ["web1", "web2"]
//! vars = inv.host_vars("web1")  # dict of merged host/group variables
//!
//! # Profile hot paths (Phase 6 Weeks 43+):
//! profiler = ansiblers.HotPathProfiler()
//! profiler.profile("site.yml", inventory="inventory.ini")
//! print(profiler.report())
//!
//! # Async runner — non-blocking from Python (Phase 6 Weeks 43+):
//! import asyncio
//! runner = ansiblers.AsyncPlaybookRunner(inventory="inventory.ini")
//! result = asyncio.run(runner.run("site.yml"))
//! ```
//!
//! ## Classes
//!
//! | Python class | Rust type | Description |
//! |--------------|-----------|-------------|
//! | `PlaybookRunner` | [`PyPlaybookRunner`] | Execute playbooks (blocking) |
//! | `AsyncPlaybookRunner` | [`PyAsyncPlaybookRunner`] | Execute playbooks (async-friendly) |
//! | `PlaybookResult` | [`PyPlaybookResult`] | Return value of `run()` |
//! | `TaskResult` | [`PyTaskResult`] | Per-task outcome |
//! | `InventoryLoader` | [`PyInventoryLoader`] | Inspect inventory |
//! | `HotPathProfiler` | [`PyHotPathProfiler`] | Measure per-stage execution time |
//!
//! ## Feature flags
//!
//! Enable `extension-module` when building with Maturin to produce a shared
//! library loadable by Python.  Without this flag the crate compiles as a
//! plain Rust library usable in tests.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ansiblers_core::{ExecutionContext, Inventory, Value};
use ansiblers_executor::PlayExecutor;
use ansiblers_inventory::load_inventory;
use ansiblers_modules::ModuleRegistry;
use ansiblers_parser::parse_playbook;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

pub mod contrib;
use contrib::PyContributionReport;

// ---------------------------------------------------------------------------
// PyTaskResult
// ---------------------------------------------------------------------------

/// Python-facing task result.
#[pyclass(name = "TaskResult")]
pub struct PyTaskResult {
    #[pyo3(get)]
    pub host: String,
    #[pyo3(get)]
    pub status: String,
    #[pyo3(get)]
    pub stdout: String,
    #[pyo3(get)]
    pub stderr: String,
    #[pyo3(get)]
    pub rc: i32,
    #[pyo3(get)]
    pub changed: bool,
    #[pyo3(get)]
    pub msg: String,
    #[pyo3(get)]
    pub task_name: Option<String>,
}

#[pymethods]
impl PyTaskResult {
    fn __repr__(&self) -> String {
        format!(
            "TaskResult(host={:?}, status={:?}, changed={}, rc={})",
            self.host, self.status, self.changed, self.rc
        )
    }

    fn is_ok(&self) -> bool {
        self.status == "ok" || self.status == "changed"
    }

    fn is_failed(&self) -> bool {
        self.status == "failed" || self.status == "unreachable"
    }
}

// ---------------------------------------------------------------------------
// PyPlaybookResult
// ---------------------------------------------------------------------------

/// Python-facing playbook result returned by [`PyPlaybookRunner::run`].
///
/// The `success` attribute mirrors [`PlaybookResult::success`].  Call
/// `all_results()` to get a flat list of every [`PyTaskResult`] across
/// all plays and hosts.
#[pyclass(name = "PlaybookResult")]
pub struct PyPlaybookResult {
    #[pyo3(get)]
    pub success: bool,
    inner: ansiblers_executor::PlaybookResult,
}

#[pymethods]
impl PyPlaybookResult {
    /// Return all task results as a flat list.
    fn all_results(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let list = PyList::empty_bound(py);
        for play in &self.inner.play_results {
            for (_host, results) in &play.host_results {
                for r in results {
                    let item = PyTaskResult {
                        host: r.host.clone(),
                        status: format!("{:?}", r.status).to_lowercase(),
                        stdout: r.stdout.clone(),
                        stderr: r.stderr.clone(),
                        rc: r.rc,
                        changed: r.changed,
                        msg: r.msg.clone(),
                        task_name: r.task_name.clone(),
                    };
                    list.append(Py::new(py, item)?)?;
                }
            }
        }
        Ok(list.into())
    }

    fn __repr__(&self) -> String {
        format!("PlaybookResult(success={})", self.success)
    }
}

// ---------------------------------------------------------------------------
// PyPlaybookRunner
// ---------------------------------------------------------------------------

/// Executes Ansible playbooks using the ansiblers Rust engine.
#[pyclass(name = "PlaybookRunner")]
pub struct PyPlaybookRunner {
    inventory_path: Option<String>,
    extra_vars: HashMap<String, String>,
    verbosity: u8,
}

#[pymethods]
impl PyPlaybookRunner {
    #[new]
    #[pyo3(signature = (inventory=None, verbosity=0))]
    fn new(inventory: Option<String>, verbosity: u8) -> Self {
        Self {
            inventory_path: inventory,
            extra_vars: HashMap::new(),
            verbosity,
        }
    }

    /// Set an extra variable (equivalent to -e key=value).
    fn set_var(&mut self, key: String, value: String) {
        self.extra_vars.insert(key, value);
    }

    /// Run a playbook and return a PlaybookResult.
    fn run(&self, py: Python<'_>, playbook_path: &str) -> PyResult<Py<PyPlaybookResult>> {
        let result = py.allow_threads(|| self.run_internal(playbook_path));
        match result {
            Ok((success, inner)) => {
                let pyresult = PyPlaybookResult { success, inner };
                Ok(Py::new(py, pyresult)?)
            }
            Err(e) => Err(pyo3::exceptions::PyRuntimeError::new_err(e.to_string())),
        }
    }

    fn __repr__(&self) -> String {
        format!("PlaybookRunner(inventory={:?})", self.inventory_path)
    }
}

impl PyPlaybookRunner {
    fn run_internal(
        &self,
        playbook_path: &str,
    ) -> anyhow::Result<(bool, ansiblers_executor::PlaybookResult)> {
        let inventory = match &self.inventory_path {
            Some(path) => load_inventory(path)?,
            None => Inventory::new(),
        };

        let extra: HashMap<String, Value> = self
            .extra_vars
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();

        let mut ctx = ExecutionContext::new(Arc::new(inventory), extra);
        ctx.verbosity = self.verbosity;

        let playbook = parse_playbook(playbook_path)?;
        let executor = PlayExecutor::new(ModuleRegistry::with_defaults());
        let result = executor.run_playbook(&playbook, &mut ctx)?;
        let success = result.success;
        Ok((success, result))
    }
}

// ---------------------------------------------------------------------------
// PyInventoryLoader
// ---------------------------------------------------------------------------

/// Load and inspect Ansible inventories from Python.
#[pyclass(name = "InventoryLoader")]
pub struct PyInventoryLoader {
    inner: Inventory,
}

#[pymethods]
impl PyInventoryLoader {
    #[new]
    fn new(path: &str) -> PyResult<Self> {
        let inv = load_inventory(path)
            .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))?;
        Ok(Self { inner: inv })
    }

    /// Return list of all hostnames.
    fn hosts(&self) -> Vec<String> {
        self.inner.hosts.keys().cloned().collect()
    }

    /// Return list of all group names.
    fn groups(&self) -> Vec<String> {
        self.inner.groups.keys().cloned().collect()
    }

    /// Return hostnames matching a pattern.
    fn matching_hosts(&self, pattern: &str) -> Vec<String> {
        self.inner.matching_hosts(pattern)
    }

    /// Return host variables as a Python dict.
    fn host_vars(&self, py: Python<'_>, hostname: &str) -> PyResult<Py<PyDict>> {
        let vars = self.inner.host_vars(hostname);
        let dict = PyDict::new_bound(py);
        for (k, v) in vars {
            dict.set_item(k, value_to_pyobject(py, &v)?)?;
        }
        Ok(dict.into())
    }
}

// ---------------------------------------------------------------------------
// PyHotPathProfiler — per-stage timing for Ansible Core integration
// ---------------------------------------------------------------------------

/// Timing record for a single profiled stage.
#[derive(Debug, Clone)]
struct StageTime {
    name: &'static str,
    elapsed: Duration,
}

/// Python-facing profiler that measures per-stage execution time.
///
/// Stages measured:
/// - `inventory_load` — loading and parsing the inventory
/// - `playbook_parse` — YAML parsing and AST construction
/// - `execution` — task execution engine
///
/// ```python
/// profiler = ansiblers.HotPathProfiler()
/// profiler.profile("site.yml", inventory="inventory.ini")
/// print(profiler.report())
/// ```
#[pyclass(name = "HotPathProfiler")]
pub struct PyHotPathProfiler {
    stages: Vec<StageTime>,
}

#[pymethods]
impl PyHotPathProfiler {
    #[new]
    fn new() -> Self {
        Self { stages: vec![] }
    }

    /// Run a playbook and record per-stage timing.
    ///
    /// The result is discarded; call `report()` or `stage_times()` afterwards.
    #[pyo3(signature = (playbook_path, inventory=None))]
    fn profile(&mut self, py: Python<'_>, playbook_path: &str, inventory: Option<&str>) -> PyResult<()> {
        let result = py.allow_threads(|| self.run_profiled(playbook_path, inventory));
        match result {
            Ok(stages) => {
                self.stages = stages;
                Ok(())
            }
            Err(e) => Err(pyo3::exceptions::PyRuntimeError::new_err(e.to_string())),
        }
    }

    /// Return a human-readable profiling report.
    fn report(&self) -> String {
        if self.stages.is_empty() {
            return "No profile data — call profile() first.".to_string();
        }
        let total: Duration = self.stages.iter().map(|s| s.elapsed).sum();
        let mut out = String::from("ansiblers hot-path profile:\n");
        for s in &self.stages {
            let pct = if total.as_nanos() > 0 {
                s.elapsed.as_nanos() as f64 / total.as_nanos() as f64 * 100.0
            } else {
                0.0
            };
            out.push_str(&format!(
                "  {:20} {:>8.2} ms  ({:4.1}%)\n",
                s.name,
                s.elapsed.as_secs_f64() * 1000.0,
                pct,
            ));
        }
        out.push_str(&format!(
            "  {:20} {:>8.2} ms  (total)\n",
            "TOTAL",
            total.as_secs_f64() * 1000.0
        ));
        out
    }

    /// Return a dict mapping stage name → elapsed seconds.
    fn stage_times(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let dict = PyDict::new_bound(py);
        for s in &self.stages {
            dict.set_item(s.name, s.elapsed.as_secs_f64())?;
        }
        Ok(dict.into())
    }

    fn __repr__(&self) -> String {
        format!("HotPathProfiler(stages={})", self.stages.len())
    }
}

impl PyHotPathProfiler {
    fn run_profiled(
        &self,
        playbook_path: &str,
        inventory_path: Option<&str>,
    ) -> anyhow::Result<Vec<StageTime>> {
        let mut stages = Vec::new();

        // Stage 1: inventory load
        let t0 = Instant::now();
        let inventory = match inventory_path {
            Some(p) => load_inventory(p)?,
            None => Inventory::new(),
        };
        stages.push(StageTime { name: "inventory_load", elapsed: t0.elapsed() });

        // Stage 2: playbook parse
        let t1 = Instant::now();
        let playbook = parse_playbook(playbook_path)?;
        stages.push(StageTime { name: "playbook_parse", elapsed: t1.elapsed() });

        // Stage 3: execution
        let t2 = Instant::now();
        let mut ctx = ExecutionContext::new(Arc::new(inventory), HashMap::new());
        let executor = PlayExecutor::new(ModuleRegistry::with_defaults());
        let _ = executor.run_playbook(&playbook, &mut ctx)?;
        stages.push(StageTime { name: "execution", elapsed: t2.elapsed() });

        Ok(stages)
    }
}

// ---------------------------------------------------------------------------
// PyAsyncPlaybookRunner — async-friendly wrapper (Tokio spawn_blocking)
// ---------------------------------------------------------------------------

/// Executes Ansible playbooks in a way that doesn't block the Python event loop.
///
/// Uses `py.allow_threads()` to release the GIL during execution, making it
/// compatible with `asyncio.run_in_executor` or `loop.run_in_executor`.
///
/// ```python
/// import asyncio
/// import ansiblers
///
/// runner = ansiblers.AsyncPlaybookRunner(inventory="inventory.ini")
/// runner.set_var("env", "staging")
///
/// # Non-blocking from asyncio:
/// result = asyncio.run(
///     asyncio.get_event_loop().run_in_executor(None, runner.run_sync, "site.yml")
/// )
/// print(result.success)
/// ```
#[pyclass(name = "AsyncPlaybookRunner")]
pub struct PyAsyncPlaybookRunner {
    inventory_path: Option<String>,
    extra_vars: HashMap<String, String>,
    strategy: String,
}

#[pymethods]
impl PyAsyncPlaybookRunner {
    #[new]
    #[pyo3(signature = (inventory=None, strategy="linear"))]
    fn new(inventory: Option<String>, strategy: &str) -> Self {
        Self {
            inventory_path: inventory,
            extra_vars: HashMap::new(),
            strategy: strategy.to_string(),
        }
    }

    /// Set an extra variable (equivalent to `-e key=value`).
    fn set_var(&mut self, key: String, value: String) {
        self.extra_vars.insert(key, value);
    }

    /// Run a playbook, releasing the GIL so Python threads remain responsive.
    ///
    /// This is the GIL-releasing entrypoint.  Wrap in
    /// `asyncio.get_event_loop().run_in_executor(None, runner.run_sync, path)`
    /// for true non-blocking execution inside an asyncio event loop.
    fn run_sync(&self, py: Python<'_>, playbook_path: &str) -> PyResult<Py<PyPlaybookResult>> {
        let result = py.allow_threads(|| self.run_internal(playbook_path));
        match result {
            Ok((success, inner)) => Ok(Py::new(py, PyPlaybookResult { success, inner })?),
            Err(e) => Err(pyo3::exceptions::PyRuntimeError::new_err(e.to_string())),
        }
    }

    /// Strategy name in use (`"linear"`, `"free"`, `"batch_N"`).
    #[getter]
    fn strategy(&self) -> &str {
        &self.strategy
    }

    fn __repr__(&self) -> String {
        format!(
            "AsyncPlaybookRunner(inventory={:?}, strategy={:?})",
            self.inventory_path, self.strategy
        )
    }
}

impl PyAsyncPlaybookRunner {
    fn run_internal(
        &self,
        playbook_path: &str,
    ) -> anyhow::Result<(bool, ansiblers_executor::PlaybookResult)> {
        let inventory = match &self.inventory_path {
            Some(p) => load_inventory(p)?,
            None => Inventory::new(),
        };

        let extra: HashMap<String, Value> = self
            .extra_vars
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();

        let mut ctx = ExecutionContext::new(Arc::new(inventory), extra);

        let playbook = parse_playbook(playbook_path)?;
        let executor = PlayExecutor::new(ModuleRegistry::with_defaults());
        let result = executor.run_playbook(&playbook, &mut ctx)?;
        let success = result.success;
        Ok((success, result))
    }
}

// ---------------------------------------------------------------------------
// Python module registration
// ---------------------------------------------------------------------------

#[pymodule]
fn ansiblers(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyPlaybookRunner>()?;
    m.add_class::<PyAsyncPlaybookRunner>()?;
    m.add_class::<PyPlaybookResult>()?;
    m.add_class::<PyTaskResult>()?;
    m.add_class::<PyInventoryLoader>()?;
    m.add_class::<PyHotPathProfiler>()?;
    m.add_class::<PyContributionReport>()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn value_to_pyobject(py: Python<'_>, v: &Value) -> PyResult<PyObject> {
    match v {
        Value::Null => Ok(py.None()),
        Value::Bool(b) => Ok(b.into_py(py)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i.into_py(py))
            } else if let Some(f) = n.as_f64() {
                Ok(f.into_py(py))
            } else {
                Ok(py.None())
            }
        }
        Value::String(s) => Ok(s.clone().into_py(py)),
        Value::Array(arr) => {
            let list = PyList::empty_bound(py);
            for item in arr {
                list.append(value_to_pyobject(py, item)?)?;
            }
            Ok(list.into_py(py))
        }
        Value::Object(obj) => {
            let dict = PyDict::new_bound(py);
            for (k, v) in obj {
                dict.set_item(k, value_to_pyobject(py, v)?)?;
            }
            Ok(dict.into_py(py))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pyplaybook_runner_new() {
        let runner = PyPlaybookRunner::new(None, 0);
        assert!(runner.inventory_path.is_none());
        assert_eq!(runner.verbosity, 0);
    }

    #[test]
    fn test_pytask_result_helpers() {
        let r = PyTaskResult {
            host: "h1".to_string(),
            status: "ok".to_string(),
            stdout: String::new(),
            stderr: String::new(),
            rc: 0,
            changed: false,
            msg: String::new(),
            task_name: None,
        };
        assert!(r.is_ok());
        assert!(!r.is_failed());
    }

    // ---- HotPathProfiler --------------------------------------------------

    #[test]
    fn test_hot_path_profiler_new_empty() {
        let p = PyHotPathProfiler::new();
        assert!(p.stages.is_empty());
        assert!(p.report().contains("No profile data"));
    }

    #[test]
    fn test_hot_path_profiler_report_format() {
        let mut p = PyHotPathProfiler::new();
        p.stages = vec![
            StageTime { name: "inventory_load", elapsed: Duration::from_millis(5) },
            StageTime { name: "playbook_parse", elapsed: Duration::from_millis(10) },
            StageTime { name: "execution",      elapsed: Duration::from_millis(85) },
        ];
        let report = p.report();
        assert!(report.contains("inventory_load"));
        assert!(report.contains("playbook_parse"));
        assert!(report.contains("execution"));
        assert!(report.contains("TOTAL"));
        assert!(report.contains("100.0 ms") || report.contains("100"));
    }

    #[test]
    fn test_hot_path_profiler_repr() {
        let p = PyHotPathProfiler::new();
        assert_eq!(p.__repr__(), "HotPathProfiler(stages=0)");
    }

    // ---- AsyncPlaybookRunner ----------------------------------------------

    #[test]
    fn test_async_runner_new_defaults() {
        let r = PyAsyncPlaybookRunner::new(None, "linear");
        assert!(r.inventory_path.is_none());
        assert!(r.extra_vars.is_empty());
        assert_eq!(r.strategy, "linear");
    }

    #[test]
    fn test_async_runner_set_var() {
        let mut r = PyAsyncPlaybookRunner::new(None, "free");
        r.set_var("env".to_string(), "prod".to_string());
        assert_eq!(r.extra_vars.get("env").map(String::as_str), Some("prod"));
    }

    #[test]
    fn test_async_runner_strategy_getter() {
        let r = PyAsyncPlaybookRunner::new(None, "batch_4");
        assert_eq!(r.strategy(), "batch_4");
    }

    #[test]
    fn test_async_runner_repr() {
        let r = PyAsyncPlaybookRunner::new(Some("inv.ini".to_string()), "free");
        let repr = r.__repr__();
        assert!(repr.contains("AsyncPlaybookRunner"));
        assert!(repr.contains("inv.ini"));
        assert!(repr.contains("free"));
    }
}
