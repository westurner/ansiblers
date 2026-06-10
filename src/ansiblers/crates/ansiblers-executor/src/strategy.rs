//! Execution strategies: linear (serial), free (parallel), and batched.
//!
//! - **Linear**: tasks run on all hosts before advancing to the next task.
//!   Matches Ansible's default `linear` strategy.
//! - **Free**: each host proceeds independently as fast as possible (OS threads).
//! - **Batch(n)**: like Free but at most `n` hosts run concurrently.
//!
//! ## Async fan-out (Phase 6)
//!
//! [`execute_tasks_multi_host_async`] provides a `tokio`-based async fan-out
//! that avoids OS-thread overhead for large inventories.  It uses
//! `tokio::task::spawn_blocking` for CPU-bound module work so the runtime
//! scheduler remains responsive.
//!
//! ```rust,no_run
//! use ansiblers_executor::strategy::{Strategy, execute_tasks_multi_host_async};
//! use ansiblers_core::{ExecutionContext, HostState};
//! use ansiblers_modules::ModuleRegistry;
//! use ansiblers_parser::TaskNode;
//! use std::collections::HashMap;
//! use std::sync::Arc;
//!
//! # async fn example() {
//! let registry = Arc::new(ModuleRegistry::with_defaults());
//! let tasks: Vec<TaskNode> = vec![];
//! let hosts = vec!["host1".to_string(), "host2".to_string()];
//! let ctx = Arc::new(tokio::sync::Mutex::new(
//!     ExecutionContext::new(Arc::new(ansiblers_core::Inventory::default()), HashMap::new()),
//! ));
//! let host_states: HashMap<String, HostState> = hosts.iter()
//!     .map(|h| (h.clone(), HostState::new(h))).collect();
//! let result = execute_tasks_multi_host_async(
//!     &tasks, &hosts, registry, ctx, host_states, None,
//! ).await.unwrap();
//! # }
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;

use ansiblers_core::{ExecutionContext, HostState, Inventory, TaskResult};
use ansiblers_modules::ModuleRegistry;
use ansiblers_parser::TaskNode;
use anyhow::Result;
use tracing::error;

use crate::task::TaskExecutor;

/// Strategy for multi-host task distribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Strategy {
    /// Run each task on all hosts before proceeding to the next task (default).
    #[default]
    Linear,
    /// Each host runs tasks as fast as possible, independently (OS threads).
    Free,
    /// Like Free but cap concurrency at `n` simultaneous hosts.
    Batch(usize),
}

impl Strategy {
    pub fn from_str(s: &str) -> Self {
        match s {
            "free" => Self::Free,
            other if other.starts_with("batch_") => {
                let n: usize = other["batch_".len()..].parse().unwrap_or(5);
                Self::Batch(n)
            }
            _ => Self::Linear,
        }
    }
}

/// Execute a task list against multiple hosts using the chosen strategy.
/// Returns a map of host → task results for this batch.
pub fn execute_tasks_multi_host(
    strategy: Strategy,
    tasks: &[TaskNode],
    hosts: &[String],
    registry: &ModuleRegistry,
    shared_ctx: &mut ExecutionContext,
    host_states: &mut HashMap<String, HostState>,
) -> Result<HashMap<String, Vec<TaskResult>>> {
    match strategy {
        Strategy::Linear => execute_linear(tasks, hosts, registry, shared_ctx, host_states),
        Strategy::Free => execute_free(tasks, hosts, registry, shared_ctx, host_states),
        Strategy::Batch(n) => execute_batched(tasks, hosts, registry, shared_ctx, host_states, n),
    }
}

