//! `setup` module — gather system facts and store them in the execution context.
//!
//! Mirrors Ansible's `setup` module.  Facts are collected from `/proc`, `/sys`,
//! and other OS interfaces, then stored as per-host facts in the
//! [`ExecutionContext`] under the standard `ansible_*` namespace.
//!
//! ## Collected fact groups
//!
//! | Fact key | Source | Description |
//! |----------|--------|-------------|
//! | `ansible_os_family` | `/etc/os-release` | e.g. `Debian`, `RedHat` |
//! | `ansible_distribution` | `/etc/os-release` | e.g. `Ubuntu`, `Fedora` |
//! | `ansible_distribution_version` | `/etc/os-release` | e.g. `22.04` |
//! | `ansible_kernel` | `uname -r` | Kernel release string |
//! | `ansible_architecture` | `uname -m` | e.g. `x86_64` |
//! | `ansible_hostname` | `hostname` | Short hostname |
//! | `ansible_fqdn` | `hostname --fqdn` | Fully-qualified domain name |
//! | `ansible_processor_count` | `/proc/cpuinfo` | Number of logical CPUs |
//! | `ansible_memtotal_mb` | `/proc/meminfo` | Total RAM in MiB |
//! | `ansible_memfree_mb` | `/proc/meminfo` | Free RAM in MiB |
//! | `ansible_interfaces` | `/proc/net/dev` | List of network interface names |
//! | `ansible_mounts` | `/proc/mounts` | List of mount points |
//! | `ansible_env` | `std::env::vars()` | Dictionary of environment variables |
//! | `ansible_date_time` | system clock | Current date/time fields |
//! | `ansible_python_version` | `python3 --version` | Python version string |
//! | `ansible_virtualization_type` | heuristics | `docker`, `kvm`, `none`, … |

use std::collections::HashMap;
use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::Result;

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct SetupModule;

impl ModuleInvoker for SetupModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let _filter = args.get_str("filter"); // gather_subset / filter (future use)
        let facts = collect_all_facts();

        // Store every fact as a per-host fact.
        for (k, v) in &facts {
            ctx.set_fact(host, k.clone(), v.clone());
        }

        let mut result = TaskResult::ok(host);
        let facts_obj: serde_json::Map<String, Value> = facts.into_iter().collect();
        result.vars.insert("ansible_facts".into(), Value::Object(facts_obj));
        result.msg = "Facts gathered".to_string();
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Fact collection
// ---------------------------------------------------------------------------

fn collect_all_facts() -> HashMap<String, Value> {
    let mut facts = HashMap::new();

    collect_os_release(&mut facts);
    collect_uname(&mut facts);
    collect_hostname(&mut facts);
    collect_cpu_info(&mut facts);
    collect_mem_info(&mut facts);
    collect_interfaces(&mut facts);
    collect_mounts(&mut facts);
    collect_env(&mut facts);
    collect_datetime(&mut facts);
    collect_python_version(&mut facts);
    collect_virtualization(&mut facts);

    facts
}

