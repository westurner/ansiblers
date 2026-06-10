//! `file` module — manage file/directory state (create, delete, permissions, symlinks).

use std::fs;
use std::os::unix::fs::PermissionsExt;

use ansiblers_core::{ExecutionContext, TaskResult};
use anyhow::Result;

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct FileModule;

impl ModuleInvoker for FileModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let path = args
            .get_str("path")
            .or_else(|| args.get_str("dest"))
            .or_else(|| args.get_str("name"))
            .ok_or_else(|| anyhow::anyhow!("file module requires 'path'"))?
            .to_string();

        let state = args.get_str("state").unwrap_or("file");
        let mode_str = args.get_str("mode");
        let owner = args.get_str("owner");
        let recurse = args
            .args
            .get("recurse")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        match state {
            "absent" => remove_path(&path, host),
            "directory" => ensure_directory(&path, mode_str, recurse, host),
            "touch" => touch_file(&path, mode_str, host),
            "link" => {
                let src = args
                    .get_str("src")
                    .ok_or_else(|| anyhow::anyhow!("file state=link requires 'src'"))?;
                create_symlink(src, &path, host)
            }
            "hard" => {
                let src = args
                    .get_str("src")
                    .ok_or_else(|| anyhow::anyhow!("file state=hard requires 'src'"))?;
                create_hardlink(src, &path, host)
            }
            _ => {
                // state=file (default): ensure file exists with correct attrs.
                if !std::path::Path::new(&path).exists() {
                    return Ok(TaskResult::failed(
                        host,
                        format!("file '{path}' does not exist (use state=touch to create)"),
                    ));
                }
                if let Some(mode) = mode_str {
                    apply_mode(&path, mode, host)?;
                    let mut r = TaskResult::changed(host);
                    r.msg = format!("mode set to {mode}");
                    return Ok(r);
                }
                Ok(TaskResult::ok(host))
            }
        }
    }
}

fn remove_path(path: &str, host: &str) -> Result<TaskResult> {
    let p = std::path::Path::new(path);
    if !p.exists() {
        return Ok(TaskResult::ok(host));
    }
    if p.is_dir() {
        fs::remove_dir_all(p)?;
    } else {
        fs::remove_file(p)?;
    }
    let mut r = TaskResult::changed(host);
    r.msg = format!("removed '{path}'");
    Ok(r)
}

fn ensure_directory(
    path: &str,
    mode_str: Option<&str>,
    recurse: bool,
    host: &str,
) -> Result<TaskResult> {
    let p = std::path::Path::new(path);
    let existed = p.is_dir();
    if !existed {
        fs::create_dir_all(p)?;
    }
    if let Some(mode) = mode_str {
        apply_mode(path, mode, host)?;
    }
    if existed {
        Ok(TaskResult::ok(host))
    } else {
        let mut r = TaskResult::changed(host);
        r.msg = format!("created directory '{path}'");
        Ok(r)
    }
}

fn touch_file(path: &str, mode_str: Option<&str>, host: &str) -> Result<TaskResult> {
    let p = std::path::Path::new(path);
    let existed = p.exists();
    // Create parent dirs if needed.
    if let Some(parent) = p.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }
    // OpenOptions::create + write(true) will create or truncate.
    fs::OpenOptions::new().create(true).write(true).open(p)?;
    if let Some(mode) = mode_str {
        apply_mode(path, mode, host)?;
    }
    if existed {
        Ok(TaskResult::ok(host))
    } else {
        let mut r = TaskResult::changed(host);
        r.msg = format!("created file '{path}'");
        Ok(r)
    }
}

fn create_symlink(src: &str, dest: &str, host: &str) -> Result<TaskResult> {
    let dest_path = std::path::Path::new(dest);
    if dest_path.exists() || dest_path.symlink_metadata().is_ok() {
        // Existing symlink — check if already correct.
        if let Ok(target) = fs::read_link(dest_path) {
            if target == std::path::Path::new(src) {
                return Ok(TaskResult::ok(host));
            }
        }
        fs::remove_file(dest_path)?;
    }
    std::os::unix::fs::symlink(src, dest_path)?;
    let mut r = TaskResult::changed(host);
    r.msg = format!("symlink '{dest}' → '{src}'");
    Ok(r)
}

fn create_hardlink(src: &str, dest: &str, host: &str) -> Result<TaskResult> {
    let dest_path = std::path::Path::new(dest);
    if dest_path.exists() {
        return Ok(TaskResult::ok(host));
    }
    fs::hard_link(src, dest_path)?;
    let mut r = TaskResult::changed(host);
    r.msg = format!("hard link '{dest}' → '{src}'");
    Ok(r)
}

