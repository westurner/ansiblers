//! Dynamic inventory — execute external scripts and parse their JSON output.
//!
//! Ansible supports *dynamic inventory scripts*: executable files that print
//! a JSON inventory when called with `--list` (all hosts) or `--host <name>`
//! (single host vars).
//!
//! ## JSON `--list` format
//!
//! ```json
//! {
//!   "_meta": {
//!     "hostvars": {
//!       "web1": { "ansible_host": "10.0.0.1" }
//!     }
//!   },
//!   "webservers": {
//!     "hosts": ["web1", "web2"],
//!     "vars": { "http_port": 80 }
//!   },
//!   "databases": ["db1", "db2"]
//! }
//! ```
//!
//! ## Usage
//!
//! ```rust,no_run
//! use ansiblers_inventory::dynamic::DynamicInventoryScript;
//!
//! let script = DynamicInventoryScript::new("./inventory.py");
//! let inventory = script.load().unwrap();
//! ```

use std::process::Command;

use ansiblers_core::{Group, Host, Inventory, Value};
use anyhow::{Context, Result};

/// Loads inventory from an external executable (dynamic inventory script).
///
/// Calls the script with `--list` and parses the JSON output according to
/// the Ansible dynamic inventory format.
pub struct DynamicInventoryScript {
    /// Path to the executable inventory script.
    pub script_path: String,
    /// Additional arguments passed before `--list` / `--host`.
    pub extra_args: Vec<String>,
}

