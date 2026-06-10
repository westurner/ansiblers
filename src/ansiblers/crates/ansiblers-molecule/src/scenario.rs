//! [`Scenario`] — a named Molecule test scenario.
//!
//! A scenario corresponds to a `molecule/<name>/` directory containing a
//! `molecule.yml` configuration file plus playbooks (`converge.yml`,
//! optionally `prepare.yml`, `verify.yml`, `cleanup.yml`).
//!
//! ## Loading
//!
//! ```rust,no_run
//! use std::path::Path;
//! use ansiblers_molecule::scenario::Scenario;
//!
//! // From filesystem (requires molecule/<name>/molecule.yml to exist):
//! let scenario = Scenario::load(Path::new("."), "default").unwrap();
//!
//! // From in-memory config (no files needed — useful for unit tests):
//! use ansiblers_molecule::config::MoleculeConfig;
//! use ansiblers_molecule::driver::DriverKind;
//! let mut cfg = MoleculeConfig::default_docker("instance", "ubuntu:24.04");
//! cfg.driver.name = DriverKind::None;
//! let scenario = Scenario::from_config("unit", cfg);
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::MoleculeConfig;
use crate::driver::{make_driver, Driver, Instance};

/// A Molecule scenario: a self-contained test directory with molecule.yml.
pub struct Scenario {
    pub name: String,
    pub root: PathBuf,
    pub config: MoleculeConfig,
    pub instances: Vec<Instance>,
    driver: Box<dyn Driver>,
}

impl Scenario {
    /// Load from `<root>/molecule/<name>/molecule.yml`.
    pub fn load(project_root: &Path, scenario_name: &str) -> Result<Self> {
        let root = project_root.join("molecule").join(scenario_name);
        let config_path = root.join("molecule.yml");
        let config = MoleculeConfig::from_file(config_path.to_str().unwrap())
            .with_context(|| format!("loading molecule.yml for scenario '{scenario_name}'"))?;

        let instances = config
            .platforms
            .iter()
            .map(|p| Instance::new(&p.name, &p.image))
            .collect();

        let driver = make_driver(&config.driver.name);

        Ok(Self {
            name: scenario_name.to_string(),
            root,
            instances,
            driver,
            config,
        })
    }

    /// Create a scenario from an in-memory config (useful for tests).
    pub fn from_config(name: &str, config: MoleculeConfig) -> Self {
        let instances = config
            .platforms
            .iter()
            .map(|p| Instance::new(&p.name, &p.image))
            .collect();
        let driver = make_driver(&config.driver.name);
        Self {
            name: name.to_string(),
            root: PathBuf::from(format!("molecule/{name}")),
            instances,
            driver,
            config,
        }
    }

    pub fn create(&mut self) -> Result<()> {
        for inst in &mut self.instances {
            self.driver.create(inst)?;
        }
        Ok(())
    }

    pub fn destroy(&mut self) -> Result<()> {
        for inst in &mut self.instances {
            self.driver.destroy(inst)?;
        }
        Ok(())
    }

    /// Path to the converge playbook (defaults to `converge.yml` in scenario root).
    pub fn converge_playbook(&self) -> PathBuf {
        self.config
            .provisioner
            .playbooks
            .converge
            .clone()
            .unwrap_or_else(|| self.root.join("converge.yml"))
    }

    /// Path to the verify playbook (defaults to `verify.yml`).
    pub fn verify_playbook(&self) -> PathBuf {
        self.config
            .provisioner
            .playbooks
            .verify
            .clone()
            .unwrap_or_else(|| self.root.join("verify.yml"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MoleculeConfig;

    #[test]
    fn test_scenario_from_config() {
        let cfg = MoleculeConfig::default_docker("instance", "ubuntu:24.04");
        let scenario = Scenario::from_config("default", cfg);
        assert_eq!(scenario.name, "default");
        assert_eq!(scenario.instances.len(), 1);
        assert_eq!(scenario.instances[0].name, "instance");
    }
}
