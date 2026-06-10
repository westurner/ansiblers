/// Multi-stage Dockerfile builder with BuildKit cache mount support.
///
/// # Example
///
/// ```rust
/// use ansiblers_build::dockerfile::{DockerfileBuilder, BuildStage, Instruction};
///
/// let mut builder = DockerfileBuilder::new();
/// builder.syntax_buildkit();
///
/// let mut stage = BuildStage::new("builder", "rust:1.78-slim");
/// stage.workdir("/app");
/// stage.run_cached(
///     "cargo fetch",
///     &[("/usr/local/cargo/registry", "cargo-registry")],
/// );
/// stage.run("cargo build --release");
/// builder.add_stage(stage);
///
/// let mut final_stage = BuildStage::new("runtime", "debian:bookworm-slim");
/// final_stage.copy_from("builder", "/app/target/release/ransible-playbook", "/usr/local/bin/");
/// final_stage.entrypoint(&["ransible-playbook"]);
/// builder.add_stage(final_stage);
///
/// let text = builder.render();
/// assert!(text.contains("FROM rust:1.78-slim AS builder"));
/// assert!(text.contains("RUN --mount=type=cache"));
/// ```
use std::fmt::Write as _;

/// A single `FROM` stage in a multi-stage Dockerfile.
#[derive(Debug, Clone)]
pub struct BuildStage {
    /// Stage name (used in `AS <name>` and `--from=<name>`).
    pub name: String,
    /// Base image.
    pub from: String,
    instructions: Vec<Instruction>,
}

impl BuildStage {
    /// Create a new named build stage.
    pub fn new(name: impl Into<String>, from: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            from: from.into(),
            instructions: Vec::new(),
        }
    }

    /// Append a `WORKDIR` instruction.
    pub fn workdir(&mut self, path: impl Into<String>) -> &mut Self {
        self.instructions.push(Instruction::Workdir(path.into()));
        self
    }

    /// Append an `ENV` instruction.
    pub fn env(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.instructions
            .push(Instruction::Env(key.into(), value.into()));
        self
    }

    /// Append an `ARG` instruction with optional default.
    pub fn arg(&mut self, name: impl Into<String>, default: Option<String>) -> &mut Self {
        self.instructions
            .push(Instruction::Arg(name.into(), default));
        self
    }

    /// Append a plain `RUN` instruction.
    pub fn run(&mut self, cmd: impl Into<String>) -> &mut Self {
        self.instructions.push(Instruction::Run(cmd.into()));
        self
    }

    /// Append a `RUN` instruction with BuildKit `--mount=type=cache` mounts.
    ///
    /// `caches` is a slice of `(host_path, cache_id)` pairs.
    pub fn run_cached(&mut self, cmd: impl Into<String>, caches: &[(&str, &str)]) -> &mut Self {
        let mounts: Vec<(String, String)> = caches
            .iter()
            .map(|(p, id)| (p.to_string(), id.to_string()))
            .collect();
        self.instructions.push(Instruction::RunCached {
            cmd: cmd.into(),
            caches: mounts,
        });
        self
    }

    /// Append a `COPY` instruction.
    pub fn copy(&mut self, src: impl Into<String>, dst: impl Into<String>) -> &mut Self {
        self.instructions.push(Instruction::Copy {
            src: src.into(),
            dst: dst.into(),
            from_stage: None,
        });
        self
    }

    /// Append a `COPY --from=<stage>` instruction.
    pub fn copy_from(
        &mut self,
        stage: impl Into<String>,
        src: impl Into<String>,
        dst: impl Into<String>,
    ) -> &mut Self {
        self.instructions.push(Instruction::Copy {
            src: src.into(),
            dst: dst.into(),
            from_stage: Some(stage.into()),
        });
        self
    }

    /// Append an `EXPOSE` instruction.
    pub fn expose(&mut self, port: u16) -> &mut Self {
        self.instructions.push(Instruction::Expose(port));
        self
    }

    /// Append an `ENTRYPOINT` instruction (exec form).
    pub fn entrypoint(&mut self, args: &[&str]) -> &mut Self {
        self.instructions.push(Instruction::Entrypoint(
            args.iter().map(|s| s.to_string()).collect(),
        ));
        self
    }

    /// Append a `CMD` instruction (exec form).
    pub fn cmd(&mut self, args: &[&str]) -> &mut Self {
        self.instructions.push(Instruction::Cmd(
            args.iter().map(|s| s.to_string()).collect(),
        ));
        self
    }

    /// Append a `LABEL` instruction.
    pub fn label(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.instructions
            .push(Instruction::Label(key.into(), value.into()));
        self
    }

    /// Render this stage to Dockerfile text.
    pub fn render(&self) -> String {
        let mut out = format!("FROM {} AS {}\n", self.from, self.name);
        for instr in &self.instructions {
            out.push_str(&instr.render());
            out.push('\n');
        }
        out
    }
}

/// A single Dockerfile instruction.
#[derive(Debug, Clone)]
pub enum Instruction {
    Run(String),
    RunCached {
        cmd: String,
        caches: Vec<(String, String)>,
    },
    Copy {
        src: String,
        dst: String,
        from_stage: Option<String>,
    },
    Env(String, String),
    Arg(String, Option<String>),
    Workdir(String),
    Expose(u16),
    Entrypoint(Vec<String>),
    Cmd(Vec<String>),
    Label(String, String),
}

