/// `repo2jupyterlite` static site scaffolding.
///
/// [JupyterLite](https://jupyterlite.readthedocs.io/) is a distribution of
/// JupyterLab that runs entirely in the browser via Pyodide / Wasm.
/// This module generates the minimum static-site skeleton needed to host
/// a JupyterLite instance together with pre-built WASM modules and Python
/// wheels, so Ansible playbooks can be demonstrated/run without a server.
///
/// # Example
///
/// ```rust
/// use ansiblers_build::jupyterlite::{JupyterLiteScaffold, NotebookEntry};
/// use std::path::PathBuf;
///
/// let scaffold = JupyterLiteScaffold::new("Ansiblers Demo", PathBuf::from("/tmp/site"))
///     .add_notebook(NotebookEntry {
///         path: PathBuf::from("demo.ipynb"),
///         title: "Ansiblers demo notebook".into(),
///     });
///
/// let index = scaffold.index_html();
/// assert!(index.contains("Ansiblers Demo"));
/// assert!(index.contains("jupyterlite"));
/// ```
use std::path::{Path, PathBuf};

use anyhow::Result;

/// A notebook to include in the JupyterLite deployment.
#[derive(Debug, Clone)]
pub struct NotebookEntry {
    /// Path to the `.ipynb` file on disk (will be embedded in the site).
    pub path: PathBuf,
    /// Human-readable title shown in the launcher.
    pub title: String,
}

/// A Python wheel to pre-install in the Pyodide environment.
#[derive(Debug, Clone)]
pub struct WheelEntry {
    /// Path to the `.whl` file.
    pub path: PathBuf,
    /// Optional version override shown in `jupyter_lite_config.json`.
    pub version: Option<String>,
}

/// Static-site scaffold for a JupyterLite deployment.
#[derive(Debug)]
pub struct JupyterLiteScaffold {
    /// Site title rendered into `<title>` and the launcher header.
    pub site_name: String,
    /// Root output directory for the generated site.
    pub output_dir: PathBuf,
    /// Notebooks to include.
    pub notebooks: Vec<NotebookEntry>,
    /// Python wheels to pre-install.
    pub wheels: Vec<WheelEntry>,
    /// Pre-built WASM binary paths to embed.
    pub wasm_modules: Vec<PathBuf>,
    /// JupyterLite version pin used in CDN URLs.
    pub jupyterlite_version: String,
}

impl JupyterLiteScaffold {
    /// Create a new scaffold with sensible defaults.
    pub fn new(site_name: impl Into<String>, output_dir: PathBuf) -> Self {
        Self {
            site_name: site_name.into(),
            output_dir,
            notebooks: Vec::new(),
            wheels: Vec::new(),
            wasm_modules: Vec::new(),
            jupyterlite_version: "0.4.2".into(),
        }
    }

    /// Add a notebook entry.
    pub fn add_notebook(mut self, nb: NotebookEntry) -> Self {
        self.notebooks.push(nb);
        self
    }

    /// Add a wheel entry.
    pub fn add_wheel(mut self, whl: WheelEntry) -> Self {
        self.wheels.push(whl);
        self
    }

    /// Add a WASM binary path.
    pub fn add_wasm(mut self, path: PathBuf) -> Self {
        self.wasm_modules.push(path);
        self
    }

    /// Override the JupyterLite CDN version.
    pub fn jupyterlite_version(mut self, version: impl Into<String>) -> Self {
        self.jupyterlite_version = version.into();
        self
    }

    // ------------------------------------------------------------------
    // Rendering helpers
    // ------------------------------------------------------------------