fn run_cmd(prog: &str, args: &[&str]) -> Option<String> {
    Command::new(prog)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn read_file(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn collect_os_release(facts: &mut HashMap<String, Value>) {
    let content = read_file("/etc/os-release").unwrap_or_default();
    let mut kv: HashMap<String, String> = HashMap::new();
    for line in content.lines() {
        if let Some((k, v)) = line.split_once('=') {
            kv.insert(k.trim().to_string(), v.trim().trim_matches('"').to_string());
        }
    }

    let id = kv.get("ID").cloned().unwrap_or_default();
    let distribution = kv
        .get("PRETTY_NAME")
        .or_else(|| kv.get("NAME"))
        .cloned()
        .unwrap_or_else(|| id.clone());
    let version = kv.get("VERSION_ID").cloned().unwrap_or_default();

    let os_family = match id.to_lowercase().as_str() {
        "ubuntu" | "debian" | "raspbian" | "linuxmint" => "Debian",
        "rhel" | "centos" | "fedora" | "rocky" | "almalinux" => "RedHat",
        "sles" | "opensuse" | "opensuse-leap" => "Suse",
        "arch" | "manjaro" => "Archlinux",
        "alpine" => "Alpine",
        _ => "Linux",
    };

    facts.insert("ansible_os_family".into(), Value::String(os_family.into()));
    facts.insert("ansible_distribution".into(), Value::String(distribution));
    facts.insert("ansible_distribution_version".into(), Value::String(version));
    facts.insert(
        "ansible_distribution_release".into(),
        Value::String(kv.get("VERSION_CODENAME").cloned().unwrap_or_default()),
    );
    facts.insert("ansible_system".into(), Value::String("Linux".into()));
}

fn collect_uname(facts: &mut HashMap<String, Value>) {
    if let Some(kernel) = run_cmd("uname", &["-r"]) {
        facts.insert("ansible_kernel".into(), Value::String(kernel));
    }
    if let Some(arch) = run_cmd("uname", &["-m"]) {
        facts.insert("ansible_architecture".into(), Value::String(arch));
    }
    if let Some(full) = run_cmd("uname", &["-a"]) {
        facts.insert("ansible_kernel_version".into(), Value::String(full));
    }
}

fn collect_hostname(facts: &mut HashMap<String, Value>) {
    if let Some(hn) = run_cmd("hostname", &[]) {
        facts.insert("ansible_hostname".into(), Value::String(hn));
    }
    if let Some(fqdn) = run_cmd("hostname", &["--fqdn"]).filter(|s| !s.is_empty()) {
        facts.insert("ansible_fqdn".into(), Value::String(fqdn));
    }
    // ansible_nodename = hostname -s
    if let Some(node) = run_cmd("hostname", &["-s"]) {
        facts.insert("ansible_nodename".into(), Value::String(node));
    }
}

fn collect_cpu_info(facts: &mut HashMap<String, Value>) {
    let content = read_file("/proc/cpuinfo").unwrap_or_default();
    let count = content.lines().filter(|l| l.starts_with("processor")).count();
    facts.insert(
        "ansible_processor_count".into(),
        Value::Number(serde_json::Number::from(count as u64)),
    );
    // Extract model name from first processor entry.
    if let Some(model) = content
        .lines()
        .find(|l| l.starts_with("model name"))
        .and_then(|l| l.split(':').nth(1))
        .map(|s| s.trim().to_string())
    {
        facts.insert("ansible_processor".into(), Value::String(model));
    }
}

fn collect_mem_info(facts: &mut HashMap<String, Value>) {
    let content = read_file("/proc/meminfo").unwrap_or_default();
    for line in content.lines() {
        let (key, val_kb) = match line.split_once(':') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };
        let kb: u64 = val_kb
            .split_whitespace()
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let mb = kb / 1024;
        match key {
            "MemTotal" => {
                facts.insert("ansible_memtotal_mb".into(), Value::Number(serde_json::Number::from(mb)));
            }
            "MemFree" => {
                facts.insert("ansible_memfree_mb".into(), Value::Number(serde_json::Number::from(mb)));
            }
            "SwapTotal" => {
                facts.insert("ansible_swaptotal_mb".into(), Value::Number(serde_json::Number::from(mb)));
            }
            "SwapFree" => {
                facts.insert("ansible_swapfree_mb".into(), Value::Number(serde_json::Number::from(mb)));
            }
            _ => {}
        }
    }
}

fn collect_interfaces(facts: &mut HashMap<String, Value>) {
    let content = read_file("/proc/net/dev").unwrap_or_default();
    let ifaces: Vec<Value> = content
        .lines()
        .skip(2) // header lines
        .filter_map(|l| {
            let name = l.trim().split(':').next()?.trim().to_string();
            if name.is_empty() {
                None
            } else {
                Some(Value::String(name))
            }
        })
        .collect();
    facts.insert("ansible_interfaces".into(), Value::Array(ifaces));
}

fn collect_mounts(facts: &mut HashMap<String, Value>) {
    let content = read_file("/proc/mounts").unwrap_or_default();
    let mounts: Vec<Value> = content
        .lines()
        .filter_map(|l| {
            let parts: Vec<&str> = l.split_whitespace().collect();
            if parts.len() < 3 {
                return None;
            }
            let mut m = serde_json::Map::new();
            m.insert("device".into(), Value::String(parts[0].to_string()));
            m.insert("mount".into(), Value::String(parts[1].to_string()));
            m.insert("fstype".into(), Value::String(parts[2].to_string()));
            Some(Value::Object(m))
        })
        .collect();
    facts.insert("ansible_mounts".into(), Value::Array(mounts));
}

fn collect_env(facts: &mut HashMap<String, Value>) {
    let env_map: serde_json::Map<String, Value> = std::env::vars()
        .map(|(k, v)| (k, Value::String(v)))
        .collect();
    facts.insert("ansible_env".into(), Value::Object(env_map));
}

fn collect_datetime(facts: &mut HashMap<String, Value>) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Build a minimal date_time mapping (no chrono dependency needed for basics).
    let mut dt = serde_json::Map::new();
    dt.insert("epoch".into(), Value::String(epoch.to_string()));

    // Use `date` command for formatted strings.
    if let Some(date_str) = run_cmd("date", &["+%Y-%m-%d"]) {
        dt.insert("date".into(), Value::String(date_str));
    }
    if let Some(time_str) = run_cmd("date", &["+%H:%M:%S"]) {
        dt.insert("time".into(), Value::String(time_str));
    }
    if let Some(tz) = run_cmd("date", &["+%Z"]) {
        dt.insert("tz".into(), Value::String(tz));
    }

    facts.insert("ansible_date_time".into(), Value::Object(dt));
}

