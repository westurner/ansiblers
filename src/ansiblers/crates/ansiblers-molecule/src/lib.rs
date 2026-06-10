//! `ansiblers-molecule` — multi-node test isolation and orchestration.
//!
//! Implements the [Molecule](https://molecule.readthedocs.io/) lifecycle model
//! in Rust, providing container-based test isolation for Ansible roles and
//! playbooks.
//!
//! ## Lifecycle
//!
//! ```text
//! Create → Prepare → Converge → Idempotence → Verify → Cleanup → Destroy
//! ```
//!
//! Each phase runs a playbook against the managed instances.  The
//! [`ScenarioRunner`] sequences them and handles failure recovery (Destroy is
//! always attempted on failure).
//!
//! ## Drivers
//!
//! | Driver | Description | Requires |
//! |--------|-------------|----------|
//! | [`DockerDriver`] | Container isolation via Docker | `docker` CLI |
//! | [`PodmanDriver`] | Rootless containers via Podman | `podman` CLI |
//! | [`NoneDriver`] | Local execution, no container | Nothing |
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use ansiblers_molecule::{
//!     config::MoleculeConfig,
//!     driver::DriverKind,
//!     lifecycle::{LifecyclePhase, ScenarioRunner},
//!     scenario::Scenario,
//! };
//!
//! // Create a scenario from an in-memory config (no molecule.yml needed):
//! let mut cfg = MoleculeConfig::default_docker("instance", "debian:12");
//! cfg.driver.name = DriverKind::None; // Use None driver for CI
//! let mut scenario = Scenario::from_config("default", cfg);
//!
//! let runner = ScenarioRunner::new();
//! let result = runner
//!     .run_sequence(&mut scenario, &[LifecyclePhase::Create, LifecyclePhase::Destroy])
//!     .unwrap();
//! assert!(result.success);
//! ```
//!
//! ## Configuration file
//!
//! Load from `molecule/<scenario>/molecule.yml`:
//!
//! ```yaml
//! driver:
//!   name: docker
//! platforms:
//!   - name: instance
//!     image: ubuntu:24.04
//!     pre_build_image: true
//! provisioner:
//!   name: ansible
//! verifier:
//!   name: ansible
//! ```
//!
//! [`DockerDriver`]: driver::DockerDriver
//! [`PodmanDriver`]: driver::PodmanDriver
//! [`NoneDriver`]: driver::NoneDriver

pub mod config;
pub mod driver;
pub mod lifecycle;
pub mod scenario;

pub use config::MoleculeConfig;
pub use driver::{Driver, DriverKind, Instance, InstanceState};
pub use lifecycle::{LifecyclePhase, ScenarioRunner};
pub use scenario::Scenario;
