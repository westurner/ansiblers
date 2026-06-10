//! `find` module — recursively search a directory tree for files matching
//! given criteria (age, size, name pattern).
//!
//! Returns `files` (list of matching absolute paths) as a registered variable.
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `paths` / `path` | — | Root directory or list of root dirs to search |
//! | `patterns` / `pattern` | `*` | Shell glob pattern(s) to match filenames |
//! | `file_type` | `file` | `file`, `directory`, `link`, `any` |
//! | `recurse` | `false` | Descend into sub-directories |
//! | `age` | — | Only match files older than N seconds (prefix `-` for newer) |
//! | `size` | — | Only match files larger than N bytes (prefix `-` for smaller) |
//! | `hidden` | `false` | Include dot-files |
//! | `excludes` | — | Glob patterns of paths to skip |

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::Result;

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct FindModule;

impl ModuleInvoker for FindModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        // Collect root paths.
        let roots = collect_strings(args, &["paths", "path"]);
        if roots.is_empty() {
            return Ok(TaskResult::failed(
                host,
                "find: 'paths' is required".to_string(),
            ));
        }

        let patterns = collect_strings(args, &["patterns", "pattern"]);
        let patterns: Vec<&str> = if patterns.is_empty() {
            vec!["*"]
        } else {
            patterns.iter().map(|s| s.as_str()).collect()
        };

        let file_type = args.get_str("file_type").unwrap_or("file");
        let recurse = bool_arg(args, "recurse", false);
        let hidden = bool_arg(args, "hidden", false);
        let age_secs = parse_age(args.get_str("age"));
        let size_bytes = parse_size(args.get_str("size"));
        let excludes = collect_strings(args, &["excludes", "exclude"]);

        let mut found: Vec<PathBuf> = Vec::new();

        for root in &roots {
            let root_path = Path::new(root);
            if !root_path.is_dir() {
                continue;
            }
            find_in(
                root_path, &patterns, file_type, recurse, hidden, age_secs, size_bytes, &excludes,
                &mut found,
            )?;
        }

        found.sort();

        let file_list: Vec<Value> = found
            .iter()
            .map(|p| Value::String(p.to_string_lossy().into_owned()))
            .collect();

        let mut result = TaskResult::ok(host);
        result
            .vars
            .insert("files".to_string(), Value::Array(file_list));
        result.msg = format!("{} file(s) found", found.len());
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Recursive find implementation
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn find_in(
    dir: &Path,
    patterns: &[&str],
    file_type: &str,
    recurse: bool,
    hidden: bool,
    age: Option<(bool, u64)>,
    size: Option<(bool, u64)>,
    excludes: &[String],
    found: &mut Vec<PathBuf>,
) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        // Hidden filter.
        if !hidden && name.starts_with('.') {
            continue;
        }

        // Excludes.
        if excludes.iter().any(|ex| glob_match(ex, &name)) {
            continue;
        }

        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        if meta.is_dir() {
            if recurse {
                find_in(
                    &path, patterns, file_type, recurse, hidden, age, size, excludes, found,
                )?;
            }
            if !matches!(file_type, "directory" | "any") {
                continue;
            }
        } else if meta.is_symlink() || path.is_symlink() {
            if !matches!(file_type, "link" | "any") {
                continue;
            }
        } else if meta.is_file() {
            if !matches!(file_type, "file" | "any") {
                continue;
            }
        }

        // Pattern match on filename.
        if !patterns.iter().any(|p| glob_match(p, &name)) {
            continue;
        }

        // Age filter.
        if let Some((older, secs)) = age {
            if let Ok(modified) = meta.modified() {
                if let Ok(age_of_file) = SystemTime::now().duration_since(modified) {
                    let file_age = age_of_file.as_secs();
                    // older=true → file_age > secs; older=false (newer) → file_age < secs
                    if older && file_age <= secs {
                        continue;
                    }
                    if !older && file_age >= secs {
                        continue;
                    }
                }
            }
        }

        // Size filter.
        if let Some((larger, bytes)) = size {
            let file_size = meta.len();
            if larger && file_size <= bytes {
                continue;
            }
            if !larger && file_size >= bytes {
                continue;
            }
        }

        found.push(path);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tiny glob matcher (supports `*` and `?` only — no path separators).
// ---------------------------------------------------------------------------

fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    glob_rec(&p, &t)
}

