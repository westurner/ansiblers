//! Execution strategies: linear (serial) and free (parallel).
//!
//! - **Linear**: tasks run on all hosts before advancing to the next task.
//!   Matches Ansible's default `linear` strategy.
//! - **Free**: each host proceeds independently as fast as possible.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;

use ansiblers_core::{ExecutionContext, HostState, TaskResult};
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
    /// Each host runs tasks as fast as possible, independently.
    Free,
}

impl Strategy {
    pub fn from_str(s: &str) -> Self {
        match s {
            "free" => Self::Free,
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
    }
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
}
