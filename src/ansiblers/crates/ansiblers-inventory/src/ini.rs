//! INI-format inventory parser.
//!
//! Supports the standard Ansible INI inventory format:
//!
//! ```ini
//! # Ungrouped hosts
//! host1 ansible_host=192.168.1.1
//! host2
//!
//! [webservers]
//! web1 ansible_port=2222
//! web2
//!
//! [databases]
//! db1 ansible_host=10.0.0.1
//!
//! [webservers:vars]
//! http_port=80
//!
//! [datacenter:children]
//! webservers
//! databases
//! ```

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use ansiblers_core::{Group, Host, Inventory, Value};

/// Parse an INI-format inventory string into an [`Inventory`].
pub fn parse_ini_inventory(content: &str) -> Result<Inventory> {
    let mut inventory = Inventory::new();
    let mut current_section: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();

        // Skip blank lines and comments.
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        // Section header: [group_name] or [group_name:vars] or [group_name:children]
        if line.starts_with('[') && line.ends_with(']') {
            current_section = Some(line[1..line.len() - 1].to_string());
            // Ensure the group (or base group) exists.
            let base = section_base(current_section.as_deref().unwrap());
            if base != "all" && !inventory.groups.contains_key(base) {
                inventory
                    .groups
                    .insert(base.to_string(), Group::new(base));
            }
            continue;
        }

        match current_section.as_deref() {
            None | Some("all") => {
                // Ungrouped host (implicitly in 'all').
                let (name, vars) = parse_host_line(line)?;
                let mut host = Host::new(&name);
                host.vars = vars;
                apply_ansible_host_vars(&mut host);
                inventory.hosts.insert(name, host);
            }
            Some(section) if section.ends_with(":vars") => {
                // Group variable assignment.
                let group_name = &section[..section.len() - 5];
                let (key, val) = parse_var_assignment(line)?;
                inventory
                    .groups
                    .entry(group_name.to_string())
                    .or_insert_with(|| Group::new(group_name))
                    .vars
                    .insert(key, val);
            }
            Some(section) if section.ends_with(":children") => {
                // Child group declaration.
                let parent_name = &section[..section.len() - 9];
                inventory
                    .groups
                    .entry(parent_name.to_string())
                    .or_insert_with(|| Group::new(parent_name))
                    .children
                    .push(line.to_string());
            }
            Some(group_name) => {
                // Host inside a group.
                let (name, vars) = parse_host_line(line)?;
                let mut host = inventory
                    .hosts
                    .entry(name.clone())
                    .or_insert_with(|| Host::new(&name))
                    .clone();
                host.vars.extend(vars);
                if !host.groups.contains(&group_name.to_string()) {
                    host.groups.push(group_name.to_string());
                }
                apply_ansible_host_vars(&mut host);
                inventory.hosts.insert(name.clone(), host);
                inventory
                    .groups
                    .entry(group_name.to_string())
                    .or_insert_with(|| Group::new(group_name))
                    .hosts
                    .push(name);
            }
        }
    }

    Ok(inventory)
}

/// Extract the base group name (strip `:vars` / `:children` suffix).
fn section_base(section: &str) -> &str {
    if let Some(base) = section.strip_suffix(":vars") {
        base
    } else if let Some(base) = section.strip_suffix(":children") {
        base
    } else {
        section
    }
}

/// Parse a host line: `hostname [key=value ...]`
fn parse_host_line(line: &str) -> Result<(String, HashMap<String, Value>)> {
    let mut parts = line.split_whitespace();
    let name = parts
        .next()
        .ok_or_else(|| anyhow!("empty host line"))?
        .to_string();
    let mut vars = HashMap::new();
    for part in parts {
        let (k, v) = parse_var_assignment(part)?;
        vars.insert(k, v);
    }
    Ok((name, vars))
}

