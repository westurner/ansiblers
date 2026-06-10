//! Variable resolver — builds a flat variable map for template rendering,
//! respecting Ansible's variable precedence order.

use std::collections::HashMap;

use ansiblers_core::{ExecutionContext, Value};

use crate::precedence::VarScope;

/// Builds and queries merged variable maps for a given host context.
pub struct VariableResolver<'ctx> {
    ctx: &'ctx ExecutionContext,
    /// Extra vars (-e flag), highest precedence.
    extra_vars: HashMap<String, Value>,
    /// Task-level vars (from `vars:` on a task).
    task_vars: HashMap<String, Value>,
}

impl<'ctx> VariableResolver<'ctx> {
    pub fn new(ctx: &'ctx ExecutionContext) -> Self {
        Self {
            ctx,
            extra_vars: HashMap::new(),
            task_vars: HashMap::new(),
        }
    }

    pub fn with_extra_vars(mut self, extra: HashMap<String, Value>) -> Self {
        self.extra_vars = extra;
        self
    }

    pub fn with_task_vars(mut self, vars: HashMap<String, Value>) -> Self {
        self.task_vars = vars;
        self
    }

    /// Return a flat variable map merged in precedence order for `host`.
    ///
    /// Merge order (later entries win):
    /// inventory vars < group_vars/all < group_vars < host_vars
    ///   < play_vars < registered < facts < task_vars < extra_vars
    pub fn merged(&self, host: &str) -> HashMap<String, Value> {
        let mut vars: HashMap<String, Value> = HashMap::new();

        // 1. Inventory host vars (group vars already merged by Inventory::host_vars).
        if let Some(inv_host) = self.ctx.inventory.get_host(host) {
            let inv_vars = self.ctx.inventory.host_vars(&inv_host.name);
            vars.extend(inv_vars);
        }

        // 2. Play-level vars.
        vars.extend(self.ctx.playbook_vars.clone());

        // 3. Registered vars.
        vars.extend(self.ctx.registered_vars.clone());

        // 4. Facts for this host.
        if let Some(facts) = self.ctx.facts.get(host) {
            vars.extend(facts.clone());
        }

        // 5. Task vars.
        vars.extend(self.task_vars.clone());

        // 6. Extra vars (highest precedence).
        vars.extend(self.extra_vars.clone());

        // Magic variables.
        vars.insert(
            "inventory_hostname".to_string(),
            Value::String(host.to_string()),
        );
        vars.insert(
            "inventory_hostname_short".to_string(),
            Value::String(host.split('.').next().unwrap_or(host).to_string()),
        );

        vars
    }

    /// Look up a single variable by name for `host`.
    pub fn get(&self, host: &str, name: &str) -> Option<Value> {
        self.merged(host).remove(name)
    }
}

/// Variable resolver that operates without a full ExecutionContext — useful
/// for unit-testing the resolver in isolation.
pub struct StandaloneResolver {
    layers: Vec<(VarScope, HashMap<String, Value>)>,
}

impl StandaloneResolver {
    pub fn new() -> Self {
        Self { layers: Vec::new() }
    }

    pub fn add(&mut self, scope: VarScope, vars: HashMap<String, Value>) {
        self.layers.push((scope, vars));
    }

    /// Set a single key at the given scope.
    pub fn set(&mut self, scope: VarScope, key: impl Into<String>, val: Value) {
        let key = key.into();
        for (s, m) in &mut self.layers {
            if *s == scope {
                m.insert(key, val);
                return;
            }
        }
        let mut m = HashMap::new();
        m.insert(key, val);
        self.layers.push((scope, m));
    }

    /// Merge all layers respecting precedence order.
    pub fn merged(&self) -> HashMap<String, Value> {
        let mut sorted = self.layers.clone();
        sorted.sort_by_key(|(s, _)| *s);
        let mut vars = HashMap::new();
        for (_, layer) in sorted {
            vars.extend(layer);
        }
        vars
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        self.merged().remove(key)
    }
}

impl Default for StandaloneResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(VarScope::PlayVars, VarScope::SetFact, "setfact_wins")]
    #[case(VarScope::Inventory, VarScope::PlayVars, "playvars_wins")]
    fn test_precedence_ordering(
        #[case] low: VarScope,
        #[case] high: VarScope,
        #[case] winner: &str,
    ) {
        let mut r = StandaloneResolver::new();
        r.set(low, "myvar", Value::String("low_wins".to_string()));
        r.set(high, "myvar", Value::String(winner.to_string()));
        assert_eq!(r.get("myvar"), Some(Value::String(winner.to_string())));
    }

    #[test]
    fn test_extra_vars_highest() {
        let mut r = StandaloneResolver::new();
        r.set(VarScope::PlayVars, "x", Value::String("play".to_string()));
        r.set(VarScope::ExtraVars, "x", Value::String("extra".to_string()));
        r.set(VarScope::SetFact, "x", Value::String("fact".to_string()));
        // ExtraVars > SetFact > PlayVars
        assert_eq!(r.get("x"), Some(Value::String("extra".to_string())));
    }

    #[test]
    fn test_missing_var_returns_none() {
        let r = StandaloneResolver::new();
        assert_eq!(r.get("nonexistent"), None);
    }
}
