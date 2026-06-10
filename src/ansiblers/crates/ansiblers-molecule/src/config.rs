//! Molecule scenario configuration (molecule/default/molecule.yml equivalent).

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::driver::DriverKind;

/// Top-level molecule.yml configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoleculeConfig {
    pub dependency: Option<DependencyConfig>,
    pub driver: DriverConfig,
    pub platforms: Vec<PlatformConfig>,
    pub provisioner: ProvisionerConfig,
    pub verifier: VerifierConfig,
    #[serde(default)]
    pub scenario: ScenarioConfig,
}

impl MoleculeConfig {
    /// Load from a molecule.yml file.
    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Ok(serde_yaml::from_str(&content)?)
    }

    /// Minimal default config (for testing without a file).
    pub fn default_docker(platform_name: &str, image: &str) -> Self {
        Self {
            dependency: None,
            driver: DriverConfig {
                name: DriverKind::Docker,
            },
            platforms: vec![PlatformConfig {
                name: platform_name.to_string(),
                image: image.to_string(),
                pre_build_image: true,
                volumes: vec![],
                env: HashMap::new(),
                privileged: false,
                command: None,
                network: None,
                published_ports: vec![],
            }],
            provisioner: ProvisionerConfig::default(),
            verifier: VerifierConfig::default(),
            scenario: ScenarioConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverConfig {
    pub name: DriverKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformConfig {
    pub name: String,
    pub image: String,
    #[serde(default = "default_true")]
    pub pre_build_image: bool,
    #[serde(default)]
    pub volumes: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub privileged: bool,
    pub command: Option<String>,
    pub network: Option<String>,
    #[serde(default)]
    pub published_ports: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionerConfig {
    pub name: String,
    #[serde(default)]
    pub playbooks: ProvisionerPlaybooks,
    #[serde(default)]
    pub inventory: ProvisionerInventory,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

impl Default for ProvisionerConfig {
    fn default() -> Self {
        Self {
            name: "ansible".to_string(),
            playbooks: ProvisionerPlaybooks::default(),
            inventory: ProvisionerInventory::default(),
            env: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProvisionerPlaybooks {
    pub converge: Option<PathBuf>,
    pub prepare: Option<PathBuf>,
    pub cleanup: Option<PathBuf>,
    pub verify: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProvisionerInventory {
    #[serde(default)]
    pub group_vars: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub host_vars: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifierConfig {
    pub name: String,
}

impl Default for VerifierConfig {
    fn default() -> Self {
        Self {
            name: "ansible".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScenarioConfig {
    pub name: Option<String>,
    pub test_sequence: Option<Vec<String>>,
    pub create_sequence: Option<Vec<String>>,
    pub destroy_sequence: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyConfig {
    pub name: String,
}

fn default_true() -> bool {
    true
}
