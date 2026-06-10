//! Integration tests: inventory loading (INI and YAML).

use ansiblers_it::fixture_path;
use ansiblers_inventory::{load_inventory, InlineFormat, InlineInventoryLoader, InventoryLoader};
use rstest::rstest;

#[rstest]
#[case("simple.ini")]
#[case("with_groups.ini")]
#[case("simple.yml")]
fn test_load_fixture_inventory(#[case] filename: &str) {
    let path = fixture_path(&format!("inventories/{filename}"));
    let inv = load_inventory(&path)
        .unwrap_or_else(|e| panic!("failed to load {filename}: {e}"));
    assert!(!inv.hosts.is_empty(), "{filename} should have hosts");
}

#[test]
fn test_ini_groups() {
    let path = fixture_path("inventories/with_groups.ini");
    let inv = load_inventory(&path).unwrap();
    assert!(inv.groups.contains_key("webservers"));
    assert!(inv.groups.contains_key("databases"));
    let hosts = inv.matching_hosts("webservers");
    assert!(hosts.contains(&"web1".to_string()));
}

#[test]
fn test_yaml_group_vars() {
    let path = fixture_path("inventories/simple.yml");
    let inv = load_inventory(&path).unwrap();
    let g = inv.get_group("webservers").unwrap();
    assert_eq!(
        g.vars.get("http_port"),
        Some(&ansiblers_core::Value::Number(80.into()))
    );
}

#[test]
fn test_inventory_matching_hosts_all() {
    let path = fixture_path("inventories/simple.ini");
    let inv = load_inventory(&path).unwrap();
    let hosts = inv.matching_hosts("all");
    assert!(!hosts.is_empty());
}

#[rstest]
#[case("web1", "webservers", true)]
#[case("db1", "webservers", false)]
fn test_host_group_membership(
    #[case] host: &str,
    #[case] group: &str,
    #[case] expected: bool,
) {
    let path = fixture_path("inventories/with_groups.ini");
    let inv = load_inventory(&path).unwrap();
    let h = inv.get_host(host).unwrap();
    assert_eq!(h.in_group(group), expected);
}

#[test]
fn test_inline_ini_loader() {
    let ini = "[webservers]\nweb1\nweb2\n";
    let loader = InlineInventoryLoader { format: InlineFormat::Ini };
    let inv = loader.load(ini).unwrap();
    assert!(inv.hosts.contains_key("web1"));
}
