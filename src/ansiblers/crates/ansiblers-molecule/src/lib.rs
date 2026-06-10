//! ansiblers-molecule — multi-node test isolation and orchestration.
//!
//! Implements the Molecule lifecycle model:
//!
//! ```text
//! Create → Prepare → Converge → Idempotence → Verify → Cleanup → Destroy
//! ```
//!
//! Each lifecycle phase runs a playbook against container-based instances.
//! Driver backends (Docker, Podman, none) are swappable via the [`Driver`] trait.

pub mod config;
pub mod driver;
pub mod lifecycle;
pub mod scenario;

pub use config::MoleculeConfig;
pub use driver::{Driver, DriverKind, Instance, InstanceState};
pub use lifecycle::{LifecyclePhase, ScenarioRunner};
pub use scenario::Scenario;