/// Async fan-out: run tasks across `hosts` using Tokio tasks.
///
/// This is the Phase 6 high-performance path.  Module invocations that
/// block (I/O, subprocess) are offloaded to the blocking thread pool via
/// `tokio::task::spawn_blocking`, keeping the async runtime responsive.
///
/// Returns a merged `(results, host_states)` pair.
pub async fn execute_tasks_multi_host_async(
    tasks: &[TaskNode],
    hosts: &[String],
    registry: Arc<ModuleRegistry>,
    ctx: Arc<tokio::sync::Mutex<ExecutionContext>>,
    initial_host_states: HashMap<String, HostState>,
    batch_size: Option<usize>,
) -> Result<(HashMap<String, Vec<TaskResult>>, HashMap<String, HostState>)> {
    let batch = batch_size.unwrap_or(hosts.len().max(1));
    let merged_results: Arc<tokio::sync::Mutex<HashMap<String, Vec<TaskResult>>>> =
        Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let merged_states: Arc<tokio::sync::Mutex<HashMap<String, HostState>>> =
        Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    for chunk in hosts.chunks(batch) {
        let mut join_handles = Vec::new();

        for host in chunk {
            let host = host.clone();
            let tasks = tasks.to_vec();
            let registry = Arc::clone(&registry);
            let results_ref = Arc::clone(&merged_results);
            let states_ref = Arc::clone(&merged_states);
            // Snapshot the shared context for this host.
            let mut host_ctx = {
                let guard = ctx.lock().await;
                guard.clone()
            };
            let mut state = initial_host_states
                .get(&host)
                .cloned()
                .unwrap_or_else(|| HostState::new(&host));

            let handle = tokio::task::spawn_blocking(move || -> Result<()> {
                let executor = TaskExecutor::new(&registry);
                let mut host_results = Vec::new();

                for task_node in &tasks {
                    if !state.is_active() {
                        break;
                    }
                    match task_node {
                        TaskNode::Task(task) => {
                            let result = executor.run(task, &host, &mut host_ctx)?;
                            if result.status.is_failed() {
                                error!(host, task = ?task.name, "FAILED (async)");
                                state.failed = true;
                            } else if result.changed {
                                state.changed_count += 1;
                            } else {
                                state.ok_count += 1;
                            }
                            host_results.push(result);
                        }
                        TaskNode::Block(block) => {
                            let block_results = execute_block_single_host(
                                block,
                                &host,
                                &registry,
                                &mut host_ctx,
                                &mut state,
                            )?;
                            host_results.extend(block_results);
                        }
                    }
                }

                // Write results back — these are blocking Mutexes inside
                // spawn_blocking, which is correct.
                results_ref
                    .blocking_lock()
                    .insert(host.clone(), host_results);
                states_ref.blocking_lock().insert(host, state);
                Ok(())
            });

            join_handles.push(handle);
        }

        // Await the current batch before starting the next.
        for handle in join_handles {
            handle
                .await
                .map_err(|e| anyhow::anyhow!("tokio task panicked: {e}"))?
                .map_err(|e| anyhow::anyhow!("async host task error: {e}"))?;
        }
    }

    let results = Arc::try_unwrap(merged_results)
        .map_err(|_| anyhow::anyhow!("results arc still held"))?
        .into_inner();
    let states = Arc::try_unwrap(merged_states)
        .map_err(|_| anyhow::anyhow!("states arc still held"))?
        .into_inner();

    Ok((results, states))
}

// ---------------------------------------------------------------------------
// Linear strategy — task-by-task across all hosts
// ---------------------------------------------------------------------------

fn execute_linear(
    tasks: &[TaskNode],
    hosts: &[String],
    registry: &ModuleRegistry,
    ctx: &mut ExecutionContext,
    host_states: &mut HashMap<String, HostState>,
) -> Result<HashMap<String, Vec<TaskResult>>> {
    let executor = TaskExecutor::new(registry);
    let mut all_results: HashMap<String, Vec<TaskResult>> = HashMap::new();

    for task_node in tasks {
        for host in hosts {
            let state = host_states.get_mut(host).unwrap();
            if !state.is_active() {
                continue;
            }
            match task_node {
                TaskNode::Task(task) => {
                    let result = executor.run(task, host, ctx)?;
                    if result.status.is_failed() {
                        error!(host, task = ?task.name, "FAILED");
                        state.failed = true;
                    } else if result.changed {
                        state.changed_count += 1;
                    } else {
                        state.ok_count += 1;
                    }
                    all_results.entry(host.clone()).or_default().push(result);
                }
                TaskNode::Block(block) => {
                    // Delegate block handling back to PlayExecutor — this
                    // avoids duplicating block logic here.  We run the block
                    // per-host serially in the linear strategy.
                    let block_results =
                        execute_block_single_host(block, host, registry, ctx, state)?;
                    all_results
                        .entry(host.clone())
                        .or_default()
                        .extend(block_results);
                }
            }
        }
    }

    Ok(all_results)
}

// ---------------------------------------------------------------------------
// Free strategy — each host runs its task list in a separate thread
// ---------------------------------------------------------------------------

