//! Driver trait and implementations (Docker, Podman, None).

use std::collections::HashMap;
use std::process::Command;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DriverKind {
    Docker,
    Podman,
    /// Local execution (no container isolation).
    None,
}

impl std::fmt::Display for DriverKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Docker => write!(f, "docker"),
            Self::Podman => write!(f, "podman"),
            Self::None => write!(f, "none"),
        }
    }
}

/// Current state of a molecule instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstanceState {
    Absent,
    Starting,
    Running,
    Stopping,
    Stopped,
}

/// A managed test instance (container or local).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub name: String,
    pub image: String,
    pub state: InstanceState,
    /// The address/hostname used to connect to this instance.
    pub address: String,
    /// SSH port (if applicable).
    pub port: u16,
    /// Additional labels for tracking.
    pub labels: HashMap<String, String>,
}

impl Instance {
    pub fn new(name: impl Into<String>, image: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            address: "localhost".to_string(),
            port: 22,
            image: image.into(),
            labels: HashMap::new(),
            state: InstanceState::Absent,
            name,
        }
    }
}

// ---------------------------------------------------------------------------
// Driver trait
// ---------------------------------------------------------------------------

/// Pluggable backend for creating/destroying molecule instances.
pub trait Driver: Send + Sync {
    fn kind(&self) -> DriverKind;

    /// Create an instance from an image.
    fn create(&self, instance: &mut Instance) -> Result<()>;

    /// Destroy (remove) an instance.
    fn destroy(&self, instance: &mut Instance) -> Result<()>;

    /// Check if an instance is running.
    fn status(&self, name: &str) -> Result<InstanceState>;

    /// List all molecule-managed instances.
    fn list(&self) -> Result<Vec<Instance>>;
}

// ---------------------------------------------------------------------------
// Docker driver
// ---------------------------------------------------------------------------

pub struct DockerDriver;

impl Driver for DockerDriver {
    fn kind(&self) -> DriverKind {
        DriverKind::Docker
    }

    fn create(&self, instance: &mut Instance) -> Result<()> {
        let container_name = molecule_container_name(&instance.name);
        let output = Command::new("docker")
            .args([
                "run",
                "--detach",
                "--name",
                &container_name,
                "--label",
                "molecule=true",
                "--label",
                &format!("molecule_instance={}", instance.name),
                &instance.image,
                "sleep",
                "infinity",
            ])
            .output()
            .context("running docker")?;

        if !output.status.success() {
            anyhow::bail!(
                "docker run failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        instance.state = InstanceState::Running;
        // Use container name as address for docker exec.
        instance.address = container_name;
        Ok(())
    }

    fn destroy(&self, instance: &mut Instance) -> Result<()> {
        let container_name = molecule_container_name(&instance.name);
        let _ = Command::new("docker")
            .args(["rm", "-f", &container_name])
            .output();
        instance.state = InstanceState::Absent;
        Ok(())
    }

    fn status(&self, name: &str) -> Result<InstanceState> {
        let container_name = molecule_container_name(name);
        let output = Command::new("docker")
            .args([
                "inspect",
                "--format",
                "{{.State.Running}}",
                &container_name,
            ])
            .output()
            .context("docker inspect")?;

        if !output.status.success() {
            return Ok(InstanceState::Absent);
        }
        let running = String::from_utf8_lossy(&output.stdout).trim() == "true";
        Ok(if running {
            InstanceState::Running
        } else {
            InstanceState::Stopped
        })
    }

    fn list(&self) -> Result<Vec<Instance>> {
        let output = Command::new("docker")
            .args([
                "ps",
                "-a",
                "--filter",
                "label=molecule=true",
                "--format",
                "{{.Names}}\t{{.Image}}\t{{.Status}}",
            ])
            .output()
            .context("docker ps")?;

        let mut instances = Vec::new();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            if parts.len() < 2 {
                continue;
            }
            let name = parts[0].to_string();
            let image = parts[1].to_string();
            let state = if parts.get(2).map(|s| s.starts_with("Up")).unwrap_or(false) {
                InstanceState::Running
            } else {
                InstanceState::Stopped
            };
            instances.push(Instance {
                name: name.clone(),
                image,
                state,
                address: name,
                port: 22,
                labels: HashMap::new(),
            });
        }
        Ok(instances)
    }
}

// ---------------------------------------------------------------------------
// Podman driver
// ---------------------------------------------------------------------------

pub struct PodmanDriver;

impl Driver for PodmanDriver {
    fn kind(&self) -> DriverKind {
        DriverKind::Podman
    }

