//! YAML-format inventory parser.
//!
//! Supports the standard Ansible YAML inventory format:
//!
//! ```yaml
//! all:
//!   hosts:
//!     host1:
//!       ansible_host: 192.168.1.1
//!   children:
//!     webservers:
//!       hosts:
//!         web1:
//!           ansible_port: 2222
//!       vars:
//!         http_port: 80
//! ```

use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use ansiblers_core::{Group, Host, Inventory, Value};
use serde_yaml::Value as YamlValue;

/// Parse a YAML-format inventory string into an [`Inventory`].
pub fn parse_yaml_inventory(content: &str) -> Result<Inventory> {
    let root: YamlValue =
        serde_yaml::from_str(content).context("parsing YAML inventory")?;
    let mut inventory = Inventory::new();
    // Top-level must be a mapping (group_name → group_data).
    let top = root
        .as_mapping()
        .ok_or_else(|| anyhow!("YAML inventory root must be a mapping"))?;
    for (k, v) in top.iter() {
        let group_name = k
            .as_str()
            .ok_or_else(|| anyhow!("group name must be a string"))?;
        parse_group(group_name, v, &mut inventory)?;
    }
    Ok(inventory)
}

fn parse_group(name: &str, val: &YamlValue, inventory: &mut Inventory) -> Result<()> {
    let map = match val.as_mapping() {
        Some(m) => m,
        None => {
            // Empty group (null value).
            inventory
                .groups
                .entry(name.to_string())
                .or_insert_with(|| Group::new(name));
            return Ok(());
        }
    };

    let group = inventory
        .groups
        .entry(name.to_string())
        .or_insert_with(|| Group::new(name));

    // Group-level vars.
    if let Some(YamlValue::Mapping(vars_map)) = map.get("vars") {
        for (k, v) in vars_map.iter() {
            if let Some(key) = k.as_str() {
                group
                    .vars
                    .insert(key.to_string(), yaml_to_json(v)?);
            }
        }
    }

    // Hosts in this group.
    if let Some(YamlValue::Mapping(hosts_map)) = map.get("hosts") {
        for (hk, hv) in hosts_map.iter() {
            let hostname = hk
                .as_str()
                .ok_or_else(|| anyhow!("host name must be a string"))?
                .to_string();

            let host = inventory
                .hosts
                .entry(hostname.clone())
                .or_insert_with(|| Host::new(&hostname));

            // Per-host variables.
            if let Some(YamlValue::Mapping(hm)) = Some(hv).filter(|v| v.is_mapping()) {
                for (vk, vv) in hm.iter() {
                    if let Some(key) = vk.as_str() {
                        host.vars.insert(key.to_string(), yaml_to_json(vv)?);
                    }
                }
            }

            // Promote ansible_* vars to typed fields.
            apply_ansible_host_vars(host);

            if !host.groups.contains(&name.to_string()) {
                host.groups.push(name.to_string());
            }

            let group = inventory.groups.get_mut(name).unwrap();
            if !group.hosts.contains(&hostname) {
                group.hosts.push(hostname);
            }
        }
    }

    // Child groups.
    if let Some(YamlValue::Mapping(children_map)) = map.get("children") {
        for (ck, cv) in children_map.iter() {
            let child_name = ck
                .as_str()
                .ok_or_else(|| anyhow!("child group name must be a string"))?;
            parse_group(child_name, cv, inventory)?;
            inventory
                .groups
                .get_mut(name)
                .unwrap()
                .children
                .push(child_name.to_string());
        }
    }

    Ok(())
}

fn apply_ansible_host_vars(host: &mut Host) {
    if let Some(Value::String(h)) = host.vars.remove("ansible_host") {
        host.ansible_host = Some(h);
    }
    if let Some(Value::Number(p)) = host.vars.get("ansible_port").cloned() {
        if let Some(port) = p.as_u64() {
            host.ansible_port = Some(port as u16);
            host.vars.remove("ansible_port");
        }
    }
    if let Some(Value::String(c)) = host.vars.remove("ansible_connection") {
        host.ansible_connection = Some(c);
    }
}

fn yaml_to_json(val: &YamlValue) -> Result<Value> {
    let s = serde_json::to_string(
        &serde_yaml::from_value::<serde_json::Value>(val.clone())
            .context("converting yaml to json")?,
    )
    .context("serialising to json string")?;
    serde_json::from_str(&s).context("parsing json string")
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML_INV: &str = r#"
all:
  hosts:
    host1:
      ansible_host: 192.168.1.1
  children:
    webservers:
      hosts:
        web1:
          ansible_port: 2222
        web2:
      vars:
        http_port: 80
    databases:
      hosts:
        db1:
"#;

    #[test]
    fn test_parse_yaml_inventory() {
        let inv = parse_yaml_inventory(YAML_INV).unwrap();
        assert!(inv.hosts.contains_key("host1"));
        assert!(inv.hosts.contains_key("web1"));
        assert!(inv.groups.contains_key("webservers"));
    }

    #[test]
    fn test_yaml_ansible_host() {
        let inv = parse_yaml_inventory(YAML_INV).unwrap();
        assert_eq!(
            inv.get_host("host1").unwrap().ansible_host.as_deref(),
            Some("192.168.1.1")
        );
    }

    #[test]
    fn test_yaml_group_vars() {
        let inv = parse_yaml_inventory(YAML_INV).unwrap();
        let g = inv.get_group("webservers").unwrap();
        assert_eq!(g.vars.get("http_port"), Some(&Value::Number(80.into())));
    }
}
