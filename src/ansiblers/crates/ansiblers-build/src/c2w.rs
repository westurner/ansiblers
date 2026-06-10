/// Container-to-WASM (`c2w`) artifact configuration.
///
/// `c2w` (<https://github.com/ktock/container2wasm>) converts OCI container
/// images into self-contained WASM modules that run in a browser or
/// WASI-compatible runtime.
///
/// This module does **not** execute `c2w` directly — it constructs the command
/// arguments and artifact paths so callers can integrate with their preferred
/// process-management approach (e.g. `std::process::Command`, CI scripts, or
/// the `ansiblers-build` Docker builder).
///
/// # Example
///
/// ```rust
/// use ansiblers_build::c2w::C2wConfig;
/// use std::path::PathBuf;
///
/// let cfg = C2wConfig::new("ghcr.io/ansiblers/ransible-playbook:latest")
///     .output_dir(PathBuf::from("/tmp/wasm"))
///     .target_name("ransible-playbook");
///
/// let args = cfg.to_args();
/// assert!(args.contains(&"ghcr.io/ansiblers/ransible-playbook:latest".to_string()));
/// assert!(args.iter().any(|a| a.ends_with(".wasm")));
/// ```
use std::path::{Path, PathBuf};

/// Configuration for a single `c2w` invocation.
#[derive(Debug, Clone)]
pub struct C2wConfig {
    /// Source OCI container image reference.
    pub container_image: String,
    /// Directory where the WASM output file will be placed.
    pub output_dir: PathBuf,
    /// Base name of the output artifact (without `.wasm` extension).
    pub target_name: String,
    /// Extra flags to pass through to `c2w` verbatim.
    pub extra_args: Vec<String>,
}

impl C2wConfig {
    /// Create a new configuration with defaults.
    ///
    /// Defaults: output to `./wasm_out/`, target name derived from the last
    /// segment of the image reference before any `:` tag.
    pub fn new(container_image: impl Into<String>) -> Self {
        let image = container_image.into();
        let base = image
            .split('/')
            .last()
            .unwrap_or("output")
            .split(':')
            .next()
            .unwrap_or("output")
            .to_string();
        Self {
            container_image: image,
            output_dir: PathBuf::from("wasm_out"),
            target_name: base,
            extra_args: Vec::new(),
        }
    }

    /// Override the output directory.
    pub fn output_dir(mut self, dir: PathBuf) -> Self {
        self.output_dir = dir;
        self
    }

    /// Override the output file base name.
    pub fn target_name(mut self, name: impl Into<String>) -> Self {
        self.target_name = name.into();
        self
    }

    /// Append extra arguments to the `c2w` invocation.
    pub fn extra_arg(mut self, arg: impl Into<String>) -> Self {
        self.extra_args.push(arg.into());
        self
    }

    /// Return the full path to the expected WASM output file.
    pub fn output_path(&self) -> PathBuf {
        self.output_dir.join(format!("{}.wasm", self.target_name))
    }

    /// Build the argument list for `c2w <args...>`.
    ///
    /// The returned `Vec` contains everything *after* the `c2w` binary name.
    pub fn to_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        // Extra pass-through args come first so callers can override flags.
        args.extend(self.extra_args.clone());
        args.push(self.container_image.clone());
        args.push(self.output_path().to_string_lossy().into_owned());
        args
    }
}

/// A collection of [`C2wConfig`] entries to convert as a batch.
#[derive(Debug, Default)]
pub struct C2wBatch {
    items: Vec<C2wConfig>,
}

impl C2wBatch {
    /// Create an empty batch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a configuration entry.
    pub fn add(mut self, cfg: C2wConfig) -> Self {
        self.items.push(cfg);
        self
    }

    /// Return all configurations.
    pub fn configs(&self) -> &[C2wConfig] {
        &self.items
    }

    /// Return all expected output paths.
    pub fn output_paths(&self) -> Vec<PathBuf> {
        self.items.iter().map(|c| c.output_path()).collect()
    }
}

/// Convenience: generate a `Dockerfile` stage that runs `c2w` inside a build
/// container and emits the WASM artifact.
///
/// Returns the multi-line `RUN` shell snippet (not a full Dockerfile).
pub fn c2w_dockerfile_snippet(cfg: &C2wConfig, c2w_image: &str) -> String {
    let args = cfg.to_args().join(" ");
    format!(
        "# Convert OCI image to WASM with c2w\n\
         FROM {c2w_image} AS c2w-builder\n\
         RUN c2w {args}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_target_name_derived() {
        let cfg = C2wConfig::new("ghcr.io/ansiblers/ransible:latest");
        assert_eq!(cfg.target_name, "ransible");
    }

    #[test]
    fn test_output_path() {
        let cfg = C2wConfig::new("myimage:v1")
            .output_dir(PathBuf::from("/out"))
            .target_name("app");
        assert_eq!(cfg.output_path(), PathBuf::from("/out/app.wasm"));
    }

    #[test]
    fn test_to_args_contains_image_and_output() {
        let cfg = C2wConfig::new("myimage:v1")
            .output_dir(PathBuf::from("/out"))
            .target_name("app");
        let args = cfg.to_args();
        assert!(args.contains(&"myimage:v1".to_string()));
        assert!(args.iter().any(|a| a.ends_with("app.wasm")));
    }

    #[test]
    fn test_extra_args_prepended() {
        let cfg = C2wConfig::new("img").extra_arg("--debug");
        let args = cfg.to_args();
        assert_eq!(args[0], "--debug");
    }

    #[test]
    fn test_batch_output_paths() {
        let batch = C2wBatch::new()
            .add(
                C2wConfig::new("img1")
                    .output_dir(PathBuf::from("/out"))
                    .target_name("a"),
            )
            .add(
                C2wConfig::new("img2")
                    .output_dir(PathBuf::from("/out"))
                    .target_name("b"),
            );
        let paths = batch.output_paths();
        assert_eq!(paths.len(), 2);
        assert!(paths[0].ends_with("a.wasm"));
        assert!(paths[1].ends_with("b.wasm"));
    }

    #[test]
    fn test_dockerfile_snippet() {
        let cfg = C2wConfig::new("myimage").target_name("out");
        let snippet = c2w_dockerfile_snippet(&cfg, "ghcr.io/ktock/c2w:latest");
        assert!(snippet.contains("FROM ghcr.io/ktock/c2w:latest AS c2w-builder"));
        assert!(snippet.contains("RUN c2w"));
        assert!(snippet.contains("out.wasm"));
    }
}
