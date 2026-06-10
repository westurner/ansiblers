//! `ransible-molecule` CLI entry point.

use std::path::PathBuf;
use std::process;

use anyhow::{Context, Result};
use ansiblers_molecule::{
    config::MoleculeConfig, driver::DriverKind, lifecycle::ScenarioRunner, scenario::Scenario,
};
use clap::{Parser, Subcommand};
use tracing::{error, info};

#[derive(Parser, Debug)]
#[command(name = "ransible-molecule", version, about = "Molecule-compatible multi-node test runner")]
struct Cli {
    #[command(subcommand)]
    command: MoleculeCommand,

    /// Molecule scenario name (default: default).
    #[arg(short = 's', long = "scenario-name", default_value = "default")]
    scenario: String,

    /// Project root directory.
    #[arg(long = "base-config")]
    base_config: Option<PathBuf>,

    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
    verbose: u8,
}

#[derive(Subcommand, Debug)]
enum MoleculeCommand {
    /// Run full test sequence (create→converge→verify→destroy).
    Test,
    /// Create instances.
    Create,
    /// Run the converge playbook.
    Converge,
    /// Run the verify playbook.
    Verify,
    /// Destroy instances.
    Destroy,
    /// List instances.
    List,
}

fn main() {
    let cli = Cli::parse();

    let log_level = match cli.verbose {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| log_level.into()),
        )
        .init();

    match run(cli) {
        Ok(()) => {}
        Err(e) => {
            error!("{e:#}");
            process::exit(1);
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let project_root = std::env::current_dir().context("get cwd")?;
    let config_path = project_root
        .join("molecule")
        .join(&cli.scenario)
        .join("molecule.yml");

    // Load config, or use a minimal none-driver default for `list`.
    let config = if config_path.exists() {
        MoleculeConfig::from_file(config_path.to_str().unwrap())?
    } else {
        // No molecule.yml — create a minimal none-driver scenario.
        let mut cfg = MoleculeConfig::default_docker("instance", "localhost");
        cfg.driver.name = DriverKind::None;
        cfg
    };

    let mut scenario = Scenario::from_config(&cli.scenario, config);
    let runner = ScenarioRunner::new();

    match cli.command {
        MoleculeCommand::Test => {
            let result = runner.test(&mut scenario)?;
            print_result(&result);
            if !result.success {
                process::exit(2);
            }
        }
        MoleculeCommand::Create => {
            scenario.create()?;
            info!("Instances created");
        }
        MoleculeCommand::Destroy => {
            scenario.destroy()?;
            info!("Instances destroyed");
        }
        MoleculeCommand::Converge => {
            let result = runner.converge(&mut scenario)?;
            print_result(&result);
            if !result.success {
                process::exit(2);
            }
        }
        MoleculeCommand::Verify => {
            use ansiblers_molecule::lifecycle::LifecyclePhase;
            let result = runner.run_sequence(&mut scenario, &[LifecyclePhase::Verify])?;
            print_result(&result);
            if !result.success {
                process::exit(2);
            }
        }
        MoleculeCommand::List => {
            println!("{:<20} {:<30} {:<10}", "INSTANCE", "IMAGE", "STATE");
            println!("{}", "-".repeat(62));
            for inst in &scenario.instances {
                println!("{:<20} {:<30} {:?}", inst.name, inst.image, inst.state);
            }
        }
    }

    Ok(())
}

fn print_result(result: &ansiblers_molecule::lifecycle::ScenarioResult) {
    println!("\nScenario: {}", result.scenario_name);
    for phase in &result.phases {
        let status = if phase.success { "ok" } else { "FAILED" };
        println!("  {:<15} {}", phase.phase, status);
    }
    println!(
        "\nResult: {}",
        if result.success { "SUCCESS" } else { "FAILURE" }
    );
}