impl DynamicInventoryScript {
    pub fn new(script_path: impl Into<String>) -> Self {
        Self {
            script_path: script_path.into(),
            extra_args: Vec::new(),
        }
    }

    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.extra_args = args;
        self
    }

    /// Execute `<script> --list` and parse the returned JSON into an [`Inventory`].
    pub fn load(&self) -> Result<Inventory> {
        let output = Command::new(&self.script_path)
            .args(&self.extra_args)
            .arg("--list")
            .output()
            .with_context(|| {
                format!("executing dynamic inventory script '{}'", self.script_path)
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "dynamic inventory script '{}' exited with {}: {}",
                self.script_path,
                output.status,
                stderr.trim()
            );
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_list_output(&stdout)
            .with_context(|| format!("parsing --list output from '{}'", self.script_path))
    }

    /// Execute `<script> --host <hostname>` and return the host variables.
    pub fn host_vars(&self, hostname: &str) -> Result<std::collections::HashMap<String, Value>> {
        let output = Command::new(&self.script_path)
            .args(&self.extra_args)
            .args(["--host", hostname])
            .output()
            .with_context(|| {
                format!(
                    "executing dynamic inventory script '{}' --host {}",
                    self.script_path, hostname
                )
            })?;

        if !output.status.success() {
            return Ok(std::collections::HashMap::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: serde_json::Value =
            serde_json::from_str(stdout.trim()).context("parsing --host JSON output")?;

        match json.as_object() {
            Some(obj) => Ok(obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
            None => Ok(std::collections::HashMap::new()),
        }
    }
}

// ---------------------------------------------------------------------------
// JSON parsing
// ---------------------------------------------------------------------------

/// Parse the JSON output of `<script> --list` into an [`Inventory`].
pub fn parse_list_output(json_str: &str) -> Result<Inventory> {
    let json: serde_json::Value =
        serde_json::from_str(json_str.trim()).context("parsing dynamic inventory JSON")?;

    let obj = json
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("dynamic inventory --list must return a JSON object"))?;

    let mut inventory = Inventory::new();

    // Extract _meta.hostvars first.
    let hostvars: std::collections::HashMap<String, serde_json::Value> = obj
        .get("_meta")
        .and_then(|m| m.get("hostvars"))
        .and_then(|hv| hv.as_object())
        .map(|hv| hv.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();

    for (key, value) in obj {
        if key == "_meta" {
            continue;
        }

        let group_name = key.clone();

        match value {
            // Compact form: "groupname": ["host1", "host2"]
            serde_json::Value::Array(hosts_arr) => {
                let host_names: Vec<String> = hosts_arr
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect();
                for h in &host_names {
                    ensure_host(&mut inventory, h, &hostvars);
                    inventory
                        .hosts
                        .get_mut(h)
                        .unwrap()
                        .groups
                        .push(group_name.clone());
                }
                let mut group = Group::new(&group_name);
                group.hosts = host_names;
                inventory.groups.insert(group_name, group);
            }
            // Full form: { "hosts": [...], "vars": {...}, "children": [...] }
            serde_json::Value::Object(group_obj) => {
                let hosts: Vec<String> = group_obj
                    .get("hosts")
                    .and_then(|h| h.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();

                for h in &hosts {
                    ensure_host(&mut inventory, h, &hostvars);
                    inventory
                        .hosts
                        .get_mut(h)
                        .unwrap()
                        .groups
                        .push(group_name.clone());
                }

                let vars: std::collections::HashMap<String, Value> = group_obj
                    .get("vars")
                    .and_then(|v| v.as_object())
                    .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default();

                let children: Vec<String> = group_obj
                    .get("children")
                    .and_then(|c| c.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();

                let mut group = Group::new(&group_name);
                group.hosts = hosts;
                group.vars = vars;
                group.children = children;
                inventory.groups.insert(group_name, group);
            }
            _ => {}
        }
    }

    Ok(inventory)
}

fn ensure_host(
    inventory: &mut Inventory,
    name: &str,
    hostvars: &std::collections::HashMap<String, serde_json::Value>,
) {
    if inventory.hosts.contains_key(name) {
        return;
    }
    let mut host = Host::new(name);
    if let Some(hv) = hostvars.get(name) {
        if let Some(obj) = hv.as_object() {
            for (k, v) in obj {
                host.vars.insert(k.clone(), v.clone());
            }
        }
    }
    inventory.hosts.insert(name.to_string(), host);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE_LIST: &str = r#"{
        "_meta": {
            "hostvars": {
                "web1": { "ansible_host": "10.0.0.1" },
                "db1": { "ansible_host": "10.0.1.1" }
            }
        },
        "webservers": {
            "hosts": ["web1", "web2"],
            "vars": { "http_port": 80 }
        },
        "databases": ["db1"],
        "all": {
            "children": ["webservers", "databases"]
        }
    }"#;

    #[test]
    fn test_parse_list_output() {
        let inv = parse_list_output(SIMPLE_LIST).unwrap();
        assert!(inv.hosts.contains_key("web1"));
        assert!(inv.hosts.contains_key("db1"));
        assert!(inv.groups.contains_key("webservers"));
    }

    #[test]
    fn test_hostvars_from_meta() {
        let inv = parse_list_output(SIMPLE_LIST).unwrap();
        let web1 = inv.get_host("web1").unwrap();
        assert_eq!(
            web1.vars.get("ansible_host"),
            Some(&Value::String("10.0.0.1".into()))
        );
    }

    #[test]
    fn test_group_vars_parsed() {
        let inv = parse_list_output(SIMPLE_LIST).unwrap();
        let ws = inv.get_group("webservers").unwrap();
        assert_eq!(ws.vars.get("http_port"), Some(&Value::Number(80.into())));
    }

    #[test]
    fn test_compact_group_form() {
        let inv = parse_list_output(SIMPLE_LIST).unwrap();
        let db = inv.get_group("databases").unwrap();
        assert!(db.hosts.contains(&"db1".to_string()));
    }

    #[test]
    fn test_children_parsed() {
        let inv = parse_list_output(SIMPLE_LIST).unwrap();
        let all = inv.get_group("all").unwrap();
        assert!(all.children.contains(&"webservers".to_string()));
        assert!(all.children.contains(&"databases".to_string()));
    }

    #[test]
    fn test_host_group_membership() {
        let inv = parse_list_output(SIMPLE_LIST).unwrap();
        let web1 = inv.get_host("web1").unwrap();
        assert!(web1.groups.contains(&"webservers".to_string()));
    }
}