/// Parse `key=value` (value is unquoted; quotes are stripped if present).
fn parse_var_assignment(s: &str) -> Result<(String, Value)> {
    let (k, v) = s
        .split_once('=')
        .ok_or_else(|| anyhow!("expected 'key=value', got '{s}'"))?;
    let key = k.trim().to_string();
    let raw = v.trim().trim_matches('"').trim_matches('\'').to_string();
    // Attempt numeric / boolean coercion; default to string.
    let val = if raw == "true" || raw == "yes" {
        Value::Bool(true)
    } else if raw == "false" || raw == "no" {
        Value::Bool(false)
    } else if let Ok(n) = raw.parse::<i64>() {
        Value::Number(n.into())
    } else if let Ok(f) = raw.parse::<f64>() {
        Value::Number(serde_json::Number::from_f64(f).unwrap_or(0.into()))
    } else {
        Value::String(raw)
    };
    Ok((key, val))
}

/// Move well-known `ansible_*` inventory vars into typed Host fields.
fn apply_ansible_host_vars(host: &mut Host) {
    if let Some(Value::String(h)) = host.vars.remove("ansible_host") {
        host.ansible_host = Some(h);
    }
    if let Some(Value::Number(p)) = host.vars.get("ansible_port") {
        if let Some(port) = p.as_u64() {
            host.ansible_port = Some(port as u16);
            host.vars.remove("ansible_port");
        }
    }
    if let Some(Value::String(c)) = host.vars.remove("ansible_connection") {
        host.ansible_connection = Some(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    const SIMPLE_INI: &str = r#"
host1 ansible_host=192.168.1.1
host2

[webservers]
web1 ansible_port=2222
web2

[databases]
db1

[webservers:vars]
http_port=80
"#;

    #[test]
    fn test_parse_simple_ini() {
        let inv = parse_ini_inventory(SIMPLE_INI).unwrap();
        assert!(inv.hosts.contains_key("host1"));
        assert!(inv.hosts.contains_key("web1"));
        assert!(inv.groups.contains_key("webservers"));
    }

    #[test]
    fn test_ansible_host_var() {
        let inv = parse_ini_inventory(SIMPLE_INI).unwrap();
        let h = inv.get_host("host1").unwrap();
        assert_eq!(h.ansible_host.as_deref(), Some("192.168.1.1"));
    }

    #[test]
    fn test_ansible_port_var() {
        let inv = parse_ini_inventory(SIMPLE_INI).unwrap();
        let h = inv.get_host("web1").unwrap();
        assert_eq!(h.ansible_port, Some(2222));
    }

    #[test]
    fn test_group_vars() {
        let inv = parse_ini_inventory(SIMPLE_INI).unwrap();
        let g = inv.get_group("webservers").unwrap();
        assert_eq!(g.vars.get("http_port"), Some(&Value::Number(80.into())));
    }

    #[test]
    fn test_group_members() {
        let inv = parse_ini_inventory(SIMPLE_INI).unwrap();
        let g = inv.get_group("webservers").unwrap();
        assert!(g.hosts.contains(&"web1".to_string()));
        assert!(g.hosts.contains(&"web2".to_string()));
    }

    #[rstest]
    #[case("true", Value::Bool(true))]
    #[case("false", Value::Bool(false))]
    #[case("42", Value::Number(42.into()))]
    #[case("hello", Value::String("hello".to_string()))]
    fn test_var_coercion(#[case] raw: &str, #[case] expected: Value) {
        let ini = format!("[grp]\nhost1 myvar={raw}\n");
        let inv = parse_ini_inventory(&ini).unwrap();
        let host = inv.get_host("host1").unwrap();
        assert_eq!(host.vars.get("myvar"), Some(&expected));
    }

    #[test]
    fn test_children_groups() {
        let ini = r#"
[webservers]
web1

[databases]
db1

[datacenter:children]
webservers
databases
"#;
        let inv = parse_ini_inventory(ini).unwrap();
        let dc = inv.get_group("datacenter").unwrap();
        assert!(dc.children.contains(&"webservers".to_string()));
        assert!(dc.children.contains(&"databases".to_string()));
        let hosts = inv.matching_hosts("datacenter");
        assert!(hosts.contains(&"web1".to_string()));
        assert!(hosts.contains(&"db1".to_string()));
    }
}
