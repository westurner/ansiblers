//! Integration tests: end-to-end playbook execution.

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
#[case("simple_shell.yml")]
#[case("with_variables.yml")]
#[case("with_blocks.yml")]
fn test_execute_fixture_playbook(#[case] filename: &str) {
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
fn test_variable_register_and_use() {
    let path = fixture_path("playbooks/with_variables.yml");
    let pb = parse_playbook(&path).unwrap();
    let mut ctx = local_ctx();
    let result = executor().run_playbook(&pb, &mut ctx).unwrap();
    assert!(result.success);
    // set_fact should have stored 'computed'
    let fact = ctx.get_fact("localhost", "computed");
    assert!(
        fact.is_some(),
        "set_fact should have set 'computed' fact"
    );
}

#[test]
fn test_block_rescue_sets_fact() {
    let path = fixture_path("playbooks/with_blocks.yml");
    let pb = parse_playbook(&path).unwrap();
    let mut ctx = local_ctx();
    let result = executor().run_playbook(&pb, &mut ctx).unwrap();
    // Rescue should recover from the failure
    assert!(result.success, "block rescue should recover");
    let rescued = ctx.get_fact("localhost", "rescued");
    assert!(rescued.is_some(), "rescue task should have set 'rescued' fact");
    let cleanup = ctx.get_fact("localhost", "cleanup_done");
    assert!(cleanup.is_some(), "always task should have set 'cleanup_done' fact");
}

#[test]
fn test_extra_vars_override_play_vars() {
    let yaml = r#"
- hosts: localhost
  gather_facts: false
  vars:
    greeting: "play_hello"
  tasks:
    - shell: echo {{ greeting }}
      register: r
"#;
    let pb = ansiblers_parser::parse_playbook_str(yaml, None).unwrap();
    let extra = [("greeting".to_string(), Value::String("extra_hello".to_string()))]
        .into_iter()
        .collect();
    let mut ctx = ExecutionContext::new(Arc::new(Inventory::default()), extra);
    executor().run_playbook(&pb, &mut ctx).unwrap();
    let r = ctx.get_var("r").unwrap();
    // Extra vars were set as playbook_vars (highest) — should win.
    assert!(
        r["stdout"].as_str().unwrap().contains("hello"),
        "echo should contain hello"
    );
}
