//! Benchmarks for playbook parsing and end-to-end execution.
//!
//! Run with:
//! ```bash
//! cargo bench --bench playbook_execution
//! cargo bench --bench playbook_execution -- --save-baseline main
//! cargo bench --bench playbook_execution -- --baseline main
//! ```

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::collections::HashMap;
use std::sync::Arc;

use ansiblers_core::{Inventory, TaskResult};
use ansiblers_executor::PlayExecutor;
use ansiblers_modules::ModuleRegistry;
use ansiblers_parser::parse_playbook_str;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const SIMPLE_PLAYBOOK: &str = r#"
- hosts: localhost
  gather_facts: false
  tasks:
    - name: Echo a message
      shell: echo "hello from benchmark"
    - name: Set a variable
      set_fact:
        bench_var: "value"
    - name: Print variable
      debug:
        msg: "{{ bench_var }}"
"#;

const MULTI_TASK_PLAYBOOK: &str = r#"
- hosts: localhost
  gather_facts: false
  vars:
    greeting: "bench"
  tasks:
    - shell: echo "task 1"
    - shell: echo "task 2"
    - shell: echo "task 3"
    - shell: echo "task 4"
    - shell: echo "task 5"
    - set_fact:
        result: "done"
    - debug:
        msg: "{{ result }}"
"#;

const LOOP_PLAYBOOK: &str = r#"
- hosts: localhost
  gather_facts: false
  tasks:
    - name: Loop over items
      debug:
        msg: "item={{ item }}"
      loop:
        - alpha
        - beta
        - gamma
        - delta
        - epsilon
"#;

const BLOCK_PLAYBOOK: &str = r#"
- hosts: localhost
  gather_facts: false
  tasks:
    - block:
        - shell: echo "in block"
        - set_fact:
            block_done: true
      rescue:
        - debug:
            msg: "rescued"
      always:
        - debug:
            msg: "always runs"
"#;

// ---------------------------------------------------------------------------
// Parsing benchmarks
// ---------------------------------------------------------------------------

fn bench_parse_simple(c: &mut Criterion) {
    c.bench_function("parse_simple_playbook", |b| {
        b.iter(|| parse_playbook_str(black_box(SIMPLE_PLAYBOOK), None).unwrap())
    });
}

fn bench_parse_multi_task(c: &mut Criterion) {
    c.bench_function("parse_multi_task_playbook", |b| {
        b.iter(|| parse_playbook_str(black_box(MULTI_TASK_PLAYBOOK), None).unwrap())
    });
}

// ---------------------------------------------------------------------------
// Execution benchmarks
// ---------------------------------------------------------------------------

fn bench_execute_simple(c: &mut Criterion) {
    let registry = ModuleRegistry::with_defaults();
    let executor = PlayExecutor::new(registry);
    let playbook = parse_playbook_str(SIMPLE_PLAYBOOK, None).unwrap();

    c.bench_function("execute_simple_playbook", |b| {
        b.iter(|| {
            let mut ctx = ansiblers_core::ExecutionContext::new(
                Arc::new(Inventory::default()),
                HashMap::new(),
            );
            executor
                .run_playbook(black_box(&playbook), &mut ctx)
                .unwrap()
        })
    });
}

fn bench_execute_multi_task(c: &mut Criterion) {
    let registry = ModuleRegistry::with_defaults();
    let executor = PlayExecutor::new(registry);
    let playbook = parse_playbook_str(MULTI_TASK_PLAYBOOK, None).unwrap();

    c.bench_function("execute_multi_task_playbook", |b| {
        b.iter(|| {
            let mut ctx = ansiblers_core::ExecutionContext::new(
                Arc::new(Inventory::default()),
                HashMap::new(),
            );
            executor
                .run_playbook(black_box(&playbook), &mut ctx)
                .unwrap()
        })
    });
}

