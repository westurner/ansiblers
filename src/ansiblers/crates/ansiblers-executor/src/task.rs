//! Single-task executor — resolves templates, evaluates conditions, runs module.

use std::collections::HashMap;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use ansiblers_modules::{ModuleArgs, ModuleRegistry};
use ansiblers_parser::{Task, TaskArgs};
use ansiblers_templates::{AnsibleTemplateEngine, TemplateEngineConfig, TrustLevel};
use ansiblers_vars::VariableResolver;
use anyhow::Result;
use tracing::{debug, info, warn};

pub struct TaskExecutor<'reg> {
    registry: &'reg ModuleRegistry,
    /// Trust level applied to all template rendering in this executor.
    /// `TrustLevel::Trusted` (default) has zero overhead.
    pub trust_level: TrustLevel,
}

impl<'reg> TaskExecutor<'reg> {
    pub fn new(registry: &'reg ModuleRegistry) -> Self {
        Self {
            registry,
            trust_level: TrustLevel::Trusted,
        }
    }

    /// Create an executor with a specific trust level for template rendering.
    pub fn with_trust(registry: &'reg ModuleRegistry, trust_level: TrustLevel) -> Self {
        Self {
            registry,
            trust_level,
        }
    }

    /// Execute `task` for `host`, returning the task result.
    pub fn run(&self, task: &Task, host: &str, ctx: &mut ExecutionContext) -> Result<TaskResult> {
        let resolver = VariableResolver::new(ctx);
        let vars = resolver.merged(host);

        // Evaluate `when:` condition — skip if false.
        if let Some(when_expr) = &task.when {
            for condition in when_expr.conditions() {
                let rendered = render_when(condition, &vars, self.trust_level);
                if !evaluate_bool(&rendered) {
                    debug!(task = ?task.name, host, "skipped (when condition false)");
                    let mut r = TaskResult::skipped(host);
                    r.task_name = task.name.clone();
                    return Ok(r);
                }
            }
        }

        // If this task has a loop, iterate.
        if let Some(items) = &task.loop_items {
            return self.run_loop(task, host, ctx, items, &task.loop_var);
        }

        info!(
            task = task.name.as_deref().unwrap_or(&task.module),
            module = %task.module,
            host,
            "TASK"
        );

        let result = self.run_single(task, host, ctx, &vars)?;
        self.post_task(task, host, ctx, &result)?;
        Ok(result)
    }

    /// Run a task once per loop item.
    fn run_loop(
        &self,
        task: &Task,
        host: &str,
        ctx: &mut ExecutionContext,
        items: &Value,
        loop_var: &str,
    ) -> Result<TaskResult> {
        let list = match items {
            Value::Array(arr) => arr.clone(),
            other => vec![other.clone()],
        };

        let mut last_result = TaskResult::ok(host);
        last_result.task_name = task.name.clone();

        for item in &list {
            // Inject loop variable into context.
            ctx.register_var(loop_var.to_string(), item.clone());

            let resolver = VariableResolver::new(ctx);
            let vars = resolver.merged(host);
            let result = self.run_single(task, host, ctx, &vars)?;

            if result.status.is_failed() && !task.ignore_errors {
                return Ok(result);
            }
            last_result = result;
        }

        // Clean up loop variable.
        ctx.registered_vars.remove(loop_var);
        Ok(last_result)
    }

    /// Execute the module once (no loop).
    fn run_single(
        &self,
        task: &Task,
        host: &str,
        ctx: &mut ExecutionContext,
        vars: &HashMap<String, Value>,
    ) -> Result<TaskResult> {
        let args = build_module_args(task, vars, self.trust_level)?;
        let mut result = self.registry.invoke(&task.module, &args, host, ctx)?;
        result.task_name = task.name.clone();

        // Apply changed_when / failed_when overrides.
        if let Some(expr) = &task.changed_when {
            let rendered = render_when(expr, vars, self.trust_level);
            result.changed = evaluate_bool(&rendered);
            if result.changed && result.status.is_ok() {
                result.status = ansiblers_core::TaskStatus::Changed;
            } else if !result.changed {
                result.status = ansiblers_core::TaskStatus::Ok;
            }
        }

        if let Some(expr) = &task.failed_when {
            let rendered = render_when(expr, vars, self.trust_level);
            if evaluate_bool(&rendered) {
                result.status = ansiblers_core::TaskStatus::Failed;
            }
        }

        // Honour ignore_errors.
        if result.status.is_failed() && task.ignore_errors {
            warn!(task = ?task.name, host, "ignoring error");
            result.status = ansiblers_core::TaskStatus::Ok;
        }

        Ok(result)
    }