impl Instruction {
    fn render(&self) -> String {
        match self {
            Instruction::Run(cmd) => format!("RUN {cmd}"),
            Instruction::RunCached { cmd, caches } => {
                let mounts: String = caches
                    .iter()
                    .map(|(path, id)| format!("--mount=type=cache,id={id},target={path}"))
                    .collect::<Vec<_>>()
                    .join(" \\\n    ");
                format!("RUN {mounts} \\\n    {cmd}")
            }
            Instruction::Copy {
                src,
                dst,
                from_stage: None,
            } => format!("COPY {src} {dst}"),
            Instruction::Copy {
                src,
                dst,
                from_stage: Some(stage),
            } => {
                format!("COPY --from={stage} {src} {dst}")
            }
            Instruction::Env(k, v) => format!("ENV {k}={v}"),
            Instruction::Arg(name, None) => format!("ARG {name}"),
            Instruction::Arg(name, Some(default)) => format!("ARG {name}={default}"),
            Instruction::Workdir(p) => format!("WORKDIR {p}"),
            Instruction::Expose(port) => format!("EXPOSE {port}"),
            Instruction::Entrypoint(args) => {
                let json = json_array(args);
                format!("ENTRYPOINT {json}")
            }
            Instruction::Cmd(args) => {
                let json = json_array(args);
                format!("CMD {json}")
            }
            Instruction::Label(k, v) => format!("LABEL {k}={v:?}"),
        }
    }
}

fn json_array(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|s| format!("{s:?}")).collect();
    format!("[{}]", quoted.join(", "))
}

/// Builder for a complete multi-stage Dockerfile.
///
/// # Example
///
/// ```rust
/// use ansiblers_build::dockerfile::DockerfileBuilder;
///
/// let builder = DockerfileBuilder::new();
/// // The preamble is empty by default (no BuildKit syntax directive).
/// let text = builder.render();
/// assert_eq!(text, "");
/// ```
#[derive(Debug, Default)]
pub struct DockerfileBuilder {
    buildkit_syntax: bool,
    stages: Vec<BuildStage>,
}

impl DockerfileBuilder {
    /// Create a new, empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Prepend the BuildKit syntax directive (`# syntax=docker/dockerfile:1`).
    pub fn syntax_buildkit(&mut self) -> &mut Self {
        self.buildkit_syntax = true;
        self
    }

    /// Append a build stage.
    pub fn add_stage(&mut self, stage: BuildStage) -> &mut Self {
        self.stages.push(stage);
        self
    }

    /// Render the complete Dockerfile as a string.
    pub fn render(&self) -> String {
        let mut out = String::new();
        if self.buildkit_syntax {
            out.push_str("# syntax=docker/dockerfile:1\n");
        }
        for stage in &self.stages {
            if !out.is_empty() {
                out.push('\n');
            }
            let _ = write!(out, "{}", stage.render());
        }
        out
    }

    /// Write the rendered Dockerfile to `path`.
    pub fn write_to(&self, path: &std::path::Path) -> anyhow::Result<()> {
        std::fs::write(path, self.render())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_dockerfile() {
        let mut builder = DockerfileBuilder::new();
        let mut stage = BuildStage::new("app", "ubuntu:22.04");
        stage.run("apt-get update");
        stage.expose(8080);
        builder.add_stage(stage);
        let text = builder.render();
        assert!(text.contains("FROM ubuntu:22.04 AS app"));
        assert!(text.contains("RUN apt-get update"));
        assert!(text.contains("EXPOSE 8080"));
    }

    #[test]
    fn test_buildkit_cache_mount() {
        let mut builder = DockerfileBuilder::new();
        builder.syntax_buildkit();
        let mut stage = BuildStage::new("build", "rust:1.78-slim");
        stage.run_cached(
            "cargo build --release",
            &[("/usr/local/cargo/registry", "cargo-reg")],
        );
        builder.add_stage(stage);
        let text = builder.render();
        assert!(text.contains("# syntax=docker/dockerfile:1"));
        assert!(text.contains("--mount=type=cache,id=cargo-reg,target=/usr/local/cargo/registry"));
    }

    #[test]
    fn test_multistage_copy_from() {
        let mut builder = DockerfileBuilder::new();
        let mut build = BuildStage::new("build", "rust:slim");
        build.run("cargo build --release");
        builder.add_stage(build);

        let mut runtime = BuildStage::new("runtime", "debian:slim");
        runtime.copy_from("build", "/app/target/release/app", "/usr/local/bin/app");
        runtime.entrypoint(&["/usr/local/bin/app"]);
        builder.add_stage(runtime);

        let text = builder.render();
        assert!(text.contains("COPY --from=build /app/target/release/app /usr/local/bin/app"));
        assert!(text.contains(r#"ENTRYPOINT ["/usr/local/bin/app"]"#));
    }

    #[test]
    fn test_env_label_arg() {
        let mut stage = BuildStage::new("s", "alpine");
        stage.env("RUST_LOG", "info");
        stage.label("maintainer", "ansiblers");
        stage.arg("VERSION", Some("0.1.0".into()));
        let text = stage.render();
        assert!(text.contains("ENV RUST_LOG=info"));
        assert!(text.contains("LABEL maintainer="));
        assert!(text.contains("ARG VERSION=0.1.0"));
    }

    #[test]
    fn test_write_to_temp() {
        use tempfile::NamedTempFile;
        let mut builder = DockerfileBuilder::new();
        let mut stage = BuildStage::new("test", "alpine");
        stage.cmd(&["echo", "hello"]);
        builder.add_stage(stage);
        let f = NamedTempFile::new().unwrap();
        builder.write_to(f.path()).unwrap();
        let content = std::fs::read_to_string(f.path()).unwrap();
        assert!(content.contains(r#"CMD ["echo", "hello"]"#));
    }
}
