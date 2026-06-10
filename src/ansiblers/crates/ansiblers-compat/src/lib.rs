//! ansiblers-compat — PyO3 bindings exposing ansiblers to Python.
//!
//! Provides a `ansiblers` Python module with:
//! - `PlaybookRunner`: execute playbooks from Python.
//! - `InventoryLoader`: load INI/YAML inventories from Python.
//! - `TaskResult`: returned from `run_playbook()`.
//!
//! # Usage (Python)
//! ```python
//! import ansiblers
//!
//! runner = ansiblers.PlaybookRunner(inventory="inventory.ini")
//! result = runner.run("site.yml")
//! print(f"success={result.success}")
//! ```

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use std::collections::HashMap;
use std::sync::Arc;

use ansiblers_core::{ExecutionContext, Inventory, Value};
use ansiblers_executor::PlayExecutor;
use ansiblers_inventory::load_inventory;
use ansiblers_modules::ModuleRegistry;
use ansiblers_parser::parse_playbook;

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
// Python module registration
// ---------------------------------------------------------------------------

#[pymodule]
fn ansiblers(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyPlaybookRunner>()?;
    m.add_class::<PyPlaybookResult>()?;
    m.add_class::<PyTaskResult>()?;
    m.add_class::<PyInventoryLoader>()?;
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
}
