//! Play-level executor — iterates over hosts and task lists.

use std::collections::HashMap;

use anyhow::Result;
use ansiblers_core::{ExecutionContext, HostState, TaskResult};
use ansiblers_modules::ModuleRegistry;
use ansiblers_parser::{Block, Play, Playbook, TaskNode};
use tracing::{error, info, warn};

use crate::task::TaskExecutor;

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct PlaybookResult {
    pub play_results: Vec<PlayResult>,
    pub success: bool,
}

#[derive(Debug)]
pub struct PlayResult {
    pub play_name: Option<String>,
    pub host_results: HashMap<String, Vec<TaskResult>>,
    pub success: bool,
}

// ---------------------------------------------------------------------------
// PlayExecutor
// ---------------------------------------------------------------------------

pub struct PlayExecutor {
    registry: ModuleRegistry,
}

impl PlayExecutor {
    pub fn new(registry: ModuleRegistry) -> Self {
        Self { registry }
    }

    /// Execute an entire playbook.
    pub fn run_playbook(
        &self,
        playbook: &Playbook,
        ctx: &mut ExecutionContext,
    ) -> Result<PlaybookResult> {
        let mut play_results = Vec::new();
        let mut success = true;

        for play in &playbook.plays {
            let result = self.run_play(play, ctx)?;
            if !result.success {
                success = false;
            }
            play_results.push(result);
        }

        Ok(PlaybookResult {
            play_results,
            success,
        })
    }

    /// Execute a single play.
    pub fn run_play(&self, play: &Play, ctx: &mut ExecutionContext) -> Result<PlayResult> {
        info!(
            play = play.name.as_deref().unwrap_or("(unnamed)"),
            hosts = %play.hosts,
            "PLAY"
        );

        // Merge play-level vars into context.
        for (k, v) in &play.vars {
            ctx.playbook_vars.insert(k.clone(), v.clone());
        }

        let hosts = ctx.inventory.matching_hosts(&play.hosts);
        if hosts.is_empty() {
            warn!(pattern = %play.hosts, "no hosts matched");
        }

        let mut host_results: HashMap<String, Vec<TaskResult>> = HashMap::new();
        let mut host_states: HashMap<String, HostState> = hosts
            .iter()
            .map(|h| (h.clone(), HostState::new(h)))
            .collect();

        let task_executor = TaskExecutor::new(&self.registry);

        // Execute task list for each host.
        for host in &hosts {
            let state = host_states.get_mut(host).unwrap();
            let results =
                self.run_task_list(&play.tasks, host, ctx, state, &task_executor)?;
            host_results.insert(host.clone(), results);
        }

        // Run handlers (triggered hosts not tracked in Phase 1 — run all).
        if !play.handlers.is_empty() {
            for host in &hosts {
                let state = host_states.get_mut(host).unwrap();
                if !state.is_active() {
                    continue;
                }
                for handler in &play.handlers {
                    let result = task_executor.run(handler, host, ctx)?;
                    host_results
                        .entry(host.clone())
                        .or_default()
                        .push(result);
                }
            }
        }

        let success = host_states.values().all(|s| s.is_active());
        Ok(PlayResult {
            play_name: play.name.clone(),
            host_results,
            success,
        })
    }

    fn run_task_list(
        &self,
        tasks: &[TaskNode],
        host: &str,
        ctx: &mut ExecutionContext,
        state: &mut HostState,
        executor: &TaskExecutor<'_>,
    ) -> Result<Vec<TaskResult>> {
        let mut results = Vec::new();

        for node in tasks {
            if !state.is_active() {
                break;
            }
            match node {
                TaskNode::Task(task) => {
                    let result = executor.run(task, host, ctx)?;
                    let failed = result.status.is_failed();
                    if result.changed {
                        state.changed_count += 1;
                    } else if result.status.is_ok() {
                        state.ok_count += 1;
                    }
                    if failed {
                        error!(task = ?task.name, host, "FAILED");
                        state.failed = true;
                    }
                    results.push(result);
                }
                TaskNode::Block(block) => {
                    let block_results =
                        self.run_block(block, host, ctx, state, executor)?;
                    results.extend(block_results);
                }
            }
        }

        Ok(results)
    }

    fn run_block(
        &self,
        block: &Block,
        host: &str,
        ctx: &mut ExecutionContext,
        state: &mut HostState,
        executor: &TaskExecutor<'_>,
    ) -> Result<Vec<TaskResult>> {
        let mut results = Vec::new();

        // Execute the main block.
        let block_results =
            self.run_task_list(&block.block, host, ctx, state, executor)?;
        let block_failed = state.failed;
        results.extend(block_results);

        // If block failed, run rescue (resetting failed state).
        if block_failed && !block.rescue.is_empty() {
            state.failed = false;
            let rescue_results =
                self.run_task_list(&block.rescue, host, ctx, state, executor)?;
            results.extend(rescue_results);
        }

        // Always block runs regardless.
        if !block.always.is_empty() {
            let saved_failed = state.failed;
            state.failed = false;
            let always_results =
                self.run_task_list(&block.always, host, ctx, state, executor)?;
            results.extend(always_results);
            // Restore failed state if always didn't fail itself.
            if !state.failed {
                state.failed = saved_failed;
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ansiblers_core::{Inventory, Value};
    use ansiblers_modules::ModuleRegistry;
    use ansiblers_parser::parse_playbook_str;
    use std::sync::Arc;

    fn executor() -> PlayExecutor {
        PlayExecutor::new(ModuleRegistry::with_defaults())
    }

    fn ctx() -> ExecutionContext {
        ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new())
    }

    #[test]
    fn test_run_simple_playbook() {
        let yaml = r#"
- hosts: localhost
  gather_facts: false
  tasks:
    - name: Echo
      shell: echo hello
      register: result
    - name: Debug
      debug:
        msg: "{{ result.stdout }}"
"#;
        let pb = parse_playbook_str(yaml, None).unwrap();
        let mut ctx = ctx();
        let result = executor().run_playbook(&pb, &mut ctx).unwrap();
        assert!(result.success);
    }

    #[test]
    fn test_run_play_with_vars() {
        let yaml = r#"
- hosts: localhost
  gather_facts: false
  vars:
    greeting: "hello"
  tasks:
    - shell: echo {{ greeting }}
      register: r
"#;
        let pb = parse_playbook_str(yaml, None).unwrap();
        let mut ctx = ctx();
        let result = executor().run_playbook(&pb, &mut ctx).unwrap();
        assert!(result.success);
        let reg = ctx.get_var("r").unwrap();
        assert!(reg["stdout"].as_str().unwrap().contains("hello"));
    }

    #[test]
    fn test_block_rescue() {
        let yaml = r#"
- hosts: localhost
  gather_facts: false
  tasks:
    - block:
        - shell: /bin/false
      rescue:
        - set_fact:
            rescued: "yes"
      always:
        - set_fact:
            always_ran: "yes"
"#;
        let pb = parse_playbook_str(yaml, None).unwrap();
        let mut ctx = ctx();
        let result = executor().run_playbook(&pb, &mut ctx).unwrap();
        // Rescue should have run, so play succeeds.
        assert!(result.success);
        assert_eq!(
            ctx.get_fact("localhost", "rescued"),
            Some(&Value::String("yes".to_string()))
        );
        assert_eq!(
            ctx.get_fact("localhost", "always_ran"),
            Some(&Value::String("yes".to_string()))
        );
    }
}
