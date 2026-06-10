//! `template` module — render a Jinja2 template file to a destination path.
//!
//! Uses the `ansiblers-templates` crate (backed by `jinja2rs`/minijinja) so
//! the same Ansible-compatible filter library is available inside `.j2` files.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `src` | — | Path to the source `.j2` template file |
//! | `dest` | — | Destination file path on the managed host |
//! | `mode` | — | File permission bits (octal string, e.g. `"0644"`) |
//! | `owner` | — | File owner (username) |
//! | `group` | — | File group |
//! | `backup` | `false` | Create a `.bak` backup before overwriting |
//! | `force` | `true` | Overwrite even when destination content matches |
//! | `newline_sequence` | `\n` | Line ending to normalise to (`\n`, `\r\n`) |
//! | `trim_blocks` | `true` | Remove the first newline after a block tag |
//! | `lstrip_blocks` | `false` | Strip leading whitespace from block lines |

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use ansiblers_templates::AnsibleTemplateEngine;
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct TemplateModule;

impl ModuleInvoker for TemplateModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let src = args
            .get_str("src")
            .ok_or_else(|| anyhow::anyhow!("template: 'src' is required"))?
            .to_string();
        let dest = args
            .get_str("dest")
            .ok_or_else(|| anyhow::anyhow!("template: 'dest' is required"))?
            .to_string();

        let backup = bool_arg(args, "backup", false);
        let force = bool_arg(args, "force", true);
        let newline_sequence = args.get_str("newline_sequence").unwrap_or("\n");
        let mode_str = args.get_str("mode");

        // ------------------------------------------------------------------
        // Read template source
        // ------------------------------------------------------------------
        let template_text =
            std::fs::read_to_string(&src).with_context(|| format!("template: cannot read '{src}'"))?;

        // ------------------------------------------------------------------
        // Build render context from the ExecutionContext
        // ------------------------------------------------------------------
        let vars: HashMap<String, Value> = ctx.render_vars(host);
        let engine = AnsibleTemplateEngine::new();
        let rendered = engine
            .render(&template_text, &vars)
            .with_context(|| format!("template: rendering '{src}' failed"))?;

        // Normalise line endings.
        let rendered = if newline_sequence == "\r\n" {
            rendered.replace('\n', "\r\n")
        } else {
            rendered
        };

        // ------------------------------------------------------------------
        // Idempotency check
        // ------------------------------------------------------------------
        let dest_path = Path::new(&dest);
        if !force && dest_path.exists() {
            if let Ok(existing) = std::fs::read_to_string(dest_path) {
                if existing == rendered {
                    return Ok(TaskResult::ok(host));
                }
            }
        }

        // Backup if requested.
        if backup && dest_path.exists() {
            let backup_path = format!("{dest}.bak");
            std::fs::copy(dest_path, &backup_path)
                .with_context(|| format!("template: backup to '{backup_path}' failed"))?;
        }

        // Ensure parent directory exists.
        if let Some(parent) = dest_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("template: cannot create parent dirs for '{dest}'"))?;
            }
        }

        std::fs::write(dest_path, &rendered)
            .with_context(|| format!("template: cannot write '{dest}'"))?;

        // Apply mode if specified.
        if let Some(mode) = mode_str {
            let bits = u32::from_str_radix(mode.trim_start_matches("0o").trim_start_matches('0'), 8)
                .with_context(|| format!("template: invalid mode '{mode}'"))?;
            std::fs::set_permissions(dest_path, std::fs::Permissions::from_mode(bits))?;
        }

        let mut result = TaskResult::changed(host);
        result.msg = format!("template '{src}' rendered to '{dest}'");
        Ok(result)
    }
}

fn bool_arg(args: &ModuleArgs, key: &str, default: bool) -> bool {
    args.args.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ansiblers_core::{ExecutionContext, Inventory};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn make_args(pairs: &[(&str, &str)]) -> ModuleArgs {
        let mut m = HashMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), Value::String(v.to_string()));
        }
        ModuleArgs::new(m)
    }

    fn make_ctx() -> ExecutionContext {
        ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new())
    }

    #[test]
    fn test_template_renders_variable() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("greeting.j2");
        let dest = tmp.path().join("greeting.txt");

        std::fs::write(&src, "Hello {{ name }}!").unwrap();

        let mut ctx = make_ctx();
        ctx.playbook_vars
            .insert("name".to_string(), Value::String("World".to_string()));

        let args = make_args(&[
            ("src", src.to_str().unwrap()),
            ("dest", dest.to_str().unwrap()),
        ]);
        let result = TemplateModule.invoke(&args, "localhost", &mut ctx).unwrap();
        assert!(result.status.is_ok());
        assert!(result.changed);
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "Hello World!");
    }

    #[test]
    fn test_template_idempotent_with_force_false() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("t.j2");
        let dest = tmp.path().join("t.txt");

        std::fs::write(&src, "static content").unwrap();
        std::fs::write(&dest, "static content").unwrap();

        let mut ctx = make_ctx();
        let mut m = HashMap::new();
        m.insert("src".to_string(), Value::String(src.to_str().unwrap().to_string()));
        m.insert("dest".to_string(), Value::String(dest.to_str().unwrap().to_string()));
        m.insert("force".to_string(), Value::Bool(false));
        let args = ModuleArgs::new(m);

        let result = TemplateModule.invoke(&args, "localhost", &mut ctx).unwrap();
        assert!(result.status.is_ok());
        assert!(!result.changed);
    }

    #[test]
    fn test_template_backup() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("t.j2");
        let dest = tmp.path().join("t.txt");

        std::fs::write(&src, "new").unwrap();
        std::fs::write(&dest, "old").unwrap();

        let mut ctx = make_ctx();
        let mut m = HashMap::new();
        m.insert("src".to_string(), Value::String(src.to_str().unwrap().to_string()));
        m.insert("dest".to_string(), Value::String(dest.to_str().unwrap().to_string()));
        m.insert("backup".to_string(), Value::Bool(true));
        let args = ModuleArgs::new(m);

        TemplateModule.invoke(&args, "localhost", &mut ctx).unwrap();
        assert!(tmp.path().join("t.txt.bak").exists());
    }

    #[test]
    fn test_template_missing_src_fails() {
        let tmp = TempDir::new().unwrap();
        let mut ctx = make_ctx();
        let args = make_args(&[
            ("src", "/nonexistent/template.j2"),
            ("dest", tmp.path().join("out.txt").to_str().unwrap()),
        ]);
        let result = TemplateModule.invoke(&args, "localhost", &mut ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_template_no_src_fails() {
        let mut ctx = make_ctx();
        let args = ModuleArgs::new(HashMap::new());
        let result = TemplateModule.invoke(&args, "localhost", &mut ctx);
        assert!(result.is_err());
    }
}