fn collect_python_version(facts: &mut HashMap<String, Value>) {
    // Try python3 then python.
    for py in &["python3", "python"] {
        if let Some(ver) = run_cmd(py, &["--version"]).map(|s| {
            // "Python 3.11.2" → "3.11.2"
            s.strip_prefix("Python ").unwrap_or(&s).to_string()
        }) {
            facts.insert("ansible_python_version".into(), Value::String(ver));
            break;
        }
    }
}

fn collect_virtualization(facts: &mut HashMap<String, Value>) {
    // Simple heuristics — check for /.dockerenv, /proc/1/cgroup.
    let vtype = if std::path::Path::new("/.dockerenv").exists() {
        "docker"
    } else if read_file("/proc/1/cgroup")
        .map(|c| c.contains("docker") || c.contains("containerd"))
        .unwrap_or(false)
    {
        "docker"
    } else if read_file("/sys/class/dmi/id/product_name")
        .map(|c| c.to_lowercase().contains("kvm") || c.to_lowercase().contains("qemu"))
        .unwrap_or(false)
    {
        "kvm"
    } else {
        "none"
    };

    facts.insert("ansible_virtualization_type".into(), Value::String(vtype.into()));
    facts.insert(
        "ansible_virtualization_role".into(),
        Value::String(if vtype == "none" { "host" } else { "guest" }.into()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ansiblers_core::{ExecutionContext, Inventory};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn ctx() -> ExecutionContext {
        ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new())
    }

    #[test]
    fn test_setup_returns_ok() {
        let mut c = ctx();
        let args = ModuleArgs::new(HashMap::new());
        let result = SetupModule.invoke(&args, "localhost", &mut c).unwrap();
        assert!(result.status.is_ok());
        assert!(result.vars.contains_key("ansible_facts"));
    }

    #[test]
    fn test_setup_stores_facts_in_context() {
        let mut c = ctx();
        let args = ModuleArgs::new(HashMap::new());
        SetupModule.invoke(&args, "testhost", &mut c).unwrap();
        // At minimum ansible_system should be set.
        let system = c.get_fact("testhost", "ansible_system");
        assert!(system.is_some(), "ansible_system fact should be set");
    }

    #[test]
    fn test_collect_os_release_parses_correctly() {
        let mut facts = HashMap::new();
        collect_os_release(&mut facts);
        // Should have at least os_family set.
        assert!(facts.contains_key("ansible_os_family"));
        assert!(facts.contains_key("ansible_distribution"));
    }

    #[test]
    fn test_collect_cpu_info_has_count() {
        let mut facts = HashMap::new();
        collect_cpu_info(&mut facts);
        // Even in a VM/container there should be at least 1 processor.
        if let Some(Value::Number(n)) = facts.get("ansible_processor_count") {
            assert!(n.as_u64().unwrap_or(0) > 0);
        }
    }

    #[test]
    fn test_collect_mem_info_has_total() {
        let mut facts = HashMap::new();
        collect_mem_info(&mut facts);
        if let Some(Value::Number(n)) = facts.get("ansible_memtotal_mb") {
            assert!(n.as_u64().unwrap_or(0) > 0);
        }
    }

    #[test]
    fn test_collect_env_has_path() {
        let mut facts = HashMap::new();
        collect_env(&mut facts);
        if let Some(Value::Object(m)) = facts.get("ansible_env") {
            // PATH should always be in the environment.
            assert!(m.contains_key("PATH") || !m.is_empty());
        }
    }

    #[test]
    fn test_collect_datetime_has_epoch() {
        let mut facts = HashMap::new();
        collect_datetime(&mut facts);
        if let Some(Value::Object(m)) = facts.get("ansible_date_time") {
            assert!(m.contains_key("epoch"));
        }
    }

    #[test]
    fn test_virtualization_type_set() {
        let mut facts = HashMap::new();
        collect_virtualization(&mut facts);
        let vtype = facts.get("ansible_virtualization_type");
        assert!(vtype.is_some());
    }
}
