//! `lineinfile` module — ensure a line is present/absent in a text file.
//!
//! Mirrors the behaviour of Ansible's `lineinfile` module.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `path` / `dest` / `name` | — | File path to modify |
//! | `line` | — | The exact line to insert/replace (required unless `state=absent`) |
//! | `regexp` | — | Regex; if it matches an existing line that line is replaced |
//! | `state` | `present` | `present` (ensure line exists) or `absent` (ensure it is removed) |
//! | `insertafter` | `EOF` | `EOF`, `BOF`, or a regex; insert after the last matching line |
//! | `insertbefore` | — | Insert before the first matching line (or `BOF`) |
//! | `backrefs` | `false` | When `true` and `regexp` matches, use back-references in `line` |
//! | `create` | `false` | Create the file if it does not exist |
//! | `backup` | `false` | Create a `.bak` backup before modifying |

use std::path::Path;

use ansiblers_core::{ExecutionContext, TaskResult};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct LineinfileModule;

impl ModuleInvoker for LineinfileModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let path = args
            .get_str("path")
            .or_else(|| args.get_str("dest"))
            .or_else(|| args.get_str("name"))
            .ok_or_else(|| anyhow::anyhow!("lineinfile: 'path' is required"))?
            .to_string();

        let state = args.get_str("state").unwrap_or("present");
        let line = args.get_str("line").map(|s| s.to_string());
        let regexp = args.get_str("regexp").map(|s| s.to_string());
        let insertafter = args.get_str("insertafter").map(|s| s.to_string());
        let insertbefore = args.get_str("insertbefore").map(|s| s.to_string());
        let backrefs = bool_arg(args, "backrefs", false);
        let create = bool_arg(args, "create", false);
        let backup = bool_arg(args, "backup", false);

        let file_path = Path::new(&path);

        // Create file if needed.
        if !file_path.exists() {
            if create {
                if let Some(parent) = file_path.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent)?;
                    }
                }
                std::fs::write(file_path, "")?;
            } else {
                return Ok(TaskResult::failed(
                    host,
                    format!("lineinfile: file '{path}' does not exist (use create=true)"),
                ));
            }
        }

        let content =
            std::fs::read_to_string(file_path).with_context(|| format!("cannot read '{path}'"))?;
        let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        // Preserve trailing newline flag.
        let had_trailing_newline = content.ends_with('\n');

        let changed = match state {
            "absent" => remove_lines(&mut lines, regexp.as_deref(), line.as_deref()),
            _ => {
                // present
                let new_line =
                    line.as_deref().ok_or_else(|| anyhow::anyhow!("lineinfile: 'line' is required for state=present"))?;
                insert_or_replace(
                    &mut lines,
                    new_line,
                    regexp.as_deref(),
                    insertafter.as_deref(),
                    insertbefore.as_deref(),
                    backrefs,
                )
            }
        };

        if !changed {
            return Ok(TaskResult::ok(host));
        }

        // Backup.
        if backup {
            std::fs::copy(file_path, format!("{path}.bak"))?;
        }

        let mut new_content = lines.join("\n");
        if had_trailing_newline || !lines.is_empty() {
            new_content.push('\n');
        }

        std::fs::write(file_path, &new_content)
            .with_context(|| format!("cannot write '{path}'"))?;

        Ok(TaskResult::changed(host))
    }
}

// ---------------------------------------------------------------------------
// Core line manipulation logic
// ---------------------------------------------------------------------------

/// Remove all lines matching `regexp` or equal to `line`.  Returns whether
/// the file content actually changed.
fn remove_lines(lines: &mut Vec<String>, regexp: Option<&str>, line: Option<&str>) -> bool {
    let before_len = lines.len();
    lines.retain(|l| {
        if let Some(re) = regexp {
            if regex_matches(re, l) {
                return false;
            }
        }
        if let Some(exact) = line {
            if l == exact {
                return false;
            }
        }
        true
    });
    lines.len() != before_len
}

/// Insert or replace a line.  Returns whether a change was made.
fn insert_or_replace(
    lines: &mut Vec<String>,
    new_line: &str,
    regexp: Option<&str>,
    insertafter: Option<&str>,
    insertbefore: Option<&str>,
    backrefs: bool,
) -> bool {
    // If regexp given, try to find a matching line and replace it.
    if let Some(re) = regexp {
        let mut match_idx: Option<usize> = None;
        for (i, l) in lines.iter().enumerate() {
            if regex_matches(re, l) {
                match_idx = Some(i);
            }
        }
        if let Some(idx) = match_idx {
            let replacement = if backrefs {
                apply_backrefs(re, &lines[idx].clone(), new_line)
            } else {
                new_line.to_string()
            };
            if lines[idx] == replacement {
                return false;
            }
            lines[idx] = replacement;
            return true;
        }
        // No match: if backrefs is true we skip insertion (Ansible semantics).
        if backrefs {
            return false;
        }
    }

    // Check if the exact line already exists (idempotent).
    if lines.iter().any(|l| l == new_line) {
        return false;
    }

    // Determine insertion point.
    if let Some(before) = insertbefore {
        if before == "BOF" {
            lines.insert(0, new_line.to_string());
        } else {
            let idx = lines.iter().position(|l| regex_matches(before, l)).unwrap_or(0);
            lines.insert(idx, new_line.to_string());
        }
        return true;
    }

    if let Some(after) = insertafter {
        if after == "BOF" {
            lines.insert(0, new_line.to_string());
            return true;
        }
        if after != "EOF" {
            // Insert after the last matching line.
            let idx = lines
                .iter()
                .enumerate()
                .filter(|(_, l)| regex_matches(after, l))
                .map(|(i, _)| i)
                .last();
            if let Some(i) = idx {
                lines.insert(i + 1, new_line.to_string());
                return true;
            }
        }
    }

    // Default: append at EOF.
    lines.push(new_line.to_string());
    true
}