fn glob_rec(p: &[char], t: &[char]) -> bool {
    match (p.first(), t.first()) {
        (None, None) => true,
        (Some(&'*'), _) => {
            // * matches zero or more characters.
            glob_rec(&p[1..], t) || (!t.is_empty() && glob_rec(p, &t[1..]))
        }
        (Some(&'?'), Some(_)) => glob_rec(&p[1..], &t[1..]),
        (Some(pc), Some(tc)) if pc == tc => glob_rec(&p[1..], &t[1..]),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Argument parsers
// ---------------------------------------------------------------------------

fn collect_strings(args: &ModuleArgs, keys: &[&str]) -> Vec<String> {
    for key in keys {
        if let Some(val) = args.args.get(*key) {
            return match val {
                Value::String(s) => vec![s.clone()],
                Value::Array(seq) => seq
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect(),
                _ => vec![],
            };
        }
    }
    vec![]
}

fn bool_arg(args: &ModuleArgs, key: &str, default: bool) -> bool {
    args.args
        .get(key)
        .and_then(|v| v.as_bool())
        .unwrap_or(default)
}

/// Parse an age string like `"3600"`, `"-3600"` (negative = newer than).
/// Returns `Some((older_than, seconds))`.
fn parse_age(s: Option<&str>) -> Option<(bool, u64)> {
    let s = s?;
    if let Some(rest) = s.strip_prefix('-') {
        let secs: u64 = rest.trim().parse().ok()?;
        Some((false, secs)) // newer than
    } else {
        let secs: u64 = s.trim().parse().ok()?;
        Some((true, secs)) // older than
    }
}

/// Parse a size string like `"1024"`, `"-1024"` (negative = smaller than).
/// Returns `Some((larger_than, bytes))`.
fn parse_size(s: Option<&str>) -> Option<(bool, u64)> {
    let s = s?;
    if let Some(rest) = s.strip_prefix('-') {
        let bytes: u64 = parse_size_value(rest)?;
        Some((false, bytes))
    } else {
        let bytes: u64 = parse_size_value(s)?;
        Some((true, bytes))
    }
}

fn parse_size_value(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(rest) = s.strip_suffix('k').or_else(|| s.strip_suffix('K')) {
        return rest.parse::<u64>().ok().map(|n| n * 1024);
    }
    if let Some(rest) = s.strip_suffix('m').or_else(|| s.strip_suffix('M')) {
        return rest.parse::<u64>().ok().map(|n| n * 1024 * 1024);
    }
    if let Some(rest) = s.strip_suffix('g').or_else(|| s.strip_suffix('G')) {
        return rest.parse::<u64>().ok().map(|n| n * 1024 * 1024 * 1024);
    }
    s.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn make_args(pairs: &[(&str, Value)]) -> ModuleArgs {
        let mut m = HashMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.clone());
        }
        ModuleArgs::new(m)
    }

    #[test]
    fn test_glob_match_star() {
        assert!(glob_match("*.rs", "main.rs"));
        assert!(!glob_match("*.rs", "main.txt"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("foo*", "foobar"));
    }

    #[test]
    fn test_glob_match_question() {
        assert!(glob_match("?.rs", "a.rs"));
        assert!(!glob_match("?.rs", "ab.rs"));
    }

    #[test]
    fn test_parse_age_positive() {
        assert_eq!(parse_age(Some("3600")), Some((true, 3600)));
    }

    #[test]
    fn test_parse_age_negative() {
        assert_eq!(parse_age(Some("-3600")), Some((false, 3600)));
    }

    #[test]
    fn test_parse_size_bytes() {
        assert_eq!(parse_size(Some("1024")), Some((true, 1024)));
    }

    #[test]
    fn test_parse_size_kilo() {
        assert_eq!(parse_size(Some("4k")), Some((true, 4096)));
    }

    #[test]
    fn test_parse_size_mega() {
        assert_eq!(parse_size(Some("2m")), Some((true, 2 * 1024 * 1024)));
    }

    #[test]
    fn test_parse_size_smaller() {
        assert_eq!(parse_size(Some("-512")), Some((false, 512)));
    }

    #[test]
    fn test_find_basic() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("hello.txt"), "hi").unwrap();
        std::fs::write(tmp.path().join("world.rs"), "fn main() {}").unwrap();

        let mut ctx = ansiblers_core::ExecutionContext::new(
            std::sync::Arc::new(ansiblers_core::Inventory::default()),
            std::collections::HashMap::new(),
        );
        let args = make_args(&[
            (
                "paths",
                Value::String(tmp.path().to_string_lossy().into_owned()),
            ),
            ("patterns", Value::String("*.txt".into())),
        ]);
        let result = FindModule.invoke(&args, "localhost", &mut ctx).unwrap();
        assert!(result.status.is_ok());
        let files = result.vars.get("files").unwrap();
        if let Value::Array(seq) = files {
            assert_eq!(seq.len(), 1);
            assert!(seq[0].as_str().unwrap().ends_with("hello.txt"));
        } else {
            panic!("expected Array");
        }
    }

    #[test]
    fn test_find_recurse() {
        let tmp = TempDir::new().unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("deep.txt"), "deep").unwrap();

        let mut ctx = ansiblers_core::ExecutionContext::new(
            std::sync::Arc::new(ansiblers_core::Inventory::default()),
            std::collections::HashMap::new(),
        );
        let args = make_args(&[
            (
                "paths",
                Value::String(tmp.path().to_string_lossy().into_owned()),
            ),
            ("patterns", Value::String("*.txt".into())),
            ("recurse", Value::Bool(true)),
        ]);
        let result = FindModule.invoke(&args, "localhost", &mut ctx).unwrap();
        assert!(result.status.is_ok());
        if let Some(Value::Array(seq)) = result.vars.get("files") {
            assert_eq!(seq.len(), 1);
        }
    }

    #[test]
    fn test_find_no_paths_fails() {
        let mut ctx = ansiblers_core::ExecutionContext::new(
            std::sync::Arc::new(ansiblers_core::Inventory::default()),
            std::collections::HashMap::new(),
        );
        let args = ModuleArgs::new(HashMap::new());
        let result = FindModule.invoke(&args, "localhost", &mut ctx).unwrap();
        assert!(result.status.is_failed());
    }
}
