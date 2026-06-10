use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use ansiblers_core::Value;
use serde_yaml::Value as YamlValue;

use crate::ast::{Block, Handler, Play, Playbook, Task, TaskArgs, TaskNode, WhenExpr};

// ---------------------------------------------------------------------------
// Known task-level directive keys (not module names).
// ---------------------------------------------------------------------------

const TASK_DIRECTIVES: &[&str] = &[
    "name",
    "when",
    "register",
    "loop",
    "with_items",
    "with_list",
    "with_together",
    "loop_control",
    "notify",
    "tags",
    "become",
    "become_user",
    "become_method",
    "ignore_errors",
    "failed_when",
    "changed_when",
    "no_log",
    "delegate_to",
    "delegate_facts",
    "run_once",
    "any_errors_fatal",
    "environment",
    "vars",
    "block",
    "rescue",
    "always",
    "listen",
    "check_mode",
    "diff",
    "timeout",
    "debugger",
    "collections",
    "module_defaults",
];

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Parse a playbook from a filesystem path.
pub fn parse_playbook(path: &str) -> Result<Playbook> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading playbook '{path}'"))?;
    parse_playbook_str(&content, Some(PathBuf::from(path)))
}

/// Parse a playbook from an in-memory YAML string.
pub fn parse_playbook_str(content: &str, path: Option<PathBuf>) -> Result<Playbook> {
    let raw: Vec<YamlValue> =
        serde_yaml::from_str(content).context("parsing playbook YAML")?;
    let plays = raw
        .iter()
        .enumerate()
        .map(|(i, v)| parse_play(v).with_context(|| format!("play #{i}")))
        .collect::<Result<Vec<_>>>()?;
    Ok(Playbook { plays, path })
}

// ---------------------------------------------------------------------------
// Play parsing
// ---------------------------------------------------------------------------

fn parse_play(val: &YamlValue) -> Result<Play> {
    let map = val
        .as_mapping()
        .ok_or_else(|| anyhow!("expected a mapping for play"))?;

    let name = get_str(map, "name").map(str::to_string);
    let hosts = get_str(map, "hosts")
        .ok_or_else(|| anyhow!("play missing required 'hosts' key"))?
        .to_string();

    let tasks = parse_task_list(map, "tasks")?;
    let handlers = parse_handler_list(map)?;
    let vars = parse_vars(map, "vars")?;
    let vars_files = parse_string_list(map, "vars_files");
    let r#become = get_bool(map, "become").unwrap_or(false);
    let become_user = get_str(map, "become_user").map(str::to_string);
    let gather_facts = get_bool(map, "gather_facts").unwrap_or(true);
    let tags = parse_string_list(map, "tags");
    let any_errors_fatal = get_bool(map, "any_errors_fatal").unwrap_or(false);

    Ok(Play {
        name,
        hosts,
        tasks,
        handlers,
        vars,
        vars_files,
        r#become,
        become_user,
        gather_facts,
        tags,
        any_errors_fatal,
    })
}

// ---------------------------------------------------------------------------
// Task list parsing
// ---------------------------------------------------------------------------

fn parse_task_list(map: &serde_yaml::Mapping, key: &str) -> Result<Vec<TaskNode>> {
    let Some(val) = map.get(key) else {
        return Ok(Vec::new());
    };
    let seq = val
        .as_sequence()
        .ok_or_else(|| anyhow!("'{key}' must be a list"))?;
    seq.iter()
        .enumerate()
        .map(|(i, v)| parse_task_node(v).with_context(|| format!("{key}[{i}]")))
        .collect()
}

fn parse_handler_list(map: &serde_yaml::Mapping) -> Result<Vec<Handler>> {
    let Some(val) = map.get("handlers") else {
        return Ok(Vec::new());
    };
    let seq = val
        .as_sequence()
        .ok_or_else(|| anyhow!("'handlers' must be a list"))?;
    seq.iter()
        .enumerate()
        .map(|(i, v)| parse_task(v).with_context(|| format!("handler[{i}]")))
        .collect()
}

fn parse_task_node(val: &YamlValue) -> Result<TaskNode> {
    let map = val
        .as_mapping()
        .ok_or_else(|| anyhow!("task must be a mapping"))?;

    // If it has a `block:` key it is a block node.
    if map.contains_key("block") {
        return Ok(TaskNode::Block(parse_block(val)?));
    }
    Ok(TaskNode::Task(parse_task(val)?))
}

// ---------------------------------------------------------------------------
// Block parsing
// ---------------------------------------------------------------------------