fn execute_free(
    tasks: &[TaskNode],
    hosts: &[String],
    _registry: &ModuleRegistry,
    ctx: &mut ExecutionContext,
    host_states: &mut HashMap<String, HostState>,
) -> Result<HashMap<String, Vec<TaskResult>>> {
    // Clone the context snapshot for each thread (facts/registered vars per-host).
    // In Phase 2 this is a clone-based approach; Phase 6 will use Arc<RwLock<>> + async.
    let registry = Arc::new(ModuleRegistry::with_defaults());
    let shared_results: Arc<Mutex<HashMap<String, Vec<TaskResult>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let shared_states: Arc<Mutex<HashMap<String, HostState>>> =
        Arc::new(Mutex::new(host_states.clone()));

    let mut handles = Vec::new();

    for host in hosts {
        let host = host.clone();
        let tasks = tasks.to_vec();
        let mut host_ctx = ctx.clone();
        let registry = Arc::clone(&registry);
        let results_ref = Arc::clone(&shared_results);
        let states_ref = Arc::clone(&shared_states);

        let handle = thread::spawn(move || -> Result<()> {
            let executor = TaskExecutor::new(&registry);
            let mut host_results = Vec::new();
            let mut state = HostState::new(&host);

            for task_node in &tasks {
                if !state.is_active() {
                    break;
                }
                match task_node {
                    TaskNode::Task(task) => {
                        let result = executor.run(task, &host, &mut host_ctx)?;
                        if result.status.is_failed() {
                            state.failed = true;
                        } else if result.changed {
                            state.changed_count += 1;
                        } else {
                            state.ok_count += 1;
                        }
                        host_results.push(result);
                    }
                    TaskNode::Block(block) => {
                        let block_results = execute_block_single_host(
                            block,
                            &host,
                            &registry,
                            &mut host_ctx,
                            &mut state,
                        )?;
                        host_results.extend(block_results);
                    }
                }
            }

            results_ref
                .lock()
                .unwrap()
                .insert(host.clone(), host_results);
            states_ref.lock().unwrap().insert(host, state);
            Ok(())
        });
        handles.push(handle);
    }

    for handle in handles {
        handle
            .join()
            .map_err(|_| anyhow::anyhow!("thread panicked"))?
            .map_err(|e| anyhow::anyhow!("thread error: {e}"))?;
    }

    // Merge thread-local states back.
    let final_states = Arc::try_unwrap(shared_states)
        .map_err(|_| anyhow::anyhow!("state arc still shared"))?
        .into_inner()
        .unwrap();
    *host_states = final_states;

    let results = Arc::try_unwrap(shared_results)
        .map_err(|_| anyhow::anyhow!("results arc still shared"))?
        .into_inner()
        .unwrap();
    Ok(results)
}

// ---------------------------------------------------------------------------
// Batched strategy — process hosts in chunks of `batch_size`
// ---------------------------------------------------------------------------

fn execute_batched(
    tasks: &[TaskNode],
    hosts: &[String],
    registry: &ModuleRegistry,
    ctx: &mut ExecutionContext,
    host_states: &mut HashMap<String, HostState>,
    batch_size: usize,
) -> Result<HashMap<String, Vec<TaskResult>>> {
    let batch_size = batch_size.max(1);
    let registry = Arc::new(ModuleRegistry::with_defaults());
    let mut all_results: HashMap<String, Vec<TaskResult>> = HashMap::new();

    for chunk in hosts.chunks(batch_size) {
        let shared_results: Arc<Mutex<HashMap<String, Vec<TaskResult>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let shared_states: Arc<Mutex<HashMap<String, HostState>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let mut handles = Vec::new();

        for host in chunk {
            let host = host.clone();
            let tasks = tasks.to_vec();
            let registry = Arc::clone(&registry);
            let mut host_ctx = ctx.clone();
            let results_ref = Arc::clone(&shared_results);
            let states_ref = Arc::clone(&shared_states);
            let mut state = host_states
                .get(&host)
                .cloned()
                .unwrap_or_else(|| HostState::new(&host));

            let handle = thread::spawn(move || -> Result<()> {
                let executor = TaskExecutor::new(&registry);
                let mut host_results = Vec::new();

                for task_node in &tasks {
                    if !state.is_active() {
                        break;
                    }
                    match task_node {
                        TaskNode::Task(task) => {
                            let result = executor.run(task, &host, &mut host_ctx)?;
                            if result.status.is_failed() {
                                state.failed = true;
                            } else if result.changed {
                                state.changed_count += 1;
                            } else {
                                state.ok_count += 1;
                            }
                            host_results.push(result);
                        }
                        TaskNode::Block(block) => {
                            let block_results = execute_block_single_host(
                                block,
                                &host,
                                &registry,
                                &mut host_ctx,
                                &mut state,
                            )?;
                            host_results.extend(block_results);
                        }
                    }
                }

                results_ref
                    .lock()
                    .unwrap()
                    .insert(host.clone(), host_results);
                states_ref.lock().unwrap().insert(host, state);
                Ok(())
            });
            handles.push(handle);
        }

        for handle in handles {
            handle
                .join()
                .map_err(|_| anyhow::anyhow!("batch thread panicked"))?
                .map_err(|e| anyhow::anyhow!("batch thread error: {e}"))?;
        }

        let batch_results = Arc::try_unwrap(shared_results)
            .map_err(|_| anyhow::anyhow!("results arc still shared"))?
            .into_inner()
            .unwrap();
        let batch_states = Arc::try_unwrap(shared_states)
            .map_err(|_| anyhow::anyhow!("states arc still shared"))?
            .into_inner()
            .unwrap();

        all_results.extend(batch_results);
        host_states.extend(batch_states);
    }

    Ok(all_results)
}

