//! `group_vars` and `host_vars` directory loader.
//!
//! Ansible supports variable files alongside the inventory in two layouts:
//!
//! ## Layout 1: adjacent to the inventory file
//! ```text
//! inventory.ini
//! group_vars/
//!   all.yml
//!   webservers.yml
//!   databases/
//!     main.yml
//!     extra.yml
//! host_vars/
//!   web1.example.com.yml
//!   web1.example.com/
//!     main.yml
//! ```
//!
//! ## Layout 2: adjacent to the playbook
//! ```text
//! site.yml
//! group_vars/all.yml
//! host_vars/web1.yml
//! ```
//!
//! This module merges variables from all found files into the [`Inventory`],
//! respecting precedence: `group_vars/all` < `group_vars/<group>` < `host_vars/<host>`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ansiblers_core::{Inventory, Value};
use serde_yaml::Value as YamlValue;

/// Merge `group_vars` and `host_vars` directories into an existing inventory.
///
/// `search_paths` is a list of directories to search for `group_vars/` and
/// `host_vars/` subdirectories (typically the inventory directory and the
/// playbook directory).
pub fn merge_group_and_host_vars(
    inventory: &mut Inventory,
    search_paths: &[&Path],
) -> Result<()> {
    for &base in search_paths {
        let group_vars_dir = base.join("group_vars");
        let host_vars_dir = base.join("host_vars");

        if group_vars_dir.is_dir() {
            load_group_vars(inventory, &group_vars_dir)
                .with_context(|| format!("loading group_vars from {}", group_vars_dir.display()))?;
        }
        if host_vars_dir.is_dir() {
            load_host_vars(inventory, &host_vars_dir)
                .with_context(|| format!("loading host_vars from {}", host_vars_dir.display()))?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// group_vars
// ---------------------------------------------------------------------------

/// Load all group variable files from a `group_vars/` directory.
///
/// Each file (or subdirectory) maps to a group name:
/// - `group_vars/all.yml` → `all` group
/// - `group_vars/webservers.yml` → `webservers` group
/// - `group_vars/databases/main.yml` → `databases` group (split files)
fn load_group_vars(inventory: &mut Inventory, group_vars_dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(group_vars_dir)? {
        let entry = entry?;
        let path = entry.path();

        let group_name = extract_name(&path)?;

        let vars = if path.is_dir() {
            load_vars_from_dir(&path)?
        } else if is_yaml(&path) {
            load_vars_file(&path)?
        } else {
            continue;
        };

        if !vars.is_empty() {
            let group = inventory
                .groups
                .entry(group_name.clone())
                .or_insert_with(|| ansiblers_core::Group::new(&group_name));
            for (k, v) in vars {
                group.vars.entry(k).or_insert(v);
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// host_vars
// ---------------------------------------------------------------------------

/// Load all host variable files from a `host_vars/` directory.
///
/// - `host_vars/web1.yml` → host `web1`
/// - `host_vars/web1.example.com.yml` → host `web1.example.com`
/// - `host_vars/web1/main.yml` + `host_vars/web1/extra.yml` → merged into `web1`
fn load_host_vars(inventory: &mut Inventory, host_vars_dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(host_vars_dir)? {
        let entry = entry?;
        let path = entry.path();

        let host_name = extract_name(&path)?;

        let vars = if path.is_dir() {
            load_vars_from_dir(&path)?
        } else if is_yaml(&path) {
            load_vars_file(&path)?
        } else {
            continue;
        };

        if !vars.is_empty() {
            let host = inventory
                .hosts
                .entry(host_name.clone())
                .or_insert_with(|| ansiblers_core::Host::new(&host_name));
            for (k, v) in vars {
                // host_vars take precedence; overwrite group/inventory vars.
                host.vars.insert(k, v);
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Load and merge all YAML files from a directory (split-file support).
fn load_vars_from_dir(dir: &Path) -> Result<HashMap<String, Value>> {
    let mut merged = HashMap::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| is_yaml(p))
        .collect();
    entries.sort(); // deterministic merge order
    for path in entries {
        let vars = load_vars_file(&path)?;
        merged.extend(vars);
    }
    Ok(merged)
}

/// Load a single YAML vars file → `HashMap<String, Value>`.
pub fn load_vars_file(path: &Path) -> Result<HashMap<String, Value>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading vars file {}", path.display()))?;

    if content.trim().is_empty() {
        return Ok(HashMap::new());
    }

    let yaml: YamlValue = serde_yaml::from_str(&content)
        .with_context(|| format!("parsing vars file {}", path.display()))?;

    let mapping = match yaml {
        YamlValue::Mapping(m) => m,
        YamlValue::Null => return Ok(HashMap::new()),
        other => {
            anyhow::bail!(
                "vars file {} must be a YAML mapping, got: {}",
                path.display(),
                serde_yaml::to_string(&other).unwrap_or_default().trim()
            );
        }
    };

    mapping
        .into_iter()
        .map(|(k, v)| {
            let key = k
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("non-string key in {}", path.display()))?
                .to_string();
            let val = yaml_to_json(&v)?;
            Ok((key, val))
        })
        .collect()
}

/// Strip extension(s) from a path's filename to get the Ansible name.
/// `group_vars/webservers.yml` → `webservers`
/// `group_vars/databases/` → `databases`
fn extract_name(path: &Path) -> Result<String> {
    let stem = if path.is_dir() {
        path.file_name()
    } else {
        path.file_stem()
    };
    // If the stem still ends in .yml (e.g. foo.yml.yml), strip again.
    let raw = stem
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid path: {}", path.display()))?
        .to_string();
    // Handle double extensions like `host.yml` where stem = `host`
    Ok(raw.trim_end_matches(".yml").to_string())
}

fn is_yaml(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("yml") | Some("yaml")
    )
}

fn yaml_to_json(val: &YamlValue) -> Result<Value> {
    let s = serde_json::to_string(
        &serde_yaml::from_value::<serde_json::Value>(val.clone())
            .context("converting yaml to json")?,
    )
    .context("serialising to json")?;
    serde_json::from_str(&s).context("parsing json")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ansiblers_core::{Group, Host, Inventory};
    use rstest::rstest;
    use tempfile::TempDir;

    fn write(dir: &Path, file: &str, content: &str) {
        let path = dir.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn test_group_vars_all() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "group_vars/all.yml", "env: test\nregion: us-east\n");
        let mut inv = Inventory::new();
        merge_group_and_host_vars(&mut inv, &[tmp.path()]).unwrap();
        let all = inv.groups.get("all").unwrap();
        assert_eq!(all.vars.get("env"), Some(&Value::String("test".into())));
        assert_eq!(all.vars.get("region"), Some(&Value::String("us-east".into())));
    }

    #[test]
    fn test_group_vars_specific_group() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "group_vars/webservers.yml", "http_port: 80\n");
        let mut inv = Inventory::new();
        inv.groups.insert("webservers".into(), Group::new("webservers"));
        merge_group_and_host_vars(&mut inv, &[tmp.path()]).unwrap();
        let g = inv.groups.get("webservers").unwrap();
        assert_eq!(g.vars.get("http_port"), Some(&Value::Number(80.into())));
    }

    #[test]
    fn test_host_vars_simple() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "host_vars/web1.yml", "ansible_user: deploy\n");
        let mut inv = Inventory::new();
        inv.hosts.insert("web1".into(), Host::new("web1"));
        merge_group_and_host_vars(&mut inv, &[tmp.path()]).unwrap();
        let h = inv.hosts.get("web1").unwrap();
        assert_eq!(h.vars.get("ansible_user"), Some(&Value::String("deploy".into())));
    }

    #[test]
    fn test_host_vars_split_dir() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "host_vars/web1/main.yml", "pkg_version: 1.2.3\n");
        write(tmp.path(), "host_vars/web1/extra.yml", "debug_mode: false\n");
        let mut inv = Inventory::new();
        inv.hosts.insert("web1".into(), Host::new("web1"));
        merge_group_and_host_vars(&mut inv, &[tmp.path()]).unwrap();
        let h = inv.hosts.get("web1").unwrap();
        assert_eq!(h.vars.get("pkg_version"), Some(&Value::String("1.2.3".into())));
        assert_eq!(h.vars.get("debug_mode"), Some(&Value::Bool(false)));
    }

    #[test]
    fn test_group_vars_split_dir() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "group_vars/databases/main.yml", "db_port: 5432\n");
        write(tmp.path(), "group_vars/databases/replication.yml", "replica_count: 2\n");
        let mut inv = Inventory::new();
        merge_group_and_host_vars(&mut inv, &[tmp.path()]).unwrap();
        let g = inv.groups.get("databases").unwrap();
        assert_eq!(g.vars.get("db_port"), Some(&Value::Number(5432.into())));
        assert_eq!(g.vars.get("replica_count"), Some(&Value::Number(2.into())));
    }

    #[test]
    fn test_multiple_search_paths() {
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();
        write(tmp1.path(), "group_vars/all.yml", "from_inv: true\n");
        write(tmp2.path(), "group_vars/all.yml", "from_playbook: true\n");
        let mut inv = Inventory::new();
        merge_group_and_host_vars(&mut inv, &[tmp1.path(), tmp2.path()]).unwrap();
        let all = inv.groups.get("all").unwrap();
        // First path wins for duplicate keys (inventory > playbook dir).
        assert_eq!(all.vars.get("from_inv"), Some(&Value::Bool(true)));
        assert_eq!(all.vars.get("from_playbook"), Some(&Value::Bool(true)));
    }

    #[rstest]
    #[case("webservers.yml", "webservers")]
    #[case("databases.yaml", "databases")]
    #[case("all.yml", "all")]
    fn test_extract_name(#[case] filename: &str, #[case] expected: &str) {
        let path = Path::new(filename);
        assert_eq!(extract_name(path).unwrap(), expected);
    }

    #[test]
    fn test_empty_vars_file() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "group_vars/empty.yml", "");
        let mut inv = Inventory::new();
        // Should not panic or error
        merge_group_and_host_vars(&mut inv, &[tmp.path()]).unwrap();
    }
}