fn parse_block(val: &YamlValue) -> Result<Block> {
    let map = val
        .as_mapping()
        .ok_or_else(|| anyhow!("block must be a mapping"))?;

    let name = get_str(map, "name").map(str::to_string);
    let block = parse_task_list(map, "block")?;
    let rescue = parse_task_list(map, "rescue")?;
    let always = parse_task_list(map, "always")?;
    let when = parse_when(map);
    let tags = parse_string_list(map, "tags");

    Ok(Block {
        name,
        block,
        rescue,
        always,
        when,
        tags,
    })
}

// ---------------------------------------------------------------------------
// Individual task parsing
// ---------------------------------------------------------------------------

fn parse_task(val: &YamlValue) -> Result<Task> {
    let map = val
        .as_mapping()
        .ok_or_else(|| anyhow!("task must be a mapping"))?;

    // Common directives
    let name = get_str(map, "name").map(str::to_string);
    let when = parse_when(map);
    let register = get_str(map, "register").map(str::to_string);
    let notify = parse_string_list(map, "notify");
    let tags = parse_string_list(map, "tags");
    let r#become = get_bool(map, "become");
    let become_user = get_str(map, "become_user").map(str::to_string);
    let ignore_errors = get_bool(map, "ignore_errors").unwrap_or(false);
    let failed_when = get_str(map, "failed_when").map(str::to_string);
    let changed_when = get_str(map, "changed_when").map(str::to_string);
    let no_log = get_bool(map, "no_log").unwrap_or(false);
    let delegate_to = get_str(map, "delegate_to").map(str::to_string);

    // Loop — `loop:` (new) or `with_items:` (legacy)
    let loop_items = map
        .get("loop")
        .or_else(|| map.get("with_items"))
        .or_else(|| map.get("with_list"))
        .map(yaml_to_json)
        .transpose()?;

    // loop_control.loop_var
    let loop_var = map
        .get("loop_control")
        .and_then(|lc| lc.as_mapping())
        .and_then(|lc| lc.get("loop_var"))
        .and_then(|v| v.as_str())
        .unwrap_or("item")
        .to_string();

    // Find the module name: the first key that is not a known directive.
    let (module, args) = identify_module(map)?;

    Ok(Task {
        name,
        module,
        args,
        when,
        register,
        loop_items,
        loop_var,
        notify,
        tags,
        r#become,
        become_user,
        ignore_errors,
        failed_when,
        changed_when,
        no_log,
        delegate_to,
    })
}

/// Find the module name key and extract its arguments from the task mapping.
fn identify_module(map: &serde_yaml::Mapping) -> Result<(String, TaskArgs)> {
    for (k, v) in map.iter() {
        let key = k.as_str().unwrap_or("");
        if TASK_DIRECTIVES.contains(&key) {
            continue;
        }
        // This key is the module name.
        let module = key.to_string();
        let args = match v {
            // `shell: echo hello` — scalar value → free-form
            YamlValue::String(s) => TaskArgs::FreeForm(s.clone()),
            YamlValue::Null => TaskArgs::Dict(HashMap::new()),
            // `debug:\n  msg: "..."` — mapping → dict
            YamlValue::Mapping(m) => {
                let dict = m
                    .iter()
                    .map(|(mk, mv)| {
                        let key = mk
                            .as_str()
                            .ok_or_else(|| anyhow!("non-string module arg key"))?
                            .to_string();
                        let val = yaml_to_json(mv)?;
                        Ok((key, val))
                    })
                    .collect::<Result<HashMap<_, _>>>()?;
                TaskArgs::Dict(dict)
            }
            other => {
                let json_val = yaml_to_json(other)?;
                TaskArgs::FreeForm(json_val.to_string())
            }
        };
        return Ok((module, args));
    }
    Err(anyhow!("no module key found in task: {:?}", map))
}

// ---------------------------------------------------------------------------
// Helper utilities
// ---------------------------------------------------------------------------

fn parse_when(map: &serde_yaml::Mapping) -> Option<WhenExpr> {
    let v = map.get("when")?;
    match v {
        YamlValue::String(s) => Some(WhenExpr::Single(s.clone())),
        YamlValue::Sequence(seq) => {
            let items: Vec<String> = seq
                .iter()
                .filter_map(|i| i.as_str().map(str::to_string))
                .collect();
            Some(WhenExpr::List(items))
        }
        _ => None,
    }
}