fn bench_execute_loop(c: &mut Criterion) {
    let registry = ModuleRegistry::with_defaults();
    let executor = PlayExecutor::new(registry);
    let playbook = parse_playbook_str(LOOP_PLAYBOOK, None).unwrap();

    c.bench_function("execute_loop_playbook", |b| {
        b.iter(|| {
            let mut ctx = ansiblers_core::ExecutionContext::new(
                Arc::new(Inventory::default()),
                HashMap::new(),
            );
            executor
                .run_playbook(black_box(&playbook), &mut ctx)
                .unwrap()
        })
    });
}

fn bench_execute_block(c: &mut Criterion) {
    let registry = ModuleRegistry::with_defaults();
    let executor = PlayExecutor::new(registry);
    let playbook = parse_playbook_str(BLOCK_PLAYBOOK, None).unwrap();

    c.bench_function("execute_block_playbook", |b| {
        b.iter(|| {
            let mut ctx = ansiblers_core::ExecutionContext::new(
                Arc::new(Inventory::default()),
                HashMap::new(),
            );
            executor
                .run_playbook(black_box(&playbook), &mut ctx)
                .unwrap()
        })
    });
}

// ---------------------------------------------------------------------------
// Multi-host parallelism benchmarks
// ---------------------------------------------------------------------------

fn bench_multi_host_scaling(c: &mut Criterion) {
    let yaml = "- hosts: all\n  gather_facts: false\n  tasks:\n    - debug:\n        msg: bench\n";
    let playbook = parse_playbook_str(yaml, None).unwrap();

    let mut group = c.benchmark_group("multi_host_scaling");
    for n_hosts in [1usize, 2, 4, 8] {
        group.bench_with_input(BenchmarkId::from_parameter(n_hosts), &n_hosts, |b, &n| {
            let registry = ModuleRegistry::with_defaults();
            let executor = PlayExecutor::new(registry);
            b.iter(|| {
                let mut inv = ansiblers_core::Inventory::default();
                for i in 0..n {
                    let h = ansiblers_core::Host::new(format!("host{i}"));
                    inv.hosts.insert(h.name.clone(), h);
                }
                let mut ctx = ansiblers_core::ExecutionContext::new(Arc::new(inv), HashMap::new());
                executor
                    .run_playbook(black_box(&playbook), &mut ctx)
                    .unwrap()
            });
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Async fan-out benchmark
// ---------------------------------------------------------------------------

fn bench_async_fan_out(c: &mut Criterion) {
    use ansiblers_core::HostState;
    use ansiblers_executor::strategy::execute_tasks_multi_host_async;
    use tokio::runtime::Runtime;

    let yaml =
        "- hosts: all\n  gather_facts: false\n  tasks:\n    - debug:\n        msg: async_bench\n";
    let playbook = parse_playbook_str(yaml, None).unwrap();
    let tasks = playbook.plays[0].tasks.clone();

    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("async_fan_out");
    for n_hosts in [2usize, 4, 8] {
        group.bench_with_input(BenchmarkId::from_parameter(n_hosts), &n_hosts, |b, &n| {
            let hosts: Vec<String> = (0..n).map(|i| format!("ahost{i}")).collect();
            let registry = Arc::new(ModuleRegistry::with_defaults());
            b.iter(|| {
                let ctx = Arc::new(tokio::sync::Mutex::new(
                    ansiblers_core::ExecutionContext::new(
                        Arc::new(Inventory::default()),
                        HashMap::new(),
                    ),
                ));
                let states: HashMap<String, HostState> = hosts
                    .iter()
                    .map(|h| (h.clone(), HostState::new(h)))
                    .collect();
                rt.block_on(execute_tasks_multi_host_async(
                    black_box(&tasks),
                    black_box(&hosts),
                    Arc::clone(&registry),
                    ctx,
                    states,
                    None,
                ))
                .unwrap()
            });
        });
    }
    group.finish();
}

criterion_group!(parse_benches, bench_parse_simple, bench_parse_multi_task,);

criterion_group!(
    execute_benches,
    bench_execute_simple,
    bench_execute_multi_task,
    bench_execute_loop,
    bench_execute_block,
    bench_multi_host_scaling,
    bench_async_fan_out,
);

criterion_main!(parse_benches, execute_benches);