    /// Post-task: register result variable, trigger notify.
    fn post_task(
        &self,
        task: &Task,
        _host: &str,
        ctx: &mut ExecutionContext,
        result: &TaskResult,
    ) -> Result<()> {
        if let Some(var_name) = &task.register {
            ctx.register_var(var_name.clone(), result.as_register_value());
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build [`ModuleArgs`] from a task, rendering template strings in values.
fn build_module_args(
    task: &Task,
    vars: &HashMap<String, Value>,
    trust: TrustLevel,
) -> Result<ModuleArgs> {
    let engine = AnsibleTemplateEngine::with_config(TemplateEngineConfig {
        trust_level: trust,
        strict_undefined: trust == TrustLevel::Untrusted,
        python_callable_warnings: false, // keep hot path fast
        allowed_read_paths: vec![],
    });

    let raw_dict = task.args.as_dict();
    let mut rendered: HashMap<String, Value> = HashMap::new();
    for (k, v) in raw_dict {
        let rv = engine.render_value(&v, vars)?;
        rendered.insert(k, rv);
    }

    // For free-form tasks, render the raw params string too.
    if let TaskArgs::FreeForm(s) = &task.args {
        let rendered_s = engine.render(s, vars)?;
        rendered.insert("_raw_params".to_string(), Value::String(rendered_s));
    }

    Ok(ModuleArgs {
        args: rendered,
        task_name: task.name.clone(),
    })
}

/// Render a `when:` condition expression through the template engine.
fn render_when(expr: &str, vars: &HashMap<String, Value>, trust: TrustLevel) -> String {
    let engine = AnsibleTemplateEngine::with_config(TemplateEngineConfig {
        trust_level: trust,
        strict_undefined: false, // `when:` conditions may test for undefined
        python_callable_warnings: false,
        allowed_read_paths: vec![],
    });
    // Wrap bare expressions in `{{ }}` if they don't look like a template.
    let template = if expr.contains("{{") || expr.contains("{%") {
        expr.to_string()
    } else {
        format!("{{{{ {expr} }}}}")
    };
    engine
        .render(&template, vars)
        .unwrap_or_else(|_| expr.to_string())
}

/// Interpret a rendered string as a boolean.
///
/// Truthy: "true", "True", "yes", "1", non-empty string that is not a known falsy.
fn evaluate_bool(s: &str) -> bool {
    match s.trim().to_lowercase().as_str() {
        "true" | "yes" | "1" => true,
        "false" | "no" | "0" | "" => false,
        _ => !s.trim().is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ansiblers_modules::ModuleRegistry;

    #[test]
    fn test_evaluate_bool() {
        assert!(evaluate_bool("true"));
        assert!(evaluate_bool("True"));
        assert!(evaluate_bool("yes"));
        assert!(!evaluate_bool("false"));
        assert!(!evaluate_bool("no"));
        assert!(!evaluate_bool(""));
    }

    #[test]
    fn test_task_executor_shell() {
        use ansiblers_core::{Inventory, TaskStatus};
        use ansiblers_parser::{Task, TaskArgs};
        use std::sync::Arc;

        let registry = ModuleRegistry::with_defaults();
        let executor = TaskExecutor::new(&registry);
        let mut ctx = ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new());

        let task = Task::new("shell", TaskArgs::FreeForm("echo test_exec".to_string()));
        let result = executor.run(&task, "localhost", &mut ctx).unwrap();
        assert_eq!(result.stdout.trim(), "test_exec");
        assert!(matches!(result.status, TaskStatus::Changed));
    }

    #[test]
    fn test_task_executor_register() {
        use ansiblers_core::{Inventory, Value};
        use ansiblers_parser::{Task, TaskArgs};
        use std::sync::Arc;

        let registry = ModuleRegistry::with_defaults();
        let executor = TaskExecutor::new(&registry);
        let mut ctx = ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new());

        let mut task = Task::new("shell", TaskArgs::FreeForm("echo registered".to_string()));
        task.register = Some("my_result".to_string());
        executor.run(&task, "localhost", &mut ctx).unwrap();

        let reg = ctx.get_var("my_result").unwrap();
        assert_eq!(reg["stdout"].as_str().unwrap().trim(), "registered");
    }

    #[test]
    fn test_task_executor_when_skip() {
        use ansiblers_core::Inventory;
        use ansiblers_parser::{Task, TaskArgs, WhenExpr};
        use std::sync::Arc;

        let registry = ModuleRegistry::with_defaults();
        let executor = TaskExecutor::new(&registry);
        let mut ctx = ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new());

        let mut task = Task::new("shell", TaskArgs::FreeForm("echo not_run".to_string()));
        task.when = Some(WhenExpr::Single("false".to_string()));
        let result = executor.run(&task, "localhost", &mut ctx).unwrap();
        assert!(matches!(result.status, ansiblers_core::TaskStatus::Skipped));
    }
}
