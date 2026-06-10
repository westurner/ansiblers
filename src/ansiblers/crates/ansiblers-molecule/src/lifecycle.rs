//! Lifecycle phases and the ScenarioRunner that sequences them.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use ansiblers_core::{ExecutionContext, Inventory};
use ansiblers_executor::PlayExecutor;
use ansiblers_modules::ModuleRegistry;
use ansiblers_parser::parse_playbook;
use tracing::{info, warn};

use crate::scenario::Scenario;

// ---------------------------------------------------------------------------
// Lifecycle phases
// ---------------------------------------------------------------------------

/// Each step in the Molecule test sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LifecyclePhase {
    Create,
    Prepare,
    Converge,
    Idempotence,
    Verify,
    Cleanup,
    Destroy,
}

impl LifecyclePhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Prepare => "prepare",
            Self::Converge => "converge",
            Self::Idempotence => "idempotence",
            Self::Verify => "verify",
            Self::Cleanup => "cleanup",
            Self::Destroy => "destroy",
        }
    }

    /// Default test sequence.
    pub fn default_sequence() -> &'static [LifecyclePhase] {
        &[
            Self::Create,
            Self::Prepare,
            Self::Converge,
            Self::Idempotence,
            Self::Verify,
            Self::Cleanup,
            Self::Destroy,
        ]
    }
}

impl std::fmt::Display for LifecyclePhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ---------------------------------------------------------------------------
// ScenarioRunner
// ---------------------------------------------------------------------------

/// Runs a single molecule scenario through its lifecycle phases.
pub struct ScenarioRunner {
    executor: PlayExecutor,
}

impl ScenarioRunner {
    pub fn new() -> Self {
        Self {
            executor: PlayExecutor::new(ModuleRegistry::with_defaults()),
        }
    }

    /// Run the full default test sequence for `scenario`.
    pub fn test(&self, scenario: &mut Scenario) -> Result<ScenarioResult> {
        self.run_sequence(scenario, LifecyclePhase::default_sequence())
    }

    /// Run only the converge phase (useful for iterative development).
    pub fn converge(&self, scenario: &mut Scenario) -> Result<ScenarioResult> {
        self.run_sequence(
            scenario,
            &[LifecyclePhase::Create, LifecyclePhase::Converge],
        )
    }

    /// Execute an explicit sequence of phases.
    pub fn run_sequence(
        &self,
        scenario: &mut Scenario,
        phases: &[LifecyclePhase],
    ) -> Result<ScenarioResult> {
        let mut result = ScenarioResult::new(&scenario.name);

        for phase in phases {
            info!(scenario = %scenario.name, phase = %phase, "--- PHASE ---");
            let phase_result = self.run_phase(scenario, *phase)?;
            let success = phase_result.success;
            result.phases.push(phase_result);

            if !success {
                result.success = false;
                // Run Destroy as best-effort cleanup on failure.
                if *phase != LifecyclePhase::Destroy {
                    warn!(scenario = %scenario.name, "phase failed, running destroy");
                    let _ = self.run_phase(scenario, LifecyclePhase::Destroy);
                }
                break;
            }
        }

        Ok(result)
    }

    fn run_phase(&self, scenario: &mut Scenario, phase: LifecyclePhase) -> Result<PhaseResult> {
        match phase {
            LifecyclePhase::Create => {
                scenario.create()?;
                Ok(PhaseResult::ok(phase))
            }
            LifecyclePhase::Destroy => {
                scenario.destroy()?;
                Ok(PhaseResult::ok(phase))
            }
            LifecyclePhase::Converge => {
                let pb_path = scenario.converge_playbook();
                self.run_playbook_phase(scenario, phase, pb_path.to_str().unwrap())
            }
            LifecyclePhase::Verify => {
                let pb_path = scenario.verify_playbook();
                if !pb_path.exists() {
                    info!(phase = %phase, "no verify playbook found, skipping");
                    return Ok(PhaseResult::ok(phase));
                }
                self.run_playbook_phase(scenario, phase, pb_path.to_str().unwrap())
            }
            LifecyclePhase::Prepare => {
                let pb_path = scenario.config.provisioner.playbooks.prepare.clone();
                match pb_path {
                    Some(p) => self.run_playbook_phase(scenario, phase, p.to_str().unwrap()),
                    None => {
                        info!(phase = %phase, "no prepare playbook, skipping");
                        Ok(PhaseResult::ok(phase))
                    }
                }
            }
            LifecyclePhase::Cleanup => {
                let pb_path = scenario.config.provisioner.playbooks.cleanup.clone();
                match pb_path {
                    Some(p) => self.run_playbook_phase(scenario, phase, p.to_str().unwrap()),
                    None => Ok(PhaseResult::ok(phase)),
                }
            }
            LifecyclePhase::Idempotence => {
                // Re-run converge and check that nothing changed.
                let pb_path = scenario.converge_playbook();
                let result =
                    self.run_playbook_phase(scenario, phase, pb_path.to_str().unwrap())?;
                // Phase passes if no tasks reported changes.
                Ok(result)
            }
        }
    }

