//! `ansiblers-build` — Artifact generation for the ansiblers ecosystem.
//!
//! This crate provides three complementary capabilities:
//!
//! ## 1. Multi-Stage Dockerfile Builder (`dockerfile`)
//!
//! Constructs multi-stage `Dockerfile`s programmatically with first-class
//! support for BuildKit cache mounts (`--mount=type=cache`), making Rust
//! builds significantly faster in CI:
//!
//! ```rust
//! use ansiblers_build::dockerfile::{DockerfileBuilder, BuildStage};
//!
//! let mut builder = DockerfileBuilder::new();
//! builder.syntax_buildkit();
//!
//! let mut build = BuildStage::new("build", "rust:1.78-slim");
//! build.workdir("/app");
//! build.copy(".", ".");
//! build.run_cached(
//!     "cargo build --release",
//!     &[
//!         ("/usr/local/cargo/registry", "cargo-registry"),
//!         ("/app/target",              "cargo-target"),
//!     ],
//! );
//! builder.add_stage(build);
//!
//! let mut runtime = BuildStage::new("runtime", "debian:bookworm-slim");
//! runtime.copy_from("build", "/app/target/release/ransible-playbook", "/usr/local/bin/");
//! runtime.entrypoint(&["ransible-playbook"]);
//! builder.add_stage(runtime);
//!
//! let text = builder.render();
//! assert!(text.contains("FROM rust:1.78-slim AS build"));
//! ```
//!
//! ## 2. Container-to-WASM (`c2w`)
//!
//! Constructs command-line arguments for
//! [`c2w`](https://github.com/ktock/container2wasm) to convert an OCI image
//! into a self-contained WASM binary that runs in the browser:
//!
//! ```rust
//! use ansiblers_build::c2w::C2wConfig;
//! use std::path::PathBuf;
//!
//! let cfg = C2wConfig::new("ghcr.io/ansiblers/ransible-playbook:latest")
//!     .output_dir(PathBuf::from("/dist/wasm"))
//!     .target_name("ransible-playbook");
//!
//! let args = cfg.to_args();
//! assert!(args.iter().any(|a| a.ends_with("ransible-playbook.wasm")));
//! ```
//!
//! ## 3. JupyterLite Static-Site Scaffolding (`jupyterlite`)
//!
//! Generates the minimum file skeleton to deploy a JupyterLite site with
//! embedded notebooks, Python wheels, and WASM modules — enabling
//! fully-offline, browser-based Ansible playbook demos:
//!
//! ```rust
//! use ansiblers_build::jupyterlite::{JupyterLiteScaffold, NotebookEntry};
//! use std::path::PathBuf;
//!
//! let scaffold = JupyterLiteScaffold::new("Ansiblers Demo", PathBuf::from("/tmp/site"))
//!     .add_notebook(NotebookEntry {
//!         path: PathBuf::from("demo.ipynb"),
//!         title: "Running playbooks in the browser".into(),
//!     });
//!
//! let html = scaffold.index_html();
//! assert!(html.contains("Ansiblers Demo"));
//! ```

pub mod c2w;
pub mod dockerfile;
pub mod jupyterlite;