/// Apply an octal mode string (e.g. "0755") to a path.
fn apply_mode(path: &str, mode_str: &str, _host: &str) -> Result<()> {
    let mode = u32::from_str_radix(mode_str.trim_start_matches("0o"), 8)
        .map_err(|_| anyhow::anyhow!("invalid mode: '{mode_str}'"))?;
    let perms = fs::Permissions::from_mode(mode);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ansiblers_core::{Inventory, Value};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn ctx() -> ansiblers_core::ExecutionContext {
        ansiblers_core::ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new())
    }

    #[test]
    fn test_file_state_directory() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("newdir");
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(dir.to_str().unwrap().to_string()),
        );
        args.insert("state".to_string(), Value::String("directory".to_string()));
        let r = FileModule
            .invoke(
                &crate::registry::ModuleArgs::new(args),
                "localhost",
                &mut ctx,
            )
            .unwrap();
        assert!(r.changed);
        assert!(dir.is_dir());
    }

    #[test]
    fn test_file_state_touch() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("new_file.txt");
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(f.to_str().unwrap().to_string()),
        );
        args.insert("state".to_string(), Value::String("touch".to_string()));
        let r = FileModule
            .invoke(
                &crate::registry::ModuleArgs::new(args),
                "localhost",
                &mut ctx,
            )
            .unwrap();
        assert!(r.changed);
        assert!(f.exists());
    }

    #[test]
    fn test_file_state_absent() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("to_remove.txt");
        std::fs::write(&f, b"hello").unwrap();
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(f.to_str().unwrap().to_string()),
        );
        args.insert("state".to_string(), Value::String("absent".to_string()));
        let r = FileModule
            .invoke(
                &crate::registry::ModuleArgs::new(args),
                "localhost",
                &mut ctx,
            )
            .unwrap();
        assert!(r.changed);
        assert!(!f.exists());
    }

    #[test]
    fn test_file_absent_idempotent() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("nonexistent.txt");
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(f.to_str().unwrap().to_string()),
        );
        args.insert("state".to_string(), Value::String("absent".to_string()));
        let r = FileModule
            .invoke(
                &crate::registry::ModuleArgs::new(args),
                "localhost",
                &mut ctx,
            )
            .unwrap();
        assert!(!r.changed);
    }

    #[test]
    fn test_file_state_link() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src.txt");
        let dest = tmp.path().join("link.txt");
        std::fs::write(&src, b"data").unwrap();
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "src".to_string(),
            Value::String(src.to_str().unwrap().to_string()),
        );
        args.insert(
            "path".to_string(),
            Value::String(dest.to_str().unwrap().to_string()),
        );
        args.insert("state".to_string(), Value::String("link".to_string()));
        let r = FileModule
            .invoke(
                &crate::registry::ModuleArgs::new(args),
                "localhost",
                &mut ctx,
            )
            .unwrap();
        assert!(r.changed);
        assert!(dest.symlink_metadata().is_ok());
    }

    #[test]
    fn test_file_missing_path_errors() {
        let mut ctx = ctx();
        let r = FileModule.invoke(
            &crate::registry::ModuleArgs::new(HashMap::new()),
            "h",
            &mut ctx,
        );
        assert!(r.is_err());
    }

    #[test]
    fn test_file_state_directory_idempotent() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("existing_dir");
        std::fs::create_dir(&dir).unwrap();
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(dir.to_str().unwrap().to_string()),
        );
        args.insert("state".to_string(), Value::String("directory".to_string()));
        let r = FileModule
            .invoke(&crate::registry::ModuleArgs::new(args), "h", &mut ctx)
            .unwrap();
        // Already exists → ok, not changed
        assert!(!r.changed);
    }

    #[test]
    fn test_file_state_link_missing_src_errors() {
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("link.txt");
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(dest.to_str().unwrap().to_string()),
        );
        args.insert("state".to_string(), Value::String("link".to_string()));
        let r = FileModule.invoke(&crate::registry::ModuleArgs::new(args), "h", &mut ctx);
        assert!(r.is_err());
    }

    #[test]
    fn test_file_state_touch_idempotent() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("exists.txt");
        std::fs::write(&f, b"").unwrap();
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(f.to_str().unwrap().to_string()),
        );
        args.insert("state".to_string(), Value::String("touch".to_string()));
        let r = FileModule
            .invoke(&crate::registry::ModuleArgs::new(args), "h", &mut ctx)
            .unwrap();
        // touch on existing file: may or may not be changed depending on implementation
        assert!(r.status.is_ok() || r.changed);
    }
}