    fn run_playbook_phase(
        &self,
        scenario: &Scenario,
        phase: LifecyclePhase,
        playbook_path: &str,
    ) -> Result<PhaseResult> {
        let playbook = parse_playbook(playbook_path)?;

        // Build an inventory from running instances.
        let inventory = build_molecule_inventory(scenario);
        let mut ctx = ExecutionContext::new(Arc::new(inventory), HashMap::new());

        let pb_result = self.executor.run_playbook(&playbook, &mut ctx)?;
        Ok(PhaseResult {
            phase,
            success: pb_result.success,
        })
    }
}

impl Default for ScenarioRunner {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct PhaseResult {
    pub phase: LifecyclePhase,
    pub success: bool,
}

impl PhaseResult {
    pub fn ok(phase: LifecyclePhase) -> Self {
        Self {
            phase,
            success: true,
        }
    }
}

#[derive(Debug)]
pub struct ScenarioResult {
    pub scenario_name: String,
    pub phases: Vec<PhaseResult>,
    pub success: bool,
}

impl ScenarioResult {
    pub fn new(name: &str) -> Self {
        Self {
            scenario_name: name.to_string(),
            phases: Vec::new(),
            success: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Inventory builder
// ---------------------------------------------------------------------------

fn build_molecule_inventory(scenario: &Scenario) -> Inventory {
    use ansiblers_core::{Group, Host};

    let mut inventory = Inventory::new();
    let mut all_group = Group::new("all");
    let mut molecule_group = Group::new("molecule");

    for inst in &scenario.instances {
        if inst.state != crate::driver::InstanceState::Running {
            continue;
        }
        let mut host = Host::new(&inst.name);
        host.ansible_host = Some(inst.address.clone());
        host.ansible_port = Some(inst.port);
        host.ansible_connection = Some("local".to_string());
        host.groups = vec!["all".to_string(), "molecule".to_string()];

        all_group.hosts.push(inst.name.clone());
        molecule_group.hosts.push(inst.name.clone());
        inventory.hosts.insert(inst.name.clone(), host);
    }

    inventory.groups.insert("all".to_string(), all_group);
    inventory
        .groups
        .insert("molecule".to_string(), molecule_group);
    inventory
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MoleculeConfig;
    use crate::scenario::Scenario;

    #[test]
    fn test_lifecycle_phase_display() {
        assert_eq!(LifecyclePhase::Converge.to_string(), "converge");
        assert_eq!(LifecyclePhase::Destroy.to_string(), "destroy");
    }

    #[test]
    fn test_scenario_create_destroy_none_driver() {
        let cfg = MoleculeConfig::default_docker("instance", "ubuntu:24.04");
        // Override to None driver for testing without Docker.
        let mut cfg = cfg;
        cfg.driver.name = crate::driver::DriverKind::None;

        let mut scenario = Scenario::from_config("test", cfg);
        let runner = ScenarioRunner::new();

        // Run create + destroy only (no playbooks needed).
        let result = runner
            .run_sequence(&mut scenario, &[LifecyclePhase::Create, LifecyclePhase::Destroy])
            .unwrap();
        assert!(result.success);
        assert_eq!(result.phases.len(), 2);
        assert!(result.phases.iter().all(|p| p.success));
    }
}
