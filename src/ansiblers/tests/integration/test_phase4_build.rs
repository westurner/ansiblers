//! Phase 4 integration tests — Artifact generation.
//!
//! Tests for `ansiblers-build`: Dockerfile builder, c2w, and JupyterLite
//! scaffolding.

use ansiblers_build::{
    c2w::{C2wBatch, C2wConfig},
    dockerfile::{BuildStage, DockerfileBuilder},
    jupyterlite::{JupyterLiteScaffold, NotebookEntry, WheelEntry},
};
use rstest::rstest;
use std::path::PathBuf;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Dockerfile builder
// ---------------------------------------------------------------------------

#[test]
fn test_dockerfile_basic_stages() {
    let mut builder = DockerfileBuilder::new();
    builder.syntax_buildkit();

    let mut build = BuildStage::new("build", "rust:1.78-slim");
    build.workdir("/app");
    build.copy(".", ".");
    build.run_cached(
        "cargo build --release",
        &[("/usr/local/cargo/registry", "cargo-registry"), ("/app/target", "cargo-target")],
    );
    builder.add_stage(build);

    let mut runtime = BuildStage::new("runtime", "debian:bookworm-slim");
    runtime.copy_from("build", "/app/target/release/ransible-playbook", "/usr/local/bin/");
    runtime.expose(22);
    runtime.env("RUST_LOG", "info");
    runtime.entrypoint(&["ransible-playbook"]);
    builder.add_stage(runtime);

    let text = builder.render();

    // Preamble
    assert!(text.contains("# syntax=docker/dockerfile:1"), "missing BuildKit syntax directive");

    // Build stage
    assert!(text.contains("FROM rust:1.78-slim AS build"));
    assert!(text.contains("WORKDIR /app"));
    assert!(text.contains("--mount=type=cache,id=cargo-registry"));
    assert!(text.contains("--mount=type=cache,id=cargo-target"));
    assert!(text.contains("cargo build --release"));

    // Runtime stage
    assert!(text.contains("FROM debian:bookworm-slim AS runtime"));
    assert!(text.contains("COPY --from=build /app/target/release/ransible-playbook /usr/local/bin/"));
    assert!(text.contains("EXPOSE 22"));
    assert!(text.contains("ENV RUST_LOG=info"));
    assert!(text.contains(r#"ENTRYPOINT ["ransible-playbook"]"#));
}

#[test]
fn test_dockerfile_write_to_file() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("Dockerfile");

    let mut builder = DockerfileBuilder::new();
    let mut stage = BuildStage::new("app", "alpine:3.20");
    stage.run("echo hello");
    stage.cmd(&["sh"]);
    builder.add_stage(stage);

    builder.write_to(&path).unwrap();
    assert!(path.exists());
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("FROM alpine:3.20 AS app"));
    assert!(content.contains(r#"CMD ["sh"]"#));
}

#[rstest]
#[case("ubuntu:22.04", "runner", "apt-get install -y curl")]
#[case("alpine:3.20",  "runner", "apk add --no-cache curl")]
fn test_dockerfile_parametrized_stages(
    #[case] base: &str,
    #[case] name: &str,
    #[case] cmd: &str,
) {
    let mut stage = BuildStage::new(name, base);
    stage.run(cmd);
    let text = stage.render();
    assert!(text.contains(&format!("FROM {base} AS {name}")));
    assert!(text.contains(&format!("RUN {cmd}")));
}

// ---------------------------------------------------------------------------
// C2w config
// ---------------------------------------------------------------------------

#[test]
fn test_c2w_output_path() {
    let cfg = C2wConfig::new("myimage:v1.0")
        .output_dir(PathBuf::from("/dist/wasm"))
        .target_name("ransible-playbook");
    assert_eq!(cfg.output_path(), PathBuf::from("/dist/wasm/ransible-playbook.wasm"));
}

#[test]
fn test_c2w_target_derived_from_image() {
    // "ghcr.io/foo/ransible:latest" → "ransible"
    let cfg = C2wConfig::new("ghcr.io/foo/ransible:latest");
    assert_eq!(cfg.target_name, "ransible");
}

#[test]
fn test_c2w_to_args_order() {
    let cfg =
        C2wConfig::new("img:v1").output_dir(PathBuf::from("/out")).target_name("app").extra_arg("--debug");
    let args = cfg.to_args();
    assert_eq!(args[0], "--debug", "extra args should come first");
    assert_eq!(args[1], "img:v1", "image should be second");
    assert!(args[2].ends_with("app.wasm"), "output path should be last");
}

#[test]
fn test_c2w_batch() {
    let batch = C2wBatch::new()
        .add(
            C2wConfig::new("img1:latest")
                .output_dir(PathBuf::from("/dist"))
                .target_name("binary1"),
        )
        .add(
            C2wConfig::new("img2:latest")
                .output_dir(PathBuf::from("/dist"))
                .target_name("binary2"),
        );
    assert_eq!(batch.configs().len(), 2);
    let paths = batch.output_paths();
    assert!(paths.iter().any(|p| p.ends_with("binary1.wasm")));
    assert!(paths.iter().any(|p| p.ends_with("binary2.wasm")));
}

// ---------------------------------------------------------------------------
// JupyterLite scaffolding
// ---------------------------------------------------------------------------

#[test]
fn test_jupyterlite_generate_structure() {
    let tmp = TempDir::new().unwrap();

    let scaffold = JupyterLiteScaffold::new("Ansiblers Demo", tmp.path().to_path_buf())
        .jupyterlite_version("0.4.2");
    scaffold.generate().unwrap();

    assert!(tmp.path().join("index.html").exists(), "index.html missing");
    assert!(tmp.path().join("jupyter_lite_config.json").exists(), "config missing");
    assert!(tmp.path().join("overrides.json").exists(), "overrides missing");
    assert!(tmp.path().join("files").is_dir(), "files/ dir missing");
    assert!(tmp.path().join("extensions").is_dir(), "extensions/ dir missing");
    assert!(tmp.path().join("wasm").is_dir(), "wasm/ dir missing");
}

#[test]
fn test_jupyterlite_index_html_content() {
    let scaffold = JupyterLiteScaffold::new("Ransible Notebooks", PathBuf::from("/tmp/unused"))
        .add_notebook(NotebookEntry {
            path: PathBuf::from("demo.ipynb"),
            title: "Demo Playbook Notebook".into(),
        });
    let html = scaffold.index_html();
    assert!(html.contains("Ransible Notebooks"));
    assert!(html.contains("demo.ipynb"));
    assert!(html.contains("Demo Playbook Notebook"));
    assert!(html.contains("jupyterlite"));
}

#[test]
fn test_jupyterlite_config_json_with_wheel() {
    let scaffold =
        JupyterLiteScaffold::new("T", PathBuf::from("/tmp/unused")).add_wheel(WheelEntry {
            path: PathBuf::from("ansiblers_compat-0.1.0-py3-none-any.whl"),
            version: Some("0.1.0".into()),
        });
    let cfg = scaffold.config_json();
    let urls = cfg["LiteBuildConfig"]["piplite_urls"].as_array().unwrap();
    assert_eq!(urls.len(), 1);
}

#[test]
fn test_jupyterlite_config_json_with_wasm() {
    let scaffold = JupyterLiteScaffold::new("T", PathBuf::from("/tmp/unused"))
        .add_wasm(PathBuf::from("/dist/ransible-playbook.wasm"));
    let cfg = scaffold.config_json();
    let wasm = cfg["LiteBuildConfig"]["wasm_modules"].as_array().unwrap();
    assert_eq!(wasm.len(), 1);
    assert!(wasm[0].as_str().unwrap().ends_with("ransible-playbook.wasm"));
}

#[test]
fn test_jupyterlite_overrides_site_name() {
    let scaffold = JupyterLiteScaffold::new("My Ansible Site", PathBuf::from("/tmp/unused"));
    let json = scaffold.overrides_json();
    assert_eq!(
        json["@jupyterlite/application-extension:page-config"]["appName"],
        "My Ansible Site"
    );
}
