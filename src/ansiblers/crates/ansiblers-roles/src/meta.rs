//! Role metadata — parsed from `meta/main.yml`.
//!
//! The `meta/main.yml` file declares role dependencies and optional galaxy
//! metadata (author, license, platforms, etc.).
//!
//! ## Example
//!
//! ```yaml
//! galaxy_info:
//!   author: myorg
//!   description: Configure a web server
//!   license: GPL-3.0
//!   min_ansible_version: "2.14"
//!   platforms:
//!     - name: Ubuntu
//!       versions: ["22.04", "24.04"]
//!   galaxy_tags: [web, nginx]
//!
//! dependencies:
//!   - role: common
//!   - role: geerlingguy.git
//!     vars:
//!       git_version: "2.40"
//!   - role: myorg.tls
//!     when: "ansible_os_family == 'Debian'"
//! ```

use std::collections::HashMap;

use ansiblers_core::Value;
use serde::{Deserialize, Serialize};

/// Metadata from `meta/main.yml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoleMeta {
    /// Galaxy metadata block.
    pub galaxy_info: Option<GalaxyInfo>,

    /// List of role dependencies.
    #[serde(default)]
    pub dependencies: Vec<RoleDependency>,

    /// Whether to allow this role to be used without a `when:` guard
    /// when `allow_duplicates: true`.
    #[serde(default)]
    pub allow_duplicates: bool,
}

impl RoleMeta {
    /// Load from a `meta/main.yml` file path.
    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        if content.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_yaml::from_str(&content).map_err(|e| anyhow::anyhow!("parsing meta/main.yml: {e}"))
    }
}

/// Galaxy metadata block inside `meta/main.yml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GalaxyInfo {
    pub author: Option<String>,
    pub description: Option<String>,
    pub license: Option<String>,
    pub min_ansible_version: Option<String>,
    #[serde(default)]
    pub platforms: Vec<PlatformEntry>,
    #[serde(default)]
    pub galaxy_tags: Vec<String>,
}

/// Platform / version entry in `galaxy_info.platforms`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformEntry {
    pub name: String,
    #[serde(default)]
    pub versions: Vec<String>,
}

/// A single role dependency declared in `meta/main.yml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RoleDependency {
    /// Simple form: just the role name.
    Simple(String),
    /// Full form: role name + optional vars and when condition.
    Full(RoleDependencySpec),
}

impl RoleDependency {
    pub fn role_name(&self) -> &str {
        match self {
            Self::Simple(s) => s.as_str(),
            Self::Full(spec) => spec.role.as_str(),
        }
    }

    pub fn vars(&self) -> &HashMap<String, Value> {
        match self {
            Self::Simple(_) => &EMPTY_VARS,
            Self::Full(spec) => &spec.vars,
        }
    }

    pub fn when(&self) -> Option<&str> {
        match self {
            Self::Simple(_) => None,
            Self::Full(spec) => spec.when.as_deref(),
        }
    }
}

static EMPTY_VARS: std::sync::LazyLock<HashMap<String, Value>> =
    std::sync::LazyLock::new(HashMap::new);

/// Full dependency specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleDependencySpec {
    pub role: String,
    #[serde(default)]
    pub vars: HashMap<String, Value>,
    pub when: Option<String>,
    pub tags: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const META_YAML: &str = r#"
galaxy_info:
  author: testorg
  description: A test role
  license: GPL-3.0
  platforms:
    - name: Ubuntu
      versions: ["22.04", "24.04"]
  galaxy_tags: [web, test]

dependencies:
  - role: common
  - role: geerlingguy.git
    vars:
      git_version: "2.40"
  - role: myorg.tls
    when: "ansible_os_family == 'Debian'"
"#;

    #[test]
    fn test_parse_meta_yaml() {
        let meta: RoleMeta = serde_yaml::from_str(META_YAML).unwrap();
        assert_eq!(meta.dependencies.len(), 3);
        assert_eq!(meta.dependencies[0].role_name(), "common");
        assert_eq!(meta.dependencies[1].role_name(), "geerlingguy.git");
        assert_eq!(meta.dependencies[2].role_name(), "myorg.tls");
    }

    #[test]
    fn test_dependency_when() {
        let meta: RoleMeta = serde_yaml::from_str(META_YAML).unwrap();
        assert_eq!(
            meta.dependencies[2].when(),
            Some("ansible_os_family == 'Debian'")
        );
    }

    #[test]
    fn test_galaxy_info() {
        let meta: RoleMeta = serde_yaml::from_str(META_YAML).unwrap();
        let info = meta.galaxy_info.unwrap();
        assert_eq!(info.author.as_deref(), Some("testorg"));
        assert_eq!(info.platforms.len(), 1);
        assert_eq!(info.galaxy_tags, vec!["web", "test"]);
    }

    #[test]
    fn test_dependency_vars() {
        let meta: RoleMeta = serde_yaml::from_str(META_YAML).unwrap();
        let git_dep_vars = meta.dependencies[1].vars();
        assert_eq!(
            git_dep_vars.get("git_version"),
            Some(&Value::String("2.40".into()))
        );
    }
}
