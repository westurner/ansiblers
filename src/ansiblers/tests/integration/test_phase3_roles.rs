//! Integration tests: Phase 3 — group_vars/host_vars, dynamic inventory, roles.

use std::path::Path;

use ansiblers_inventory::{
    dynamic::parse_list_output, group_host_vars::merge_group_and_host_vars, load_inventory,
};
use ansiblers_it::fixture_path;
use ansiblers_roles::{
    dependency::resolve_dependencies, galaxy::GalaxyRequirements, RoleLoader, RolePath,
};
use rstest::rstest;

// ---------------------------------------------------------------------------
// group_vars / host_vars
// ---------------------------------------------------------------------------

#[test]
fn test_group_vars_loaded_from_fixture_dir() {
    let inv_path = fixture_path("inventories/with_groups.ini");
    let mut inv = load_inventory(&inv_path).unwrap();
    let inv_dir = Path::new(&inv_path).parent().unwrap();
    merge_group_and_host_vars(&mut inv, &[inv_dir]).unwrap();

    // group_vars/all.yml should be merged into the "all" group.
    let all = inv.groups.get("all").unwrap();
    assert_eq!(
        all.vars.get("env"),
        Some(&ansiblers_core::Value::String("staging".into()))
    );

    // group_vars/webservers.yml
    let ws = inv.groups.get("webservers").unwrap();
    assert_eq!(
        ws.vars.get("http_port"),
        Some(&ansiblers_core::Value::Number(80.into()))
    );

    // host_vars/web1.example.com.yml
    let web1 = inv.hosts.get("web1.example.com");
    if let Some(h) = web1 {
        assert_eq!(
            h.vars.get("ansible_user"),
            Some(&ansiblers_core::Value::String("deploy".into()))
        );
    }
}

#[test]
fn test_host_vars_override_group_vars() {
    use ansiblers_core::{Group, Host, Inventory, Value};

    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("group_vars")).unwrap();
    std::fs::create_dir_all(tmp.path().join("host_vars")).unwrap();
    std::fs::write(
        tmp.path().join("group_vars/all.yml"),
        "server_role: generic\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("host_vars/web1.yml"),
        "server_role: web_special\n",
    )
    .unwrap();

    let mut inv = Inventory::new();
    let mut web1 = Host::new("web1");
    web1.groups = vec!["all".into()];
    inv.hosts.insert("web1".into(), web1);
    inv.groups.insert("all".into(), Group::new("all"));

    merge_group_and_host_vars(&mut inv, &[tmp.path()]).unwrap();

    // host_vars take precedence over group_vars
    let host = inv.get_host("web1").unwrap();
    assert_eq!(
        host.vars.get("server_role"),
        Some(&Value::String("web_special".into()))
    );
}

// ---------------------------------------------------------------------------
// Dynamic inventory
// ---------------------------------------------------------------------------

#[test]
fn test_parse_dynamic_inventory_json() {
    let json = r#"{
        "_meta": { "hostvars": { "web1": { "ansible_host": "10.0.0.1" } } },
        "webservers": { "hosts": ["web1", "web2"], "vars": { "http_port": 80 } },
        "databases": ["db1"],
        "all": { "children": ["webservers", "databases"] }
    }"#;
    let inv = parse_list_output(json).unwrap();
    assert!(inv.hosts.contains_key("web1"));
    assert!(inv.groups.contains_key("webservers"));
    let ws = inv.get_group("webservers").unwrap();
    assert_eq!(
        ws.vars.get("http_port"),
        Some(&ansiblers_core::Value::Number(80.into()))
    );
    assert!(inv
        .get_group("all")
        .unwrap()
        .children
        .contains(&"webservers".to_string()));
}

#[test]
fn test_dynamic_inventory_hostvars_from_meta() {
    let json = r#"{
        "_meta": { "hostvars": { "db1": { "db_port": 5432 } } },
        "databases": ["db1"]
    }"#;
    let inv = parse_list_output(json).unwrap();
    assert_eq!(
        inv.get_host("db1").unwrap().vars.get("db_port"),
        Some(&ansiblers_core::Value::Number(5432.into()))
    );
}

// ---------------------------------------------------------------------------
// Role loading
// ---------------------------------------------------------------------------

#[test]
fn test_load_common_role_from_fixture() {
    let roles_dir = fixture_path("roles");
    let loader = RoleLoader::new(RolePath::new(vec![std::path::PathBuf::from(&roles_dir)]));
    let role = loader.load("common").unwrap();
    assert!(!role.tasks.is_empty(), "common role should have tasks");
    assert_eq!(
        role.defaults.get("common_user"),
        Some(&ansiblers_core::Value::String("nobody".into()))
    );
    assert_eq!(
        role.vars.get("common_config_dir"),
        Some(&ansiblers_core::Value::String("/etc/common".into()))
    );
}

#[test]
fn test_load_webserver_role_with_meta() {
    let roles_dir = fixture_path("roles");
    let loader = RoleLoader::new(RolePath::new(vec![std::path::PathBuf::from(&roles_dir)]));
    let role = loader.load("webserver").unwrap();
    assert!(role.has_tasks());
    assert!(!role.handlers.is_empty());
    assert_eq!(role.meta.dependencies.len(), 1);
    assert_eq!(role.meta.dependencies[0].role_name(), "common");
}

// ---------------------------------------------------------------------------
// Dependency resolution
// ---------------------------------------------------------------------------

#[test]
fn test_dependency_resolution_webserver() {
    let roles_dir = fixture_path("roles");
    let loader = RoleLoader::new(RolePath::new(vec![std::path::PathBuf::from(&roles_dir)]));
    let graph = resolve_dependencies(&loader, &["webserver"]).unwrap();
    let names: Vec<&str> = graph.ordered.iter().map(|r| r.name.as_str()).collect();
    // common (dependency) must come before webserver
    let common_idx = names.iter().position(|&n| n == "common").unwrap();
    let web_idx = names.iter().position(|&n| n == "webserver").unwrap();
    assert!(common_idx < web_idx, "common must precede webserver");
}

// ---------------------------------------------------------------------------
// Galaxy requirements.yml
// ---------------------------------------------------------------------------

#[test]
fn test_parse_requirements_fixture() {
    let path = fixture_path("requirements.yml");
    let req = GalaxyRequirements::from_file(&path).unwrap();
    assert_eq!(req.roles.len(), 3);
    assert_eq!(req.collections.len(), 1);
    assert_eq!(req.roles[0].name(), "geerlingguy.nginx");
    assert_eq!(req.roles[0].version(), Some("6.0.0"));
    assert_eq!(req.roles[2].name(), "common");
}

#[rstest]
#[case("geerlingguy.nginx", Some("6.0.0"))]
#[case("testrole", Some("main"))]
#[case("common", None)]
fn test_requirements_role_versions(#[case] name: &str, #[case] version: Option<&str>) {
    let path = fixture_path("requirements.yml");
    let req = GalaxyRequirements::from_file(&path).unwrap();
    let found = req.roles.iter().find(|r| r.name() == name).unwrap();
    assert_eq!(found.version(), version);
}
