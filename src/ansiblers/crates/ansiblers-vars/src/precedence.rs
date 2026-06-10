//! Variable precedence scopes, ordered lowest → highest.
//!
//! Based on Ansible's documented variable precedence:
//! <https://docs.ansible.com/ansible/latest/playbook_guide/playbooks_variables.html#understanding-variable-precedence>

/// Represents a named precedence tier.
///
/// Variants are ordered: lower discriminant = lower precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VarScope {
    /// Role defaults (`roles/x/defaults/main.yml`)
    RoleDefaults = 0,
    /// Inventory file variables
    Inventory = 1,
    /// `group_vars/all`
    GroupVarsAll = 2,
    /// `group_vars/<group>`
    GroupVars = 3,
    /// `host_vars/<host>`
    HostVars = 4,
    /// `vars:` in a play
    PlayVars = 5,
    /// Role vars (`roles/x/vars/main.yml`)
    RoleVars = 6,
    /// Task-level `vars:`
    TaskVars = 7,
    /// `set_fact` / `register`
    SetFact = 8,
    /// Extra vars passed with `-e` (highest)
    ExtraVars = 9,
}
