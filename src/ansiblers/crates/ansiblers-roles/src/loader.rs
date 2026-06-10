//! Role loader — discovers and loads roles from the filesystem.
//!
//! Searches [`RolePath`] directories in order, loading task files, handlers,
//! defaults, vars, and metadata from the standard Ansible role layout.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ansiblers_core::Value;
use ansiblers_parser;
use anyhow::{Context, Result};
use serde_yaml::Value as YamlValue;

use crate::meta::RoleMeta;
use crate::role::Role;

/// Ordered list of directories to search for role definitions.
#[derive(Debug, Clone)]
pub struct RolePath {
    pub dirs: Vec<PathBuf>,
}

impl RolePath {
    /// Create a `RolePath` from an explicit list of directories.
    pub fn new(dirs: Vec<PathBuf>) -> Self {
        Self { dirs }
    }

    /// Build a default role path mirroring Ansible's search order:
    /// 1. `<playbook_dir>/roles`
    /// 2. `~/.ansible/roles`
    /// 3. `/etc/ansible/roles`
    /// 4. Paths from `ANSIBLE_ROLES_PATH` env var
    pub fn default_from_playbook(playbook_dir: &Path) -> Self {
        let mut dirs = vec![playbook_dir.join("roles")];

        // ANSIBLE_ROLES_PATH (colon-separated on Unix).
        if let Ok(env_paths) = std::env::var("ANSIBLE_ROLES_PATH") {
            for p in env_paths.split(':').filter(|s| !s.is_empty()) {
                dirs.push(PathBuf::from(p));
            }
        }

        // ~/.ansible/roles
        if let Some(home) = dirs_home() {
            dirs.push(home.join(".ansible").join("roles"));
        }

        dirs.push(PathBuf::from("/etc/ansible/roles"));
        Self { dirs }
    }

