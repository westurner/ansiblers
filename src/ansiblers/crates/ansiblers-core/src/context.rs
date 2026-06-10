use std::collections::HashMap;
use std::sync::Arc;

use crate::inventory::Inventory;
use crate::Value;

/// Global state threaded through playbook execution.
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub inventory: Arc<Inventory>,
    /// Variables set at the play level (`vars:` key).
    pub playbook_vars: HashMap<String, Value>,
    /// Variables captured via `register:`.
    pub registered_vars: HashMap<String, Value>,
    /// facts[hostname][fact_name] = value
    pub facts: HashMap<String, HashMap<String, Value>>,
    /// Verbosity level (0 = normal, 1 = -v, 2 = -vv, …).
    pub verbosity: u8,
    /// Current host being executed (set by executor for each host loop).
    pub current_host: Option<String>,
}

impl ExecutionContext {
    pub fn new(inventory: Arc<Inventory>, playbook_vars: HashMap<String, Value>) -> Self {
        Self {
            inventory,
            playbook_vars,
            registered_vars: HashMap::new(),
            facts: HashMap::new(),
            verbosity: 0,
            current_host: None,
        }
    }

    /// Look up a variable by name, respecting simple precedence:
    /// registered_vars > playbook_vars.
    pub fn get_var(&self, name: &str) -> Option<&Value> {
        self.registered_vars
            .get(name)
            .or_else(|| self.playbook_vars.get(name))
    }

    /// Store a registered variable (from `register:` directive).
    pub fn register_var(&mut self, name: String, value: Value) {
        self.registered_vars.insert(name, value);
    }

    /// Store a per-host fact (from `set_fact:` or `setup` module).
    pub fn set_fact(&mut self, host: &str, name: String, value: Value) {
        self.facts
            .entry(host.to_string())
            .or_default()
            .insert(name, value);
    }

    /// Retrieve a per-host fact.
    pub fn get_fact(&self, host: &str, name: &str) -> Option<&Value> {
        self.facts.get(host).and_then(|f| f.get(name))
    }

    /// Build a flat variable map for template rendering (facts + registered + play vars).
    /// Precedence (highest last, overwrites): play_vars < registered < facts.
    pub fn render_vars(&self, host: &str) -> HashMap<String, Value> {
        let mut vars = self.playbook_vars.clone();
        // Merge registered vars
        for (k, v) in &self.registered_vars {
            vars.insert(k.clone(), v.clone());
        }
        // Merge host facts (highest precedence for rendering context)
        if let Some(host_facts) = self.facts.get(host) {
            for (k, v) in host_facts {
                vars.insert(k.clone(), v.clone());
            }
        }
        // Inject Ansible magic variables
        vars.insert(
            "inventory_hostname".to_string(),
            Value::String(host.to_string()),
        );
        vars
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn empty_ctx() -> ExecutionContext {
        ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new())
    }

    #[test]
    fn test_register_and_get_var() {
        let mut ctx = empty_ctx();
        ctx.register_var("result".to_string(), Value::String("hello".to_string()));
        assert_eq!(
            ctx.get_var("result"),
            Some(&Value::String("hello".to_string()))
        );
    }

    #[rstest]
    #[case("host1", "ansible_os", "linux")]
    #[case("host2", "pkg_version", "1.2.3")]
    fn test_set_and_get_fact(#[case] host: &str, #[case] name: &str, #[case] val: &str) {
        let mut ctx = empty_ctx();
        ctx.set_fact(host, name.to_string(), Value::String(val.to_string()));
        assert_eq!(
            ctx.get_fact(host, name),
            Some(&Value::String(val.to_string()))
        );
    }

    #[test]
    fn test_render_vars_includes_magic() {
        let ctx = empty_ctx();
        let vars = ctx.render_vars("myhost");
        assert_eq!(
            vars.get("inventory_hostname"),
            Some(&Value::String("myhost".to_string()))
        );
    }
}
