//! ansible-galaxy `requirements.yml` parsing.
//!
//! The `requirements.yml` format allows declaring role (and collection)
//! dependencies that `ansible-galaxy install` downloads automatically.
//!
//! ## Example `requirements.yml`
//!
//! ```yaml
//! roles:
//!   - name: geerlingguy.nginx
//!     version: "6.0.0"
//!   - src: https://github.com/myorg/myrole
//!     name: myrole
//!     version: main
//!   - role: common          # short form
//!
//! collections:
//!   - name: community.general
//!     version: ">=7.0.0"
//! ```
//!
//! This module parses `requirements.yml` into typed structs.
//! Actual downloading is out of scope for Phase 3 (stub only).

use serde::{Deserialize, Serialize};

/// Contents of `requirements.yml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GalaxyRequirements {
    /// Role requirements.
    #[serde(default)]
    pub roles: Vec<RoleRequirement>,

    /// Collection requirements (parsed but not acted on in Phase 3).
    #[serde(default)]
    pub collections: Vec<CollectionRequirement>,
}

impl GalaxyRequirements {
    /// Parse from a `requirements.yml` file.
    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Self::from_str(&content)
    }

    /// Parse from a YAML string.
    pub fn from_str(content: &str) -> anyhow::Result<Self> {
        if content.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_yaml::from_str(content)
            .map_err(|e| anyhow::anyhow!("parsing requirements.yml: {e}"))
    }
}

/// A single role entry in `requirements.yml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RoleRequirement {
    /// Short form: just the role name or Galaxy ID.
    Simple(String),
    /// Full form with optional source, version, and local name.
    Full(RoleRequirementSpec),
}

impl RoleRequirement {
    /// The local name for this role (after installation).
    pub fn name(&self) -> &str {
        match self {
            Self::Simple(s) => s.as_str(),
            Self::Full(spec) => spec
                .name
                .as_deref()
                .or(spec.role.as_deref())
                .unwrap_or("unknown"),
        }
    }

    /// The source: Galaxy ID, GitHub URL, or local path.
    pub fn source(&self) -> &str {
        match self {
            Self::Simple(s) => s.as_str(),
            Self::Full(spec) => spec
                .src
                .as_deref()
                .or(spec.name.as_deref())
                .or(spec.role.as_deref())
                .unwrap_or("unknown"),
        }
    }

    /// The requested version constraint.
    pub fn version(&self) -> Option<&str> {
        match self {
            Self::Simple(_) => None,
            Self::Full(spec) => spec.version.as_deref(),
        }
    }
}

/// Full role requirement specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleRequirementSpec {
    /// Short `role:` form (Galaxy shorthand like `geerlingguy.nginx`).
    pub role: Option<String>,
    /// Galaxy ID or URL source.
    pub src: Option<String>,
    /// Local installation name.
    pub name: Option<String>,
    /// Version constraint (e.g. `">=6.0.0"`, `"main"`, `"v1.2.3"`).
    pub version: Option<String>,
    /// SCM type (`git`, `hg`). Defaults to `git` for URL sources.
    pub scm: Option<String>,
}

/// A collection requirement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionRequirement {
    pub name: String,
    pub version: Option<String>,
    pub source: Option<String>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const REQUIREMENTS: &str = r#"
roles:
  - name: geerlingguy.nginx
    version: "6.0.0"
  - src: https://github.com/myorg/myrole
    name: myrole
    version: main
  - role: common

collections:
  - name: community.general
    version: ">=7.0.0"
  - name: ansible.posix
"#;

    #[test]
    fn test_parse_requirements() {
        let req = GalaxyRequirements::from_str(REQUIREMENTS).unwrap();
        assert_eq!(req.roles.len(), 3);
        assert_eq!(req.collections.len(), 2);
    }

    #[test]
    fn test_role_names() {
        let req = GalaxyRequirements::from_str(REQUIREMENTS).unwrap();
        assert_eq!(req.roles[0].name(), "geerlingguy.nginx");
        assert_eq!(req.roles[1].name(), "myrole");
        assert_eq!(req.roles[2].name(), "common");
    }

    #[test]
    fn test_role_versions() {
        let req = GalaxyRequirements::from_str(REQUIREMENTS).unwrap();
        assert_eq!(req.roles[0].version(), Some("6.0.0"));
        assert_eq!(req.roles[1].version(), Some("main"));
        assert_eq!(req.roles[2].version(), None);
    }

    #[test]
    fn test_role_sources() {
        let req = GalaxyRequirements::from_str(REQUIREMENTS).unwrap();
        assert_eq!(req.roles[0].source(), "geerlingguy.nginx");
        assert!(req.roles[1].source().contains("github.com"));
    }

    #[test]
    fn test_collection_version() {
        let req = GalaxyRequirements::from_str(REQUIREMENTS).unwrap();
        assert_eq!(req.collections[0].version.as_deref(), Some(">=7.0.0"));
        assert!(req.collections[1].version.is_none());
    }

    #[test]
    fn test_empty_requirements() {
        let req = GalaxyRequirements::from_str("").unwrap();
        assert!(req.roles.is_empty());
        assert!(req.collections.is_empty());
    }
}
