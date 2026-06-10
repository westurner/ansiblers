# Ansiblers Performance Tuning Guide

This document covers the performance-oriented features introduced in Phase 6 and
explains how to configure them for maximum throughput on large inventories.

---

## Execution Strategies

Ansiblers supports three multi-host execution strategies, controlled by the
`--strategy` CLI flag or the `strategy:` key in a play.

### Linear (default)

Each task runs on **all hosts** before moving to the next task.  This matches
Ansible's default behaviour and gives the most predictable output ordering.

```yaml
- hosts: all
  strategy: linear   # default — same as omitting the key
  tasks:
    - name: install packages
      apt:
        name: nginx
```

**When to use**: When tasks depend on each other across hosts (e.g. rolling
updates where each host must finish before the next starts), or when output
ordering matters.

### Free

Each host processes tasks **independently** using OS threads.  Hosts that finish
quickly don't wait for slow ones.

```yaml
- hosts: all
  strategy: free
  tasks: ...
```

**When to use**: Large inventories with independent tasks (deploy, restart,
status checks).  Expect 2-8× speedup over `linear` on I/O-bound tasks
when the control node has spare CPU cores.

**Note**: Output lines from different hosts may be interleaved.

### Batch(n)

Like `Free` but at most `n` hosts run concurrently.  Prevents the control node
from spawning hundreds of threads against a very large inventory.

```yaml
- hosts: all
  strategy: batch_8   # at most 8 hosts in flight at once
  tasks: ...
```

Parse examples: `batch_4`, `batch_16`, `batch_1` (equivalent to linear).

**When to use**: When you need parallel execution but want to limit concurrency
(rate-limiting SSH connections, respecting API throttles, protecting target
services during rolling updates).

---

## Async Fan-Out (`execute_tasks_multi_host_async`)

For programmatic usage, the `execute_tasks_multi_host_async` function provides
Tokio-based fan-out without spawning OS threads per host.

```rust
use ansiblers_executor::strategy::execute_tasks_multi_host_async;
use ansiblers_core::{ExecutionContext, HostState, Inventory};
use ansiblers_modules::ModuleRegistry;
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let registry = Arc::new(ModuleRegistry::with_defaults());
    let hosts = vec!["web1".to_string(), "web2".to_string(), "web3".to_string()];
    let ctx = Arc::new(tokio::sync::Mutex::new(
        ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new()),
    ));
    let host_states: HashMap<String, HostState> = hosts.iter()
        .map(|h| (h.clone(), HostState::new(h)))
        .collect();

    let results = execute_tasks_multi_host_async(
        &tasks,
        &hosts,
        registry,
        ctx,
        host_states,
        Some(4),  // batch_size — None means all hosts at once
    ).await?;
    Ok(())
}
```

CPU-bound module work is dispatched via `tokio::task::spawn_blocking` so the
async runtime remains responsive to I/O.

---

## Module Result Caching

Read-only, idempotent modules (e.g. `stat`, `setup`, `gather_facts`, `find`)
return the same result every time they are called with the same arguments on the
same host within a single playbook run.  The caching layer skips the underlying
work on repeated calls.

### Built-in cacheable modules

| Module | Why cacheable |
|--------|---------------|
| `stat` | File metadata doesn't change during a run |
| `setup` / `gather_facts` | Facts are collected once per host |
| `find` | Directory contents are stable during execution |

### In-memory cache (default, per-run)

```rust
use ansiblers_modules::{CachingModuleRegistry, InMemoryCache, ModuleRegistry};
use std::sync::Arc;

let inner = ModuleRegistry::with_defaults();
let cache: Arc<dyn ansiblers_modules::ModuleResultCache> = Arc::new(InMemoryCache::default());
let cached = CachingModuleRegistry::new(inner, cache);
```

The `InMemoryCache` is cleared when the registry is dropped (i.e. at the end of
the playbook run).

### SQLite cache (cross-run persistence)

```rust
use ansiblers_modules::SqliteCache;
use std::sync::Arc;

let cache = Arc::new(SqliteCache::open("/var/cache/ansiblers/modules.db")?);
// Pass to CachingModuleRegistry as above.
```

Use `:memory:` for an in-process SQLite database that persists across plays
within the same process but is discarded on exit.

### Hit-ratio telemetry

```rust
println!("Cache hit ratio: {:.1}%", cached.hit_ratio() * 100.0);
// Cache hit ratio: 73.5%
```

Expose `lookups` and `hits` counters via metrics pipelines (Prometheus, etc.)
for production observability.

### Cache invalidation

```rust
cached.invalidate_host("web1");  // Remove all entries for a specific host
cached.clear_cache();            // Wipe the entire cache
```

---

## Connection Providers

The `ConnectionProvider` trait abstracts the transport used to run a module on a
target host.  Switching providers does not require changes to executor logic.

### Local (default, zero overhead)