// ---------------------------------------------------------------------------
// Block helper
// ---------------------------------------------------------------------------

fn execute_block_single_host(
    block: &ansiblers_parser::Block,
    host: &str,
    registry: &ModuleRegistry,
    ctx: &mut ExecutionContext,
    state: &mut HostState,
) -> Result<Vec<TaskResult>> {
    let executor = TaskExecutor::new(registry);
    let mut results = Vec::new();

    // Main block.
    let mut block_failed = false;
    for task_node in &block.block {
        if !state.is_active() {
            break;
        }
        match task_node {
            TaskNode::Task(task) => {
                let result = executor.run(task, host, ctx)?;
                if result.status.is_failed() {
                    state.failed = true;
                    block_failed = true;
                }
                results.push(result);
            }
            TaskNode::Block(inner) => {
                let inner_results = execute_block_single_host(inner, host, registry, ctx, state)?;
                results.extend(inner_results);
                block_failed = state.failed;
            }
        }
    }

    // Rescue.
    if block_failed && !block.rescue.is_empty() {
        state.failed = false;
        for task_node in &block.rescue {
            match task_node {
                TaskNode::Task(task) => {
                    let result = executor.run(task, host, ctx)?;
                    results.push(result);
                }
                TaskNode::Block(inner) => {
                    let inner_results =
                        execute_block_single_host(inner, host, registry, ctx, state)?;
                    results.extend(inner_results);
                }
            }
        }
    }

    // Always.
    if !block.always.is_empty() {
        let saved_failed = state.failed;
        state.failed = false;
        for task_node in &block.always {
            match task_node {
                TaskNode::Task(task) => {
                    let result = executor.run(task, host, ctx)?;
                    results.push(result);
                }
                TaskNode::Block(inner) => {
                    let inner_results =
                        execute_block_single_host(inner, host, registry, ctx, state)?;
                    results.extend(inner_results);
                }
            }
        }
        if !state.failed {
            state.failed = saved_failed;
        }
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ansiblers_core::{Inventory, Value};
    use ansiblers_modules::ModuleRegistry;
    use ansiblers_parser::{parse_playbook_str, TaskNode};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn registry() -> ModuleRegistry {
        ModuleRegistry::with_defaults()
    }

    fn ctx() -> ExecutionContext {
        ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new())
    }

    #[test]
    fn test_linear_strategy() {
        let yaml = "- hosts: all\n  gather_facts: false\n  tasks:\n    - shell: echo linear\n";
        let pb = parse_playbook_str(yaml, None).unwrap();
        let play = &pb.plays[0];

        let registry = registry();
        let mut ctx = ctx();
        let hosts = vec!["localhost".to_string()];
        let mut states: HashMap<String, HostState> = hosts
            .iter()
            .map(|h| (h.clone(), HostState::new(h)))
            .collect();

        let results = execute_tasks_multi_host(
            Strategy::Linear,
            &play.tasks,
            &hosts,
            &registry,
            &mut ctx,
            &mut states,
        )
        .unwrap();

        assert!(results["localhost"][0].stdout.contains("linear"));
    }

    #[test]
    fn test_free_strategy_two_hosts() {
        // Two independent hosts — both should complete successfully.
        let yaml = "- hosts: all\n  gather_facts: false\n  tasks:\n    - shell: echo free\n";
        let pb = parse_playbook_str(yaml, None).unwrap();
        let play = &pb.plays[0];

        let registry = registry();
        let mut ctx = ctx();
        let hosts = vec!["host1".to_string(), "host2".to_string()];
        let mut states: HashMap<String, HostState> = hosts
            .iter()
            .map(|h| (h.clone(), HostState::new(h)))
            .collect();

        let results = execute_tasks_multi_host(
            Strategy::Free,
            &play.tasks,
            &hosts,
            &registry,
            &mut ctx,
            &mut states,
        )
        .unwrap();

        assert!(results["host1"][0].stdout.contains("free"));
        assert!(results["host2"][0].stdout.contains("free"));
    }

    #[test]
    fn test_batch_strategy_three_hosts_batch_two() {
        let yaml = "- hosts: all\n  gather_facts: false\n  tasks:\n    - shell: echo batch\n";
        let pb = parse_playbook_str(yaml, None).unwrap();
        let play = &pb.plays[0];

        let registry = registry();
        let mut ctx = ctx();
        let hosts = vec!["h1".to_string(), "h2".to_string(), "h3".to_string()];
        let mut states: HashMap<String, HostState> = hosts
            .iter()
            .map(|h| (h.clone(), HostState::new(h)))
            .collect();

        let results = execute_tasks_multi_host(
            Strategy::Batch(2),
            &play.tasks,
            &hosts,
            &registry,
            &mut ctx,
            &mut states,
        )
        .unwrap();

        for h in &hosts {
            assert!(
                results[h][0].stdout.contains("batch"),
                "host {h} missing result"
            );
        }
    }

    #[test]
    fn test_strategy_from_str() {
        assert_eq!(Strategy::from_str("linear"), Strategy::Linear);
        assert_eq!(Strategy::from_str("free"), Strategy::Free);
        assert_eq!(Strategy::from_str("batch_4"), Strategy::Batch(4));
        assert_eq!(Strategy::from_str("unknown"), Strategy::Linear);
    }

    #[tokio::test]
    async fn test_async_fan_out() {
        let yaml =
            "- hosts: all\n  gather_facts: false\n  tasks:\n    - shell: echo async_fanout\n";
        let pb = parse_playbook_str(yaml, None).unwrap();
        let play = &pb.plays[0];

        let registry = Arc::new(ModuleRegistry::with_defaults());
        let hosts = vec!["a1".to_string(), "a2".to_string()];
        let ctx = Arc::new(tokio::sync::Mutex::new(ExecutionContext::new(
            Arc::new(Inventory::default()),
            HashMap::new(),
        )));
        let states: HashMap<String, HostState> = hosts
            .iter()
            .map(|h| (h.clone(), HostState::new(h)))
            .collect();

        let (results, final_states) =
            execute_tasks_multi_host_async(&play.tasks, &hosts, registry, ctx, states, None)
                .await
                .unwrap();

        for h in &hosts {
            assert!(results[h][0].stdout.contains("async_fanout"), "host {h}");
            assert!(!final_states[h].failed, "host {h} should not be failed");
        }
    }

    #[tokio::test]
    async fn test_async_fan_out_batch_size() {
        let yaml =
            "- hosts: all\n  gather_facts: false\n  tasks:\n    - shell: echo batched_async\n";
        let pb = parse_playbook_str(yaml, None).unwrap();
        let play = &pb.plays[0];

        let registry = Arc::new(ModuleRegistry::with_defaults());
        let hosts: Vec<String> = (1..=4).map(|i| format!("node{i}")).collect();
        let ctx = Arc::new(tokio::sync::Mutex::new(ExecutionContext::new(
            Arc::new(Inventory::default()),
            HashMap::new(),
        )));
        let states: HashMap<String, HostState> = hosts
            .iter()
            .map(|h| (h.clone(), HostState::new(h)))
            .collect();

        let (results, _) = execute_tasks_multi_host_async(
            &play.tasks,
            &hosts,
            registry,
            ctx,
            states,
            Some(2), // batch of 2
        )
        .await
        .unwrap();

        assert_eq!(results.len(), 4);
        for h in &hosts {
            assert!(results[h][0].stdout.contains("batched_async"), "host {h}");
        }
    }
}
