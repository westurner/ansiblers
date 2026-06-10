use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::Value;

/// The outcome status of a single task execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Ok,
    Changed,
    Failed,
    Skipped,
    Unreachable,
}

impl TaskStatus {
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok | Self::Changed)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed | Self::Unreachable)
    }
}

/// Result returned by a module after executing a single task on one host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub host: String,
    pub status: TaskStatus,
    pub stdout: String,
    pub stderr: String,
    pub rc: i32,
    pub changed: bool,
    pub msg: String,
    /// Extra variables to merge into the registered var (e.g. stdout_lines).
    pub vars: HashMap<String, Value>,
    pub task_name: Option<String>,
}

impl TaskResult {
    pub fn ok(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            status: TaskStatus::Ok,
            stdout: String::new(),
            stderr: String::new(),
            rc: 0,
            changed: false,
            msg: String::new(),
            vars: HashMap::new(),
            task_name: None,
        }
    }

    pub fn changed(host: impl Into<String>) -> Self {
        let mut r = Self::ok(host);
        r.status = TaskStatus::Changed;
        r.changed = true;
        r
    }

    pub fn failed(host: impl Into<String>, msg: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            status: TaskStatus::Failed,
            stdout: String::new(),
            stderr: String::new(),
            rc: 1,
            changed: false,
            msg: msg.into(),
            vars: HashMap::new(),
            task_name: None,
        }
    }

    pub fn skipped(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            status: TaskStatus::Skipped,
            stdout: String::new(),
            stderr: String::new(),
            rc: 0,
            changed: false,
            msg: String::new(),
            vars: HashMap::new(),
            task_name: None,
        }
    }

    /// Build a Value suitable for storing via `register:`.
    pub fn as_register_value(&self) -> Value {
        let stdout_lines: Vec<Value> = self
            .stdout
            .lines()
            .map(|l| Value::String(l.to_string()))
            .collect();
        let stderr_lines: Vec<Value> = self
            .stderr
            .lines()
            .map(|l| Value::String(l.to_string()))
            .collect();
        let mut map = serde_json::Map::new();
        map.insert("stdout".to_string(), Value::String(self.stdout.clone()));
        map.insert("stderr".to_string(), Value::String(self.stderr.clone()));
        map.insert("stdout_lines".to_string(), Value::Array(stdout_lines));
        map.insert("stderr_lines".to_string(), Value::Array(stderr_lines));
        map.insert("rc".to_string(), Value::Number(self.rc.into()));
        map.insert("changed".to_string(), Value::Bool(self.changed));
        map.insert("failed".to_string(), Value::Bool(self.status.is_failed()));
        map.insert("msg".to_string(), Value::String(self.msg.clone()));
        // Merge extra module vars
        for (k, v) in &self.vars {
            map.insert(k.clone(), v.clone());
        }
        Value::Object(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ok_result() {
        let r = TaskResult::ok("host1");
        assert_eq!(r.status, TaskStatus::Ok);
        assert!(!r.changed);
        assert_eq!(r.rc, 0);
        assert!(r.status.is_ok());
    }

    #[test]
    fn test_changed_result() {
        let r = TaskResult::changed("host1");
        assert_eq!(r.status, TaskStatus::Changed);
        assert!(r.changed);
        assert!(r.status.is_ok());
    }

    #[test]
    fn test_failed_result() {
        let r = TaskResult::failed("host1", "boom");
        assert_eq!(r.status, TaskStatus::Failed);
        assert!(r.status.is_failed());
        assert_eq!(r.msg, "boom");
    }

    #[test]
    fn test_as_register_value_stdout_lines() {
        let mut r = TaskResult::ok("host1");
        r.stdout = "line1\nline2".to_string();
        let val = r.as_register_value();
        let lines = val["stdout_lines"].as_array().unwrap();
        assert_eq!(lines.len(), 2);
    }
}