Used for `connection: local` tasks — the module runs in-process with no
subprocess or network overhead.  Ideal for localhost tasks and testing.

### SSH

Used for `connection: ssh` (the default for remote hosts).  The provider builds
an `ssh` command from `ansible_host`, `ansible_port`, `ansible_user`,
`ansible_ssh_private_key_file`, and `ansible_become*` inventory variables.

```ini
[webservers]
web1 ansible_host=192.168.1.10 ansible_user=deploy ansible_ssh_private_key_file=~/.ssh/deploy_key
```

SSH args are constructed as:

```
ssh -p <port> -i <key> [-o <extra>] <user>@<host> [sudo -u root] <command>
```

### Custom providers

```rust
use ansiblers_executor::connection::{ConnectionContext, ConnectionProvider, ConnectionRegistry};
use ansiblers_core::{ExecutionContext, TaskResult};
use ansiblers_modules::ModuleArgs;
use anyhow::Result;

struct KubernetesExecProvider;

impl ConnectionProvider for KubernetesExecProvider {
    fn name(&self) -> &str { "kubectl_exec" }

    fn invoke_module(
        &self,
        module: &str,
        args: &ModuleArgs,
        host: &str,
        conn_ctx: &ConnectionContext,
        exec_ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        // kubectl exec -n <namespace> <pod> -- ...
        todo!()
    }
}

let mut registry = ConnectionRegistry::new();
registry.register(std::sync::Arc::new(KubernetesExecProvider));
```

---

## Template Rendering Backends

Ansiblers supports three Jinja2 rendering backends, selectable via
`ANSIBLE_TEMPLATE_BACKEND` or via `BackendSelector` in the Rust API.

| Backend | Env value | Description | Ansible filters | Speed |
|---------|-----------|-------------|-----------------|-------|
| `jinja2rs` | `jinja2rs`, `jinja2r2`, `compat` | jinja2rs with Ansible compat layer **(default)** | ✅ Full | ★★★★ |
| `minijinja` | `minijinja`, `rust` | Raw minijinja, no compat wrapper | ❌ Built-ins only | ★★★★★ |
| `python_jinja2` | `python`, `cpython` | CPython Jinja2 via `python3` subprocess | ✅ Full | ★ |

```bash
# Default (jinja2rs + Ansible compat filters):
ransible-playbook site.yml

# Raw minijinja — benchmark baseline:
ANSIBLE_TEMPLATE_BACKEND=minijinja ransible-playbook site.yml

# Python Jinja2 — 100% compatibility check:
ANSIBLE_TEMPLATE_BACKEND=python ransible-playbook site.yml
```

```rust
use ansiblers_templates::backend::{BackendSelector, TemplateBackend};
use std::collections::HashMap;
use serde_json::Value;

// Programmatic backend selection:
let sel = BackendSelector::new(TemplateBackend::Minijinja);
let out = sel.render("{{ name | upper }}", &vars)?;
```

Use `TemplateBackend::Minijinja` for benchmarks to isolate raw parse/render cost
from Ansible-filter overhead.  Use `Jinja2rs` in production — it is the same
minijinja engine with Ansible-mode compat registered on top.

Criterion benchmarks are included in `crates/ansiblers-executor/benches/`.

```bash
# Run all executor benchmarks
cargo bench -p ansiblers-executor

# Run only the multi-host scaling group
cargo bench -p ansiblers-executor --bench playbook_execution -- multi_host

# Compare against a saved baseline
cargo bench -p ansiblers-executor -- --save-baseline before_change
# ... make changes ...
cargo bench -p ansiblers-executor -- --baseline before_change
```

### Benchmark coverage

| Benchmark | File |
|-----------|------|
| Parse simple/multi-task playbooks | `benches/playbook_execution.rs` |
| Execute simple / loop / block plays | `benches/playbook_execution.rs` |
| Multi-host scaling (1/2/4/8 hosts) | `benches/playbook_execution.rs` |
| Async fan-out (2/4/8 hosts) | `benches/playbook_execution.rs` |
| Simple variable interpolation | `benches/variable_resolution.rs` |
| Nested dict access | `benches/variable_resolution.rs` |
| Loop context + Jinja2 filters | `benches/variable_resolution.rs` |
| 100-var merge | `benches/variable_resolution.rs` |

Reports land in `target/criterion/`.  Open `target/criterion/report/index.html`
for the full HTML comparison report.

---

## General Tips

1. **Prefer `batch_N` over `free`** when your inventory has >50 hosts — OS
   threads have ~8 KB stack overhead each.
2. **Enable caching** for plays that call `stat` or `setup` on many hosts.
3. **Colocate the control node** with targets to minimise SSH round-trip latency.
4. **Use `connection: local`** for tasks that target the control node itself
   (e.g. updating a local database, writing files).
5. **Profile before optimising** — run `cargo bench` to establish a baseline,
   then confirm improvements rather than guessing.