// ---------------------------------------------------------------------------
// Regex helpers (uses `fancy-regex` for Python-compatible lookahead/lookbehind
// backreference support)
// ---------------------------------------------------------------------------

fn regex_matches(pattern: &str, text: &str) -> bool {
    fancy_regex::Regex::new(pattern).map(|re| re.is_match(text).unwrap_or(false)).unwrap_or(false)
}

/// Very basic back-reference application: replace `\1` `\2` etc. in
/// `replacement` with the corresponding capture group from `text` matched by
/// `pattern`.
fn apply_backrefs(pattern: &str, text: &str, replacement: &str) -> String {
    let Ok(re) = fancy_regex::Regex::new(pattern) else {
        return replacement.to_string();
    };
    if let Ok(Some(caps)) = re.captures(text) {
        let mut result = replacement.to_string();
        for i in 1..=caps.len().saturating_sub(1) {
            let placeholder = format!("\\{i}");
            let cap_val = caps.get(i).map(|m| m.as_str()).unwrap_or("");
            result = result.replace(&placeholder, cap_val);
        }
        result
    } else {
        replacement.to_string()
    }
}

fn bool_arg(args: &ModuleArgs, key: &str, default: bool) -> bool {
    args.args.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ansiblers_core::{ExecutionContext, Inventory, Value};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn make_args(pairs: &[(&str, Value)]) -> ModuleArgs {
        let mut m = HashMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.clone());
        }
        ModuleArgs::new(m)
    }

    fn ctx() -> ExecutionContext {
        ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new())
    }

    fn tmp_file(content: &str) -> (TempDir, std::path::PathBuf) {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("test.txt");
        std::fs::write(&p, content).unwrap();
        (tmp, p)
    }

    #[test]
    fn test_insert_line_at_eof() {
        let (_tmp, path) = tmp_file("line1\nline2\n");
        let mut c = ctx();
        let args = make_args(&[
            ("path", Value::String(path.to_str().unwrap().to_string())),
            ("line", Value::String("line3".to_string())),
        ]);
        let result = LineinfileModule.invoke(&args, "h", &mut c).unwrap();
        assert!(result.changed);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("line3"));
    }

    #[test]
    fn test_idempotent_line_already_present() {
        let (_tmp, path) = tmp_file("line1\nline2\n");
        let mut c = ctx();
        let args = make_args(&[
            ("path", Value::String(path.to_str().unwrap().to_string())),
            ("line", Value::String("line1".to_string())),
        ]);
        let result = LineinfileModule.invoke(&args, "h", &mut c).unwrap();
        assert!(!result.changed);
    }

    #[test]
    fn test_remove_line_absent() {
        let (_tmp, path) = tmp_file("keep\nremove_me\nkeep2\n");
        let mut c = ctx();
        let args = make_args(&[
            ("path", Value::String(path.to_str().unwrap().to_string())),
            ("line", Value::String("remove_me".to_string())),
            ("state", Value::String("absent".to_string())),
        ]);
        let result = LineinfileModule.invoke(&args, "h", &mut c).unwrap();
        assert!(result.changed);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.contains("remove_me"));
        assert!(content.contains("keep"));
    }

    #[test]
    fn test_regexp_replace() {
        let (_tmp, path) = tmp_file("FOO=old\nBAR=baz\n");
        let mut c = ctx();
        let args = make_args(&[
            ("path", Value::String(path.to_str().unwrap().to_string())),
            ("regexp", Value::String("^FOO=".to_string())),
            ("line", Value::String("FOO=new".to_string())),
        ]);
        let result = LineinfileModule.invoke(&args, "h", &mut c).unwrap();
        assert!(result.changed);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("FOO=new"));
        assert!(!content.contains("FOO=old"));
    }

    #[test]
    fn test_insertafter_pattern() {
        let (_tmp, path) = tmp_file("line1\n[section]\nline3\n");
        let mut c = ctx();
        let args = make_args(&[
            ("path", Value::String(path.to_str().unwrap().to_string())),
            ("line", Value::String("new_setting=1".to_string())),
            ("insertafter", Value::String("\\[section\\]".to_string())),
        ]);
        LineinfileModule.invoke(&args, "h", &mut c).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        let sect_idx = lines.iter().position(|l| *l == "[section]").unwrap();
        assert_eq!(lines[sect_idx + 1], "new_setting=1");
    }

    #[test]
    fn test_create_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("new.conf");
        let mut c = ctx();
        let args = make_args(&[
            ("path", Value::String(path.to_str().unwrap().to_string())),
            ("line", Value::String("created=yes".to_string())),
            ("create", Value::Bool(true)),
        ]);
        let result = LineinfileModule.invoke(&args, "h", &mut c).unwrap();
        assert!(result.changed);
        assert!(path.exists());
    }

    #[test]
    fn test_missing_path_fails() {
        let mut c = ctx();
        let args = make_args(&[("line", Value::String("x".to_string()))]);
        let result = LineinfileModule.invoke(&args, "h", &mut c);
        assert!(result.is_err());
    }

    #[test]
    fn test_backrefs() {
        let result = apply_backrefs(r"^(FOO)=.*", "FOO=old", r"\1=new");
        assert_eq!(result, "FOO=new");
    }
}
