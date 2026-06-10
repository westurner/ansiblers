//! Integration tests: Phase 2 module execution (file, copy, stat).

use std::sync::Arc;

use ansiblers_core::{ExecutionContext, Inventory, Value};
use ansiblers_executor::PlayExecutor;
use ansiblers_it::fixture_path;
use ansiblers_modules::ModuleRegistry;
use ansiblers_parser::parse_playbook;
use rstest::rstest;

fn executor() -> PlayExecutor {
    PlayExecutor::new(ModuleRegistry::with_defaults())
}

fn local_ctx() -> ExecutionContext {
    ExecutionContext::new(Arc::new(Inventory::default()), std::collections::HashMap::new())
}

#[rstest]
#[case("file_operations.yml")]
#[case("copy_operations.yml")]
fn test_phase2_playbooks_succeed(#[case] filename: &str) {
    let path = fixture_path(&format!("playbooks/{filename}"));
    let pb = parse_playbook(&path)
        .unwrap_or_else(|e| panic!("parse {filename}: {e}"));
    let mut ctx = local_ctx();
    let result = executor()
        .run_playbook(&pb, &mut ctx)
        .unwrap_or_else(|e| panic!("execute {filename}: {e}"));
    assert!(result.success, "{filename} should succeed");
}

#[test]
fn test_file_module_directory_create_remove() {
    use ansiblers_modules::{ModuleArgs, ModuleInvoker};
    use ansiblers_modules::file::FileModule;
    use std::collections::HashMap;

    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path().join("test_dir");
    let mut ctx = local_ctx();

    let mut args = HashMap::new();
    args.insert("path".to_string(), Value::String(dir.to_str().unwrap().to_string()));
    args.insert("state".to_string(), Value::String("directory".to_string()));
    let result = FileModule.invoke(&ModuleArgs::new(args), "localhost", &mut ctx).unwrap();
    assert!(result.changed);
    assert!(dir.is_dir());

    // Idempotent second call.
    let mut args2 = HashMap::new();
    args2.insert("path".to_string(), Value::String(dir.to_str().unwrap().to_string()));
    args2.insert("state".to_string(), Value::String("directory".to_string()));
    let result2 = FileModule.invoke(&ModuleArgs::new(args2), "localhost", &mut ctx).unwrap();
    assert!(!result2.changed, "second directory create should be idempotent");
}

#[test]
fn test_copy_module_content_write() {
    use ansiblers_modules::{ModuleArgs, ModuleInvoker};
    use ansiblers_modules::copy::CopyModule;
    use std::collections::HashMap;

    let tmp = tempfile::TempDir::new().unwrap();
    let dest = tmp.path().join("output.txt");
    let mut ctx = local_ctx();

    let mut args = HashMap::new();
    args.insert("dest".to_string(), Value::String(dest.to_str().unwrap().to_string()));
    args.insert("content".to_string(), Value::String("test content\n".to_string()));
    let result = CopyModule.invoke(&ModuleArgs::new(args), "localhost", &mut ctx).unwrap();
    assert!(result.changed);
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "test content\n");
}

#[test]
fn test_stat_module_existing_file() {
    use ansiblers_modules::{ModuleArgs, ModuleInvoker};
    use ansiblers_modules::stat::StatModule;
    use std::collections::HashMap;

    let tmp = tempfile::TempDir::new().unwrap();
    let f = tmp.path().join("check.txt");
    std::fs::write(&f, b"data").unwrap();
    let mut ctx = local_ctx();

    let mut args = HashMap::new();
    args.insert("path".to_string(), Value::String(f.to_str().unwrap().to_string()));
    let result = StatModule.invoke(&ModuleArgs::new(args), "localhost", &mut ctx).unwrap();
    assert_eq!(result.vars["stat"]["exists"], Value::Bool(true));
    assert_eq!(result.vars["stat"]["isreg"], Value::Bool(true));
}

#[test]
fn test_multi_host_execution() {
    use ansiblers_core::{Group, Host, Inventory};

    // Build a small in-memory inventory with two hosts.
    let mut inv = Inventory::new();
    let mut web1 = Host::new("web1");
    web1.ansible_connection = Some("local".to_string());
    web1.groups = vec!["all".to_string()];
    let mut web2 = Host::new("web2");
    web2.ansible_connection = Some("local".to_string());
    web2.groups = vec!["all".to_string()];
    inv.hosts.insert("web1".to_string(), web1);
    inv.hosts.insert("web2".to_string(), web2);
    let mut all_group = Group::new("all");
    all_group.hosts = vec!["web1".to_string(), "web2".to_string()];
    inv.groups.insert("all".to_string(), all_group);

    let pb = ansiblers_parser::parse_playbook_str(
        "- hosts: all\n  gather_facts: false\n  tasks:\n    - shell: echo {{ inventory_hostname }}\n      register: r\n",
        None,
    )
    .unwrap();

    let mut ctx = ExecutionContext::new(Arc::new(inv), std::collections::HashMap::new());
    let result = executor().run_playbook(&pb, &mut ctx).unwrap();
    assert!(result.success);

    // Both hosts should have task results.
    let play_result = &result.play_results[0];
    assert!(play_result.host_results.contains_key("web1"));
    assert!(play_result.host_results.contains_key("web2"));
}
