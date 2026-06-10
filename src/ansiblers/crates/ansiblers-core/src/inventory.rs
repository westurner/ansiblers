use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::Value;

/// The inventory: hosts and groups loaded from inventory files.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Inventory {
    pub hosts: HashMap<String, Host>,
    pub groups: HashMap<String, Group>,
}

impl Inventory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_host(&self, name: &str) -> Option<&Host> {
        self.hosts.get(name)
    }

    pub fn get_group(&self, name: &str) -> Option<&Group> {
        self.groups.get(name)
    }

    /// Return hostnames matching a simple pattern (`all`, `*`, a group name, or
    /// an exact hostname).  More complex patterns (globs, ranges) are left for
    /// a later phase.
    pub fn matching_hosts(&self, pattern: &str) -> Vec<String> {
        match pattern {
            "all" | "*" => {
                let mut hosts: Vec<String> = self.hosts.keys().cloned().collect();
                // Ensure localhost is always reachable even if not declared.
                if hosts.is_empty() {
                    hosts.push("localhost".to_string());
                }
                hosts
            }
            "localhost" => vec!["localhost".to_string()],
            _ => {
                // Group name?
                if let Some(group) = self.groups.get(pattern) {
                    // Expand group children recursively.
                    self.expand_group(group)
                } else if self.hosts.contains_key(pattern) {
                    vec![pattern.to_string()]
                } else {
                    vec![]
                }
            }
        }
    }

    fn expand_group(&self, group: &Group) -> Vec<String> {
        let mut hosts: Vec<String> = group.hosts.clone();
        for child in &group.children {
            if let Some(child_group) = self.groups.get(child) {
                hosts.extend(self.expand_group(child_group));
            }
        }
        hosts.sort();
        hosts.dedup();
        hosts
    }

    /// Return all variables visible to a host (host vars > group vars > all group vars).
    pub fn host_vars(&self, hostname: &str) -> HashMap<String, Value> {
        let mut vars: HashMap<String, Value> = HashMap::new();
        // all-group vars first (lowest precedence)
        if let Some(all) = self.groups.get("all") {
            vars.extend(all.vars.clone());
        }
        // Group vars
        if let Some(host) = self.hosts.get(hostname) {
            for group_name in &host.groups {
                if let Some(group) = self.groups.get(group_name) {
                    vars.extend(group.vars.clone());
                }
            }
            // Host vars (highest precedence)
            vars.extend(host.vars.clone());
        }
        vars
    }
}

/// A single managed host.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Host {
    pub name: String,
    pub vars: HashMap<String, Value>,
    pub groups: Vec<String>,
    pub ansible_connection: Option<String>,
    pub ansible_host: Option<String>,
    pub ansible_port: Option<u16>,
}

impl Host {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    pub fn in_group(&self, group: &str) -> bool {
        self.groups.contains(&group.to_string())
    }

    /// Returns the address to connect to (ansible_host if set, else hostname).
    pub fn connection_host(&self) -> &str {
        self.ansible_host.as_deref().unwrap_or(&self.name)
    }
}

/// A group of hosts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    pub hosts: Vec<String>,
    pub vars: HashMap<String, Value>,
    pub children: Vec<String>,
}

impl Group {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn sample_inventory() -> Inventory {
        let mut inv = Inventory::new();
        let mut web1 = Host::new("web1");
        web1.groups = vec!["webservers".to_string()];
        let mut db1 = Host::new("db1");
        db1.groups = vec!["databases".to_string()];
        inv.hosts.insert("web1".to_string(), web1);
        inv.hosts.insert("db1".to_string(), db1);
        let mut webservers = Group::new("webservers");
        webservers.hosts = vec!["web1".to_string()];
        let mut databases = Group::new("databases");
        databases.hosts = vec!["db1".to_string()];
        inv.groups.insert("webservers".to_string(), webservers);
        inv.groups.insert("databases".to_string(), databases);
        inv
    }

    #[test]
    fn test_matching_hosts_all() {
        let inv = sample_inventory();
        let mut hosts = inv.matching_hosts("all");
        hosts.sort();
        assert_eq!(hosts, vec!["db1", "web1"]);
    }

    #[rstest]
    #[case("webservers", vec!["web1"])]
    #[case("databases", vec!["db1"])]
    fn test_matching_hosts_group(#[case] group: &str, #[case] expected: Vec<&str>) {
        let inv = sample_inventory();
        let mut hosts = inv.matching_hosts(group);
        hosts.sort();
        let mut exp: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
        exp.sort();
        assert_eq!(hosts, exp);
    }

    #[test]
    fn test_host_in_group() {
        let inv = sample_inventory();
        assert!(inv.get_host("web1").unwrap().in_group("webservers"));
        assert!(!inv.get_host("web1").unwrap().in_group("databases"));
    }
}