fn parse_vars(
    map: &serde_yaml::Mapping,
    key: &str,
) -> Result<HashMap<String, Value>> {
    let Some(val) = map.get(key) else {
        return Ok(HashMap::new());
    };
    let m = val
        .as_mapping()
        .ok_or_else(|| anyhow!("'{key}' must be a mapping"))?;
    m.iter()
        .map(|(k, v)| {
            let key = k
                .as_str()
                .ok_or_else(|| anyhow!("non-string var key"))?
                .to_string();
            let val = yaml_to_json(v)?;
            Ok((key, val))
        })
        .collect()
}

fn parse_string_list(map: &serde_yaml::Mapping, key: &str) -> Vec<String> {
    let Some(val) = map.get(key) else {
        return Vec::new();
    };
    match val {
        YamlValue::String(s) => vec![s.clone()],
        YamlValue::Sequence(seq) => seq
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

fn get_str<'a>(map: &'a serde_yaml::Mapping, key: &str) -> Option<&'a str> {
    map.get(key).and_then(|v| v.as_str())
}

fn get_bool(map: &serde_yaml::Mapping, key: &str) -> Option<bool> {
    map.get(key).and_then(|v| v.as_bool())
}

/// Convert a serde_yaml::Value to serde_json::Value.
pub(crate) fn yaml_to_json(val: &YamlValue) -> Result<Value> {
    let json_str = serde_json::to_string(&serde_yaml::from_value::<serde_json::Value>(
        val.clone(),
    )?)
    .context("serialising yaml value to json")?;
    serde_json::from_str(&json_str).context("deserialising json value")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    const SIMPLE_PLAYBOOK: &str = r#"
- name: Simple play
  hosts: all
  vars:
    greeting: Hello
  tasks:
    - name: Say hello
      shell: echo "{{ greeting }}"
      register: result
    - name: Debug output
      debug:
        msg: "{{ result.stdout }}"
"#;

    const BLOCK_PLAYBOOK: &str = r#"
- hosts: all
  tasks:
    - block:
        - name: Try something
          shell: /bin/false
      rescue:
        - name: Handle failure
          debug:
            msg: "rescued"
      always:
        - name: Cleanup
          debug:
            msg: "always runs"
"#;

    #[test]
    fn test_parse_simple_playbook() {
        let pb = parse_playbook_str(SIMPLE_PLAYBOOK, None).unwrap();
        assert_eq!(pb.plays.len(), 1);
        let play = &pb.plays[0];
        assert_eq!(play.hosts, "all");
        assert_eq!(play.tasks.len(), 2);
    }

    #[test]
    fn test_parse_vars() {
        let pb = parse_playbook_str(SIMPLE_PLAYBOOK, None).unwrap();
        let play = &pb.plays[0];
        assert_eq!(
            play.vars.get("greeting"),
            Some(&Value::String("Hello".to_string()))
        );
    }

    #[test]
    fn test_parse_register() {
        let pb = parse_playbook_str(SIMPLE_PLAYBOOK, None).unwrap();
        let play = &pb.plays[0];
        if let TaskNode::Task(task) = &play.tasks[0] {
            assert_eq!(task.register.as_deref(), Some("result"));
            assert_eq!(task.module, "shell");
        } else {
            panic!("expected task");
        }
    }

    #[test]
    fn test_parse_block() {
        let pb = parse_playbook_str(BLOCK_PLAYBOOK, None).unwrap();
        let play = &pb.plays[0];
        if let TaskNode::Block(block) = &play.tasks[0] {
            assert_eq!(block.block.len(), 1);
            assert_eq!(block.rescue.len(), 1);
            assert_eq!(block.always.len(), 1);
        } else {
            panic!("expected block");
        }
    }

    #[rstest]
    #[case("shell", "echo hello")]
    #[case("command", "/bin/true")]
    fn test_free_form_module(#[case] module: &str, #[case] cmd: &str) {
        let yaml = format!(
            "- hosts: all\n  tasks:\n    - {module}: {cmd}\n"
        );
        let pb = parse_playbook_str(&yaml, None).unwrap();
        if let TaskNode::Task(task) = &pb.plays[0].tasks[0] {
            assert_eq!(task.module, module);
            assert_eq!(task.args.as_free_form(), Some(cmd));
        } else {
            panic!("expected task");
        }
    }

    #[test]
    fn test_snapshot_simple_playbook_ast() {
        let pb = parse_playbook_str(SIMPLE_PLAYBOOK, None).unwrap();
        insta::assert_json_snapshot!(pb);
    }
}
