//! Role dependency resolution — recursive topological ordering.
//!
//! Given a list of directly required roles, [`resolve_dependencies`] traverses
//! their `meta/main.yml` dependencies recursively, detects cycles, and returns
//! an ordered list suitable for execution (dependencies before dependents).

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};

use crate::loader::RoleLoader;
use crate::role::Role;

/// Ordered, deduplicated list of roles with transitive dependencies resolved.
#[derive(Debug, Default)]
pub struct DependencyGraph {
    /// Roles in execution order: dependencies come before dependents.
    pub ordered: Vec<Role>,
    /// Names of roles that have been loaded (for deduplication).
    loaded: HashSet<String>,
}

impl DependencyGraph {
    fn needs_load(&self, name: &str) -> bool {
        !self.loaded.contains(name)
    }

    fn mark_loaded(&mut self, name: &str) {
        self.loaded.insert(name.to_string());
    }
}

/// Resolve all transitive dependencies for `role_names` and return them in
/// topological order (dependencies first).
///
/// Circular dependencies are detected and reported as errors.
///
/// ```rust,no_run
/// use ansiblers_roles::{resolve_dependencies, RoleLoader, RolePath};
/// use std::path::PathBuf;
///
/// let loader = RoleLoader::new(RolePath::new(vec![PathBuf::from("roles")]));
/// let graph = resolve_dependencies(&loader, &["app", "common"]).unwrap();
/// for role in &graph.ordered {
///     println!("{}", role.name);
/// }
/// ```
pub fn resolve_dependencies(loader: &RoleLoader, role_names: &[&str]) -> Result<DependencyGraph> {
    let mut graph = DependencyGraph::default();
    let mut visiting: HashSet<String> = HashSet::new();

    for &name in role_names {
        resolve_one(name, loader, &mut graph, &mut visiting)?;
    }

    Ok(graph)
}

fn resolve_one(
    name: &str,
    loader: &RoleLoader,
    graph: &mut DependencyGraph,
    visiting: &mut HashSet<String>,
) -> Result<()> {
    if !graph.needs_load(name) {
        return Ok(()); // already resolved
    }

    if visiting.contains(name) {
        anyhow::bail!(
            "circular role dependency detected: '{}' (cycle: {:?})",
            name,
            visiting
        );
    }

    visiting.insert(name.to_string());

    let role = loader
        .load(name)
        .with_context(|| format!("loading dependency role '{name}'"))?;

    // Recurse into declared dependencies first.
    let dep_names: Vec<String> = role
        .meta
        .dependencies
        .iter()
        .map(|d| d.role_name().to_string())
        .collect();

    for dep_name in &dep_names {
        resolve_one(dep_name, loader, graph, visiting)?;
    }

    visiting.remove(name);
    graph.mark_loaded(name);
    graph.ordered.push(role);

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::RolePath;
    use std::path::Path;
    use tempfile::TempDir;

    fn write(base: &Path, file: &str, content: &str) {
        let path = base.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn make_loader(tmp: &TempDir) -> RoleLoader {
        RoleLoader::new(RolePath::new(vec![tmp.path().join("roles")]))
    }

    #[test]
    fn test_resolve_simple_dependency_order() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "roles/common/tasks/main.yml",
            "- debug:\n    msg: common\n",
        );
        write(
            tmp.path(),
            "roles/app/tasks/main.yml",
            "- debug:\n    msg: app\n",
        );
        write(
            tmp.path(),
            "roles/app/meta/main.yml",
            "dependencies:\n  - role: common\n",
        );

        let loader = make_loader(&tmp);
        let graph = resolve_dependencies(&loader, &["app"]).unwrap();
        assert_eq!(graph.ordered.len(), 2);
        assert_eq!(graph.ordered[0].name, "common"); // dep first
        assert_eq!(graph.ordered[1].name, "app");
    }

    #[test]
    fn test_deduplication() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "roles/base/tasks/main.yml",
            "- debug:\n    msg: base\n",
        );
        write(
            tmp.path(),
            "roles/a/tasks/main.yml",
            "- debug:\n    msg: a\n",
        );
        write(
            tmp.path(),
            "roles/a/meta/main.yml",
            "dependencies:\n  - role: base\n",
        );
        write(
            tmp.path(),
            "roles/b/tasks/main.yml",
            "- debug:\n    msg: b\n",
        );
        write(
            tmp.path(),
            "roles/b/meta/main.yml",
            "dependencies:\n  - role: base\n",
        );

        let loader = make_loader(&tmp);
        let graph = resolve_dependencies(&loader, &["a", "b"]).unwrap();
        // base should appear only once despite both a and b depending on it
        let names: Vec<&str> = graph.ordered.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names.iter().filter(|&&n| n == "base").count(), 1);
    }

    #[test]
    fn test_cycle_detection() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "roles/a/tasks/main.yml",
            "- debug:\n    msg: a\n",
        );
        write(
            tmp.path(),
            "roles/a/meta/main.yml",
            "dependencies:\n  - role: b\n",
        );
        write(
            tmp.path(),
            "roles/b/tasks/main.yml",
            "- debug:\n    msg: b\n",
        );
        write(
            tmp.path(),
            "roles/b/meta/main.yml",
            "dependencies:\n  - role: a\n",
        );

        let loader = make_loader(&tmp);
        let result = resolve_dependencies(&loader, &["a"]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("circular"));
    }
}
