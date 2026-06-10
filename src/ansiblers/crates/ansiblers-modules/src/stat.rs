//! `stat` module — gather file/directory metadata.

use std::os::unix::fs::MetadataExt;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::Result;

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct StatModule;

impl ModuleInvoker for StatModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let path = args
            .get_str("path")
            .or_else(|| args.get_str("dest"))
            .ok_or_else(|| anyhow::anyhow!("stat module requires 'path'"))?
            .to_string();

        let p = std::path::Path::new(&path);

        let mut result = TaskResult::ok(host);

        if !p.exists() && p.symlink_metadata().is_err() {
            // File does not exist — return stat with exists=false.
            let mut stat_map = serde_json::Map::new();
            stat_map.insert("exists".to_string(), Value::Bool(false));
            stat_map.insert("path".to_string(), Value::String(path.clone()));
            result
                .vars
                .insert("stat".to_string(), Value::Object(stat_map));
            return Ok(result);
        }

        let meta = p.symlink_metadata()?;
        let is_file = meta.is_file();
        let is_dir = meta.is_dir();
        let is_link = meta.file_type().is_symlink();
        let size = meta.len();
        let mode = meta.mode();

        let mut stat_map = serde_json::Map::new();
        stat_map.insert("exists".to_string(), Value::Bool(true));
        stat_map.insert("path".to_string(), Value::String(path.clone()));
        stat_map.insert("isreg".to_string(), Value::Bool(is_file));
        stat_map.insert("isdir".to_string(), Value::Bool(is_dir));
        stat_map.insert("islnk".to_string(), Value::Bool(is_link));
        stat_map.insert("size".to_string(), Value::Number(size.into()));
        stat_map.insert(
            "mode".to_string(),
            Value::String(format!("{:04o}", mode & 0o7777)),
        );
        stat_map.insert("uid".to_string(), Value::Number(meta.uid().into()));
        stat_map.insert("gid".to_string(), Value::Number(meta.gid().into()));

        if is_link {
            if let Ok(target) = std::fs::read_link(p) {
                stat_map.insert(
                    "lnk_target".to_string(),
                    Value::String(target.to_string_lossy().into_owned()),
                );
            }
        }

        result
            .vars
            .insert("stat".to_string(), Value::Object(stat_map));
        Ok(result)
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
    fn test_stat_existing_file() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("test.txt");
        std::fs::write(&f, b"hello").unwrap();
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(f.to_str().unwrap().to_string()),
        );
        let r = StatModule
            .invoke(
                &crate::registry::ModuleArgs::new(args),
                "localhost",
                &mut ctx,
            )
            .unwrap();
        let stat = &r.vars["stat"];
        assert_eq!(stat["exists"], Value::Bool(true));
        assert_eq!(stat["isreg"], Value::Bool(true));
        assert_eq!(stat["isdir"], Value::Bool(false));
        assert_eq!(stat["size"], Value::Number(5.into()));
    }

    #[test]
    fn test_stat_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("nope.txt");
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(f.to_str().unwrap().to_string()),
        );
        let r = StatModule
            .invoke(
                &crate::registry::ModuleArgs::new(args),
                "localhost",
                &mut ctx,
            )
            .unwrap();
        assert_eq!(r.vars["stat"]["exists"], Value::Bool(false));
    }

    #[test]
    fn test_stat_directory() {
        let tmp = TempDir::new().unwrap();
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(tmp.path().to_str().unwrap().to_string()),
        );
        let r = StatModule
            .invoke(
                &crate::registry::ModuleArgs::new(args),
                "localhost",
                &mut ctx,
            )
            .unwrap();
        let stat = &r.vars["stat"];
        assert_eq!(stat["exists"], Value::Bool(true));
        assert_eq!(stat["isdir"], Value::Bool(true));
    }

    #[test]
    fn test_stat_file_size() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("sized.txt");
        std::fs::write(&f, b"hello").unwrap();
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(f.to_str().unwrap().to_string()),
        );
        let r = StatModule
            .invoke(&crate::registry::ModuleArgs::new(args), "h", &mut ctx)
            .unwrap();
        assert_eq!(r.vars["stat"]["size"], Value::Number(5.into()));
    }

    #[test]
    fn test_stat_is_file_not_dir() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("f.txt");
        std::fs::write(&f, b"").unwrap();
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(f.to_str().unwrap().to_string()),
        );
        let r = StatModule
            .invoke(&crate::registry::ModuleArgs::new(args), "h", &mut ctx)
            .unwrap();
        assert_eq!(r.vars["stat"]["isreg"], Value::Bool(true));
        assert_eq!(r.vars["stat"]["isdir"], Value::Bool(false));
    }

    #[test]
    fn test_stat_symlink() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("target.txt");
        let link = tmp.path().join("link.txt");
        std::fs::write(&target, b"data").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let mut ctx = ctx();
        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(link.to_str().unwrap().to_string()),
        );
        let r = StatModule
            .invoke(&crate::registry::ModuleArgs::new(args), "h", &mut ctx)
            .unwrap();
        assert_eq!(r.vars["stat"]["islnk"], Value::Bool(true));
        assert!(r.vars["stat"].get("lnk_target").is_some());
    }

    #[test]
    fn test_stat_missing_path_errors() {
        let mut ctx = ctx();
        let r = StatModule.invoke(
            &crate::registry::ModuleArgs::new(HashMap::new()),
            "h",
            &mut ctx,
        );
        assert!(r.is_err());
    }
}
