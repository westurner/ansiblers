//! `copy` module — copy files to the managed host.
//!
//! Phase 2 implementation supports local-to-local copy (for localhost execution).
//! Remote copy via SSH transport is planned for Phase 6.

use std::fs;

use ansiblers_core::{ExecutionContext, TaskResult};
use anyhow::Result;

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct CopyModule;

impl ModuleInvoker for CopyModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let dest = args
            .get_str("dest")
            .ok_or_else(|| anyhow::anyhow!("copy module requires 'dest'"))?
            .to_string();

        let dest_path = std::path::Path::new(&dest);

        // Ensure parent directory exists.
        if let Some(parent) = dest_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }

        // `content:` mode — write a string to the file.
        if let Some(content) = args.get_str("content") {
            let existing = fs::read_to_string(&dest).unwrap_or_default();
            if existing == content {
                return Ok(TaskResult::ok(host));
            }
            fs::write(&dest, content)?;
            let mut r = TaskResult::changed(host);
            r.msg = format!("wrote content to '{dest}'");
            return Ok(r);
        }

        // `src:` mode — copy from source path.
        let src = args
            .get_str("src")
            .ok_or_else(|| anyhow::anyhow!("copy module requires 'src' or 'content'"))?;

        let src_path = std::path::Path::new(src);
        if !src_path.exists() {
            return Ok(TaskResult::failed(
                host,
                format!("source '{src}' does not exist"),
            ));
        }

        // If dest is a directory, copy file into it.
        let effective_dest = if dest_path.is_dir() {
            let filename = src_path
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("cannot determine filename from src '{src}'"))?;
            dest_path.join(filename)
        } else {
            dest_path.to_path_buf()
        };

        // Check if already identical (by content).
        let src_content = fs::read(src_path)?;
        let dest_content = fs::read(&effective_dest).unwrap_or_default();
        if src_content == dest_content {
            return Ok(TaskResult::ok(host));
        }

        fs::write(&effective_dest, &src_content)?;

        // Apply mode if specified.
        if let Some(mode_str) = args.get_str("mode") {
            use std::os::unix::fs::PermissionsExt;
            let mode = u32::from_str_radix(mode_str.trim_start_matches("0o"), 8)
                .map_err(|_| anyhow::anyhow!("invalid mode: '{mode_str}'"))?;
            let perms = fs::Permissions::from_mode(mode);
            fs::set_permissions(&effective_dest, perms)?;
        }

        let mut r = TaskResult::changed(host);
        r.msg = format!("copied '{}' to '{}'", src, effective_dest.display());
        Ok(r)
    }
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
    fn test_copy_content() {
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("out.txt");
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "dest".to_string(),
            Value::String(dest.to_str().unwrap().to_string()),
        );
        args.insert("content".to_string(), Value::String("hello\n".to_string()));
        let r = CopyModule
            .invoke(
                &crate::registry::ModuleArgs::new(args),
                "localhost",
                &mut ctx,
            )
            .unwrap();
        assert!(r.changed);
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "hello\n");
    }

    #[test]
    fn test_copy_content_idempotent() {
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("out.txt");
        std::fs::write(&dest, "hello\n").unwrap();
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "dest".to_string(),
            Value::String(dest.to_str().unwrap().to_string()),
        );
        args.insert("content".to_string(), Value::String("hello\n".to_string()));
        let r = CopyModule
            .invoke(
                &crate::registry::ModuleArgs::new(args),
                "localhost",
                &mut ctx,
            )
            .unwrap();
        assert!(!r.changed);
    }

    #[test]
    fn test_copy_src() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src.txt");
        let dest = tmp.path().join("dest.txt");
        std::fs::write(&src, b"data").unwrap();
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "src".to_string(),
            Value::String(src.to_str().unwrap().to_string()),
        );
        args.insert(
            "dest".to_string(),
            Value::String(dest.to_str().unwrap().to_string()),
        );
        let r = CopyModule
            .invoke(
                &crate::registry::ModuleArgs::new(args),
                "localhost",
                &mut ctx,
            )
            .unwrap();
        assert!(r.changed);
        assert_eq!(std::fs::read(&dest).unwrap(), b"data");
    }

    #[test]
    fn test_copy_into_directory() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("file.txt");
        let dest_dir = tmp.path().join("destdir");
        std::fs::create_dir(&dest_dir).unwrap();
        std::fs::write(&src, b"abc").unwrap();
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "src".to_string(),
            Value::String(src.to_str().unwrap().to_string()),
        );
        args.insert(
            "dest".to_string(),
            Value::String(dest_dir.to_str().unwrap().to_string()),
        );
        let r = CopyModule
            .invoke(
                &crate::registry::ModuleArgs::new(args),
                "localhost",
                &mut ctx,
            )
            .unwrap();
        assert!(r.changed);
        assert!(dest_dir.join("file.txt").exists());
    }

    #[test]
    fn test_copy_content_creates_parent_dirs() {
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("sub").join("dir").join("out.txt");
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert("content".to_string(), Value::String("hello".to_string()));
        args.insert(
            "dest".to_string(),
            Value::String(dest.to_str().unwrap().to_string()),
        );
        let r = CopyModule
            .invoke(&crate::registry::ModuleArgs::new(args), "h", &mut ctx)
            .unwrap();
        assert!(r.changed);
        assert!(dest.exists());
    }

    #[test]
    fn test_copy_missing_src_and_content_fails() {
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("out.txt");
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "dest".to_string(),
            Value::String(dest.to_str().unwrap().to_string()),
        );
        let r = CopyModule.invoke(&crate::registry::ModuleArgs::new(args), "h", &mut ctx);
        assert!(r.is_err() || r.unwrap().status.is_failed());
    }

    #[test]
    fn test_copy_missing_dest_fails() {
        let mut ctx = ctx();
        let r = CopyModule.invoke(
            &crate::registry::ModuleArgs::new(HashMap::new()),
            "h",
            &mut ctx,
        );
        assert!(r.is_err());
    }
}