    fn create(&self, instance: &mut Instance) -> Result<()> {
        let container_name = molecule_container_name(&instance.name);
        let output = Command::new("podman")
            .args([
                "run",
                "--detach",
                "--name",
                &container_name,
                "--label",
                "molecule=true",
                "--label",
                &format!("molecule_instance={}", instance.name),
                &instance.image,
                "sleep",
                "infinity",
            ])
            .output()
            .context("running podman")?;

        if !output.status.success() {
            anyhow::bail!(
                "podman run failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        instance.state = InstanceState::Running;
        instance.address = container_name;
        Ok(())
    }

    fn destroy(&self, instance: &mut Instance) -> Result<()> {
        let container_name = molecule_container_name(&instance.name);
        let _ = Command::new("podman")
            .args(["rm", "-f", &container_name])
            .output();
        instance.state = InstanceState::Absent;
        Ok(())
    }

    fn status(&self, name: &str) -> Result<InstanceState> {
        let container_name = molecule_container_name(name);
        let output = Command::new("podman")
            .args([
                "inspect",
                "--format",
                "{{.State.Running}}",
                &container_name,
            ])
            .output()
            .context("podman inspect")?;

        if !output.status.success() {
            return Ok(InstanceState::Absent);
        }
        let running = String::from_utf8_lossy(&output.stdout).trim() == "true";
        Ok(if running {
            InstanceState::Running
        } else {
            InstanceState::Stopped
        })
    }

    fn list(&self) -> Result<Vec<Instance>> {
        // Delegate to podman ps with same format as docker.
        let output = Command::new("podman")
            .args([
                "ps",
                "-a",
                "--filter",
                "label=molecule=true",
                "--format",
                "{{.Names}}\t{{.Image}}\t{{.Status}}",
            ])
            .output()
            .context("podman ps")?;

        let mut instances = Vec::new();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            if parts.len() < 2 {
                continue;
            }
            let name = parts[0].to_string();
            let image = parts[1].to_string();
            let state = if parts.get(2).map(|s| s.starts_with("Up")).unwrap_or(false) {
                InstanceState::Running
            } else {
                InstanceState::Stopped
            };
            instances.push(Instance {
                name: name.clone(),
                image,
                state,
                address: name,
                port: 22,
                labels: HashMap::new(),
            });
        }
        Ok(instances)
    }
}

// ---------------------------------------------------------------------------
// None (local) driver
// ---------------------------------------------------------------------------

/// Localhost driver — runs without container isolation (useful for CI).
pub struct NoneDriver;

impl Driver for NoneDriver {
    fn kind(&self) -> DriverKind {
        DriverKind::None
    }

    fn create(&self, instance: &mut Instance) -> Result<()> {
        instance.state = InstanceState::Running;
        instance.address = "localhost".to_string();
        Ok(())
    }

    fn destroy(&self, instance: &mut Instance) -> Result<()> {
        instance.state = InstanceState::Absent;
        Ok(())
    }

    fn status(&self, _name: &str) -> Result<InstanceState> {
        Ok(InstanceState::Running)
    }

    fn list(&self) -> Result<Vec<Instance>> {
        Ok(Vec::new())
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

pub fn make_driver(kind: &DriverKind) -> Box<dyn Driver> {
    match kind {
        DriverKind::Docker => Box::new(DockerDriver),
        DriverKind::Podman => Box::new(PodmanDriver),
        DriverKind::None => Box::new(NoneDriver),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn molecule_container_name(instance_name: &str) -> String {
    format!("molecule-{instance_name}")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_none_driver_create_destroy() {
        let driver = NoneDriver;
        let mut inst = Instance::new("test", "localhost");
        driver.create(&mut inst).unwrap();
        assert_eq!(inst.state, InstanceState::Running);
        driver.destroy(&mut inst).unwrap();
        assert_eq!(inst.state, InstanceState::Absent);
    }

    #[test]
    fn test_none_driver_status() {
        let driver = NoneDriver;
        let state = driver.status("any").unwrap();
        assert_eq!(state, InstanceState::Running);
    }

    #[test]
    fn test_molecule_container_name() {
        assert_eq!(molecule_container_name("myinstance"), "molecule-myinstance");
    }

    #[test]
    fn test_driver_kind_display() {
        assert_eq!(DriverKind::Docker.to_string(), "docker");
        assert_eq!(DriverKind::Podman.to_string(), "podman");
        assert_eq!(DriverKind::None.to_string(), "none");
    }
}
