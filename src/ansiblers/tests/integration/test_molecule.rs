//! Integration tests: ansiblers-molecule lifecycle with the None driver.

use ansiblers_molecule::{
    config::MoleculeConfig,
    driver::DriverKind,
    lifecycle::{LifecyclePhase, ScenarioRunner},
    scenario::Scenario,
};

#[test]
fn test_molecule_none_driver_create_destroy() {
    let mut cfg = MoleculeConfig::default_docker("instance", "ubuntu:24.04");
    cfg.driver.name = DriverKind::None;
    let mut scenario = Scenario::from_config("default", cfg);
    let runner = ScenarioRunner::new();

    let result = runner
        .run_sequence(&mut scenario, &[LifecyclePhase::Create, LifecyclePhase::Destroy])
        .unwrap();
    assert!(result.success);
    assert_eq!(result.phases.len(), 2);
}

#[test]
fn test_molecule_config_default_docker() {
    let cfg = MoleculeConfig::default_docker("myinstance", "debian:12");
    assert_eq!(cfg.platforms.len(), 1);
    assert_eq!(cfg.platforms[0].name, "myinstance");
    assert_eq!(cfg.platforms[0].image, "debian:12");
    assert!(matches!(cfg.driver.name, DriverKind::Docker));
}

#[test]
fn test_molecule_scenario_from_config() {
    let cfg = MoleculeConfig::default_docker("instance", "alpine:3.19");
    let scenario = Scenario::from_config("unit", cfg);
    assert_eq!(scenario.name, "unit");
    assert_eq!(scenario.instances.len(), 1);
    assert_eq!(scenario.instances[0].image, "alpine:3.19");
}