    /// Find the first directory containing `<role_name>/tasks/main.yml`.
    pub fn find(&self, role_name: &str) -> Option<PathBuf> {
        for dir in &self.dirs {
            let candidate = dir.join(role_name);
            if candidate.join("tasks").join("main.yml").exists()
                || candidate.join("tasks").join("main.yaml").exists()
                || candidate.is_dir()
            {
                return Some(candidate);
            }
        }
        None
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

// ---------------------------------------------------------------------------
// RoleLoader
// ---------------------------------------------------------------------------

/// Loads Ansible roles from the filesystem.
///
/// ```rust,no_run
/// use ansiblers_roles::{RoleLoader, RolePath};
/// use std::path::PathBuf;
///
/// let role_path = RolePath::new(vec![PathBuf::from("roles")]);
/// let loader = RoleLoader::new(role_path);
/// let role = loader.load("common").unwrap();
/// assert!(role.has_tasks());
/// ```
pub struct RoleLoader {
    pub role_path: RolePath,
}

impl RoleLoader {
    pub fn new(role_path: RolePath) -> Self {
        Self { role_path }
    }

    /// Load a role by name.
    pub fn load(&self, name: &str) -> Result<Role> {
        let path = self.role_path.find(name).ok_or_else(|| {
            anyhow::anyhow!("role '{}' not found in {:?}", name, self.role_path.dirs)
        })?;

        self.load_from_path(name, &path)
    }

    /// Load a role from an explicit directory path.
    pub fn load_from_path(&self, name: &str, path: &Path) -> Result<Role> {
        let mut role = Role::new(name, path.to_path_buf());

        // defaults/main.yml
        role.defaults = load_vars_dir(&path.join("defaults"))?;

        // vars/main.yml
        role.vars = load_vars_dir(&path.join("vars"))?;

        // tasks/main.yml
        role.tasks = load_task_list(&path.join("tasks"))
            .with_context(|| format!("loading tasks for role '{name}'"))?;

        // handlers/main.yml
        role.handlers = load_handler_list(&path.join("handlers"))
            .with_context(|| format!("loading handlers for role '{name}'"))?;

        // meta/main.yml
        let meta_path = path.join("meta").join("main.yml");
        if meta_path.exists() {
            role.meta = RoleMeta::from_file(meta_path.to_str().unwrap())
                .with_context(|| format!("loading meta for role '{name}'"))?;
        }

        Ok(role)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Load all YAML vars files from a `defaults/` or `vars/` directory.
fn load_vars_dir(dir: &Path) -> Result<HashMap<String, Value>> {
    if !dir.is_dir() {
        return Ok(HashMap::new());
    }
    let mut merged = HashMap::new();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .map(|ext| ext == "yml" || ext == "yaml")
                .unwrap_or(false)
        })
        .collect();
    paths.sort();
    for path in paths {
        let vars = load_vars_file(&path)?;
        merged.extend(vars);
    }
    Ok(merged)
}

/// Parse a single YAML vars file.
fn load_vars_file(path: &Path) -> Result<HashMap<String, Value>> {
    let content = std::fs::read_to_string(path)?;
    if content.trim().is_empty() {
        return Ok(HashMap::new());
    }
    let yaml: YamlValue = serde_yaml::from_str(&content)?;
    match yaml {
        YamlValue::Mapping(m) => m
            .into_iter()
            .map(|(k, v)| {
                let key = k
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("non-string var key"))?
                    .to_string();
                let val = yaml_to_json(&v)?;
                Ok((key, val))
            })
            .collect(),
        YamlValue::Null => Ok(HashMap::new()),
        _ => anyhow::bail!("vars file {} must be a mapping", path.display()),
    }
}

/// Parse `tasks/main.yml` (and any statically resolved includes).
fn load_task_list(tasks_dir: &Path) -> Result<Vec<ansiblers_parser::TaskNode>> {
    let main = tasks_dir.join("main.yml");
    if !main.exists() {
        let main_yaml = tasks_dir.join("main.yaml");
        if !main_yaml.exists() {
            return Ok(Vec::new());
        }
        return parse_task_file(&main_yaml);
    }
    parse_task_file(&main)
}

fn parse_task_file(path: &Path) -> Result<Vec<ansiblers_parser::TaskNode>> {
    let raw = std::fs::read_to_string(path)?;

    // Strip YAML document separators and leading comment lines that would
    // break embedding as a tasks: sub-key.
    let content: String = raw
        .lines()
        .filter(|l| {
            let t = l.trim();
            t != "---" && t != "..."
        })
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    if content.trim().is_empty() {
        return Ok(Vec::new());
    }

    // Strategy 1: assume the file is a bare task list (list items at column 0).
    // Indent each non-empty, non-comment line by 4 spaces to nest under tasks:.
    let indented = content
        .lines()
        .map(|l| {
            if l.is_empty() || l.starts_with('#') {
                l.to_string()
            } else {
                format!("    {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let wrapped = format!("- hosts: all\n  gather_facts: false\n  tasks:\n{indented}");
    ansiblers_parser::parse_playbook_str(&wrapped, None)
        .map(|pb| pb.plays.into_iter().flat_map(|p| p.tasks).collect())
        .with_context(|| format!("parsing task file {}", path.display()))
}

fn load_handler_list(handlers_dir: &Path) -> Result<Vec<ansiblers_parser::Handler>> {
    let main = handlers_dir.join("main.yml");
    if !main.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&main)?;
    let content: String = raw
        .lines()
        .filter(|l| {
            let t = l.trim();
            t != "---" && t != "..."
        })
        .collect::<Vec<_>>()
        .join("\n");

    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    let indented = content
        .lines()
        .map(|l| {
            if l.is_empty() || l.starts_with('#') {
                l.to_string()
            } else {
                format!("    {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let wrapped = format!("- hosts: all\n  gather_facts: false\n  handlers:\n{indented}");
    let pb = ansiblers_parser::parse_playbook_str(&wrapped, None)?;
    Ok(pb.plays.into_iter().flat_map(|p| p.handlers).collect())
}

fn yaml_to_json(val: &YamlValue) -> Result<Value> {
    let s = serde_json::to_string(
        &serde_yaml::from_value::<serde_json::Value>(val.clone()).context("yaml to json")?,
    )?;
    serde_json::from_str(&s).context("json parse")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(dir: &Path, file: &str, content: &str) {
        let path = dir.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    fn simple_role(tmp: &TempDir, name: &str) -> PathBuf {
        let role_dir = tmp.path().join("roles").join(name);
        write(
            &role_dir.parent().unwrap().parent().unwrap(),
            &format!("roles/{name}/tasks/main.yml"),
            "- name: Echo\n  shell: echo hello\n",
        );
        write(
            &role_dir.parent().unwrap().parent().unwrap(),
            &format!("roles/{name}/defaults/main.yml"),
            "my_default: 42\n",
        );
        write(
            &role_dir.parent().unwrap().parent().unwrap(),
            &format!("roles/{name}/vars/main.yml"),
            "my_var: override\n",
        );
        role_dir
    }

    #[test]
    fn test_load_simple_role() {
        let tmp = TempDir::new().unwrap();
        let role_dir = simple_role(&tmp, "common");
        let loader = RoleLoader::new(RolePath::new(vec![tmp.path().join("roles")]));
        let role = loader.load("common").unwrap();
        assert!(role.has_tasks());
        assert_eq!(role.tasks.len(), 1);
        assert_eq!(
            role.defaults.get("my_default"),
            Some(&Value::Number(42.into()))
        );
        assert_eq!(
            role.vars.get("my_var"),
            Some(&Value::String("override".into()))
        );
    }

    #[test]
    fn test_role_not_found() {
        let tmp = TempDir::new().unwrap();
        let loader = RoleLoader::new(RolePath::new(vec![tmp.path().join("roles")]));
        assert!(loader.load("nonexistent").is_err());
    }

    #[test]
    fn test_role_path_find() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("roles/myrole/tasks")).unwrap();
        std::fs::write(
            tmp.path().join("roles/myrole/tasks/main.yml"),
            "- name: t\n  debug:\n    msg: hi\n",
        )
        .unwrap();
        let rp = RolePath::new(vec![tmp.path().join("roles")]);
        assert!(rp.find("myrole").is_some());
        assert!(rp.find("missing").is_none());
    }

    #[test]
    fn test_load_role_with_handlers() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "roles/web/tasks/main.yml",
            "- name: Install\n  shell: apt-get install -y nginx\n  notify: Restart nginx\n",
        );
        write(
            tmp.path(),
            "roles/web/handlers/main.yml",
            "- name: Restart nginx\n  shell: systemctl restart nginx\n",
        );
        let loader = RoleLoader::new(RolePath::new(vec![tmp.path().join("roles")]));
        let role = loader.load("web").unwrap();
        assert_eq!(role.handlers.len(), 1);
    }

    #[test]
    fn test_load_role_meta() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "roles/app/tasks/main.yml",
            "- debug:\n    msg: hi\n",
        );
        write(
            tmp.path(),
            "roles/app/meta/main.yml",
            "dependencies:\n  - role: common\n",
        );
        let loader = RoleLoader::new(RolePath::new(vec![tmp.path().join("roles")]));
        let role = loader.load("app").unwrap();
        assert_eq!(role.meta.dependencies.len(), 1);
        assert_eq!(role.meta.dependencies[0].role_name(), "common");
    }
}