    /// Render the `index.html` entry-point.
    pub fn index_html(&self) -> String {
        let ver = &self.jupyterlite_version;
        let cdn = format!("https://cdn.jsdelivr.net/npm/@jupyterlite/server@{ver}/dist");
        let title = html_escape(&self.site_name);
        let notebook_links: String = self
            .notebooks
            .iter()
            .map(|nb| {
                let name = nb.path.file_name().unwrap_or_default().to_string_lossy();
                let t = html_escape(&nb.title);
                format!(
                    r#"    <li><a href="lab/index.html?path={name}">{t}</a></li>"#,
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>{title}</title>
  <script src="{cdn}/repl/index.js" type="module"></script>
</head>
<body>
  <h1>{title}</h1>
  <p>Powered by <a href="https://jupyterlite.readthedocs.io/">jupyterlite</a>.</p>
  <ul>
{notebook_links}
  </ul>
  <script type="module">
    import {{ JupyterLiteServer }} from '{cdn}/index.js';
    const server = new JupyterLiteServer({{ rootUrl: '.' }});
    server.initialize().then(() => server.start());
  </script>
</body>
</html>
"#
        )
    }

    /// Render `jupyter_lite_config.json` (wheel pinning and WASM registration).
    pub fn config_json(&self) -> serde_json::Value {
        let wheels: Vec<serde_json::Value> = self
            .wheels
            .iter()
            .map(|w| {
                let name = w.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                if let Some(ver) = &w.version {
                    serde_json::json!({ "name": name, "version": ver })
                } else {
                    serde_json::json!(name)
                }
            })
            .collect();

        let wasm_paths: Vec<String> = self
            .wasm_modules
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();

        serde_json::json!({
            "LiteBuildConfig": {
                "federated_extensions": [],
                "piplite_urls": wheels,
                "wasm_modules": wasm_paths,
            }
        })
    }

    /// Render `overrides.json` to configure the JupyterLite labextension.
    pub fn overrides_json(&self) -> serde_json::Value {
        serde_json::json!({
            "@jupyterlite/application-extension:page-config": {
                "appName": self.site_name
            }
        })
    }

    /// Write the complete scaffold to [`Self::output_dir`].
    ///
    /// Creates the directory structure:
    /// ```text
    /// <output_dir>/
    ///   index.html
    ///   jupyter_lite_config.json
    ///   overrides.json
    ///   files/          ← notebooks copied here
    ///   extensions/     ← wheels copied here
    ///   wasm/           ← WASM modules copied here
    /// ```
    pub fn generate(&self) -> Result<()> {
        let out = &self.output_dir;
        std::fs::create_dir_all(out)?;
        std::fs::create_dir_all(out.join("files"))?;
        std::fs::create_dir_all(out.join("extensions"))?;
        std::fs::create_dir_all(out.join("wasm"))?;

        // Write index.html
        std::fs::write(out.join("index.html"), self.index_html())?;

        // Write jupyter_lite_config.json
        let config = serde_json::to_string_pretty(&self.config_json())?;
        std::fs::write(out.join("jupyter_lite_config.json"), config)?;

        // Write overrides.json
        let overrides = serde_json::to_string_pretty(&self.overrides_json())?;
        std::fs::write(out.join("overrides.json"), overrides)?;

        // Copy notebooks
        for nb in &self.notebooks {
            if nb.path.exists() {
                let dst = out.join("files").join(nb.path.file_name().unwrap_or_default());
                std::fs::copy(&nb.path, dst)?;
            }
        }

        // Copy wheels
        for whl in &self.wheels {
            if whl.path.exists() {
                let dst = out.join("extensions").join(whl.path.file_name().unwrap_or_default());
                std::fs::copy(&whl.path, dst)?;
            }
        }

        // Copy WASM modules
        for wasm in &self.wasm_modules {
            if wasm.exists() {
                let dst = out.join("wasm").join(wasm.file_name().unwrap_or_default());
                std::fs::copy(wasm, dst)?;
            }
        }

        tracing::info!("JupyterLite scaffold generated at {}", out.display());
        Ok(())
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_index_html_contains_title() {
        let scaffold = JupyterLiteScaffold::new("My Ansible Demo", PathBuf::from("/tmp/out"));
        let html = scaffold.index_html();
        assert!(html.contains("My Ansible Demo"));
        assert!(html.contains("jupyterlite"));
    }

    #[test]
    fn test_index_html_notebook_links() {
        let scaffold = JupyterLiteScaffold::new("Demo", PathBuf::from("/tmp/out")).add_notebook(
            NotebookEntry { path: PathBuf::from("playbook.ipynb"), title: "Run Playbook".into() },
        );
        let html = scaffold.index_html();
        assert!(html.contains("playbook.ipynb"));
        assert!(html.contains("Run Playbook"));
    }

    #[test]
    fn test_config_json_wheels() {
        let scaffold =
            JupyterLiteScaffold::new("T", PathBuf::from("/tmp/out")).add_wheel(WheelEntry {
                path: PathBuf::from("ansiblers-0.1.0-py3-none-any.whl"),
                version: Some("0.1.0".into()),
            });
        let cfg = scaffold.config_json();
        let urls = cfg["LiteBuildConfig"]["piplite_urls"].as_array().unwrap();
        assert!(!urls.is_empty());
    }

    #[test]
    fn test_generate_creates_dirs() {
        let tmp = TempDir::new().unwrap();
        let scaffold = JupyterLiteScaffold::new("Test", tmp.path().to_path_buf());
        scaffold.generate().unwrap();
        assert!(tmp.path().join("index.html").exists());
        assert!(tmp.path().join("jupyter_lite_config.json").exists());
        assert!(tmp.path().join("overrides.json").exists());
        assert!(tmp.path().join("files").is_dir());
        assert!(tmp.path().join("extensions").is_dir());
        assert!(tmp.path().join("wasm").is_dir());
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("A & B"), "A &amp; B");
    }

    #[test]
    fn test_overrides_json() {
        let scaffold = JupyterLiteScaffold::new("Ansiblers", PathBuf::from("/tmp"));
        let json = scaffold.overrides_json();
        assert_eq!(
            json["@jupyterlite/application-extension:page-config"]["appName"],
            "Ansiblers"
        );
    }
}
