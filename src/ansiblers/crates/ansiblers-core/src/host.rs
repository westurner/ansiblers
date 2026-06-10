use std::collections::HashMap;

use crate::Value;

/// Per-host runtime state tracked during playbook execution.
#[derive(Debug, Clone)]
pub struct HostState {
    pub hostname: String,
    pub facts: HashMap<String, Value>,
    /// Host-level variable overrides (e.g. host_vars/).
    pub vars: HashMap<String, Value>,
    pub failed: bool,
    pub unreachable: bool,
    pub skipped: bool,
    /// Count of changed tasks on this host.
    pub changed_count: u32,
    /// Count of ok (no-change) tasks on this host.
    pub ok_count: u32,
}

impl HostState {
    pub fn new(hostname: impl Into<String>) -> Self {
        Self {
            hostname: hostname.into(),
            facts: HashMap::new(),
            vars: HashMap::new(),
            failed: false,
            unreachable: false,
            skipped: false,
            changed_count: 0,
            ok_count: 0,
        }
    }

    pub fn is_active(&self) -> bool {
        !self.failed && !self.unreachable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_host_state_is_active() {
        let hs = HostState::new("web1");
        assert!(hs.is_active());
        assert_eq!(hs.hostname, "web1");
    }

    #[test]
    fn test_failed_host_not_active() {
        let mut hs = HostState::new("web1");
        hs.failed = true;
        assert!(!hs.is_active());
    }
}
