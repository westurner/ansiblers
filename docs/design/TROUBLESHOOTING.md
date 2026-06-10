# Ansiblers Troubleshooting Guide

Common problems and solutions when running `ransible-playbook` or developing
against the `ansiblers-*` crates.

---

## Quick Diagnostics

```bash
# Increase verbosity to see what the executor is doing
ransible-playbook -i inventory playbook.yml -vvv

# Get structured JSON output for programmatic inspection
ransible-playbook -i inventory playbook.yml --json 2>&1 | jq .

# Check which modules are registered
cargo test -p ansiblers-modules -- registry --nocapture 2>&1 | head -20

# Run the test suite to verify your build
cargo test --all -- --nocapture --test-threads=1
```

---

## Runtime Errors

### `Error: inventory file not found`

**Cause**: The path passed to `-i` does not exist.

**Fix**:
```bash
# Verify the path
ls -la /path/to/inventory.ini

# Use an absolute path to avoid CWD confusion
ransible-playbook -i /abs/path/to/inventory.ini playbook.yml
```

---

### `Error: failed to parse playbook: ...`

**Cause**: The YAML in the playbook is invalid or uses a construct that the
parser does not yet support.

**Fix**:
```bash
# Validate YAML syntax with Python
python3 -c "import yaml; yaml.safe_load(open('playbook.yml'))"

# Minimal reproducer — strip tasks until the error goes away:
# - include_tasks / import_tasks are not yet supported; inline the tasks instead
```

Known unsupported constructs:
- `include_tasks:` / `import_tasks:` — inline tasks or use roles instead
- `include_role:` / `import_role:` — add the role to `roles:` in the play

---

### `module 'X' not found in Rust registry; falling back to Python`

**Cause**: The module has no Rust-native implementation yet.

**Fix**: This is informational — Python fallback is used automatically.  If the
fallback also fails, verify that `ansible` is installed on the target:

```bash
python3 -c "import ansible; print(ansible.__version__)"
```

---

### `Error: template rendering failed: ...`

**Cause**: A Jinja2 template uses a filter or feature not yet supported by
`minijinja` (the Rust Jinja2 engine).

**Common unsupported filters**:
| Filter | Workaround |
|--------|-----------|
| `ansible.utils.*` | Not supported — rewrite using built-in filters |
| `to_uuid` | Not supported — use `shell` module with `uuidgen` |
| `ipaddr` | Not supported — use `regex_replace` or Python |

**Fix**: Use the Python PyO3 fallback explicitly (set `USE_PYTHON_TEMPLATES=1`
environment variable) or rewrite the template using supported filters.

---

### `Error: connection refused` / SSH errors

**Cause**: SSH connection to the target host failed.

**Diagnostics**:
```bash
# Test SSH manually
ssh -i ~/.ssh/key deploy@target-host echo ok

# Check ansible_ vars in inventory
ransible-playbook -i inventory playbook.yml -vvv 2>&1 | grep "ansible_host\|ansible_user\|ansible_port"
```

**Common fixes**:
- Set `ansible_host` if the hostname in inventory differs from the reachable IP/name.
- Set `ansible_ssh_private_key_file` for non-default key paths.
- Set `ansible_port` if SSH isn't on port 22.

---

### `Error: task failed on host X: ...` (unexpected failure)

**Cause**: The module returned a non-zero exit code or `failed: true`.

**Fix**:
1. Add `ignore_errors: true` temporarily to see the full error output.
2. Add `-vvv` to see stdout/stderr from the module.
3. Run the equivalent shell command manually on the target host.

```yaml
- name: debug failing task
  shell: your_command_here
  register: result
  ignore_errors: true

- debug:
    var: result
```

---

### `Error: variable 'X' is undefined`

**Cause**: A template references a variable that hasn't been set.

**Common causes**:
- `register:` result used before the task runs (wrong order in play).
- `host_vars` file not found (check path and YAML indentation).
- Fact not gathered — add `gather_facts: true` to the play.

**Fix**:
```yaml
# Use the 'default' filter as a safety net
- debug:
    msg: "{{ my_var | default('not set') }}"

# Check what variables are in scope
- debug:
    var: hostvars[inventory_hostname]
```

---

## Build / Compile Errors

### `error[E0308]: mismatched types` involving `Value::Int`

**Cause**: `ansiblers_core::Value` is `serde_json::Value`, not a custom enum.
There is no `Value::Int` variant.

**Fix**:
```rust
// Wrong
Value::Int(42)

// Correct
Value::Number(serde_json::Number::from(42))

// In match arms
Value::Number(n) => n.as_u64().unwrap_or(0) as usize,
```

---

### `error: cannot use struct expression for TaskResult`

**Cause**: `TaskResult` uses constructor methods, not struct literal syntax.

**Fix**:
```rust
// Wrong
TaskResult { host: "web1".into(), status: TaskStatus::Ok, .. }

// Correct
let mut result = TaskResult::ok("web1");
result.stdout = "output".to_string();
```

Available constructors: `TaskResult::ok`, `TaskResult::failed`,
`TaskResult::changed`, `TaskResult::skipped`.

---

### `error: the trait 'ModuleResultCache' is not implemented for Arc<InMemoryCache>`

**Cause**: `CachingModuleRegistry::new` takes `Arc<dyn ModuleResultCache>`, not
`Arc<InMemoryCache>` directly.

**Fix**: Add an explicit type annotation:
```rust
let cache: Arc<dyn ModuleResultCache> = Arc::new(InMemoryCache::default());
let cached = CachingModuleRegistry::new(inner, cache);
```

---

### `error: edition.workspace is not supported`

**Cause**: `jinja2rs/Cargo.toml` cannot inherit workspace edition because it is
a sibling directory, not a workspace member.

**Fix**: Use an explicit edition in `jinja2rs/Cargo.toml`:
```toml
edition = "2021"   # not edition.workspace = true
```

---

### `error[E0716]: temporary value dropped while borrowed` in benchmarks

**Cause**: Criterion benchmark closures hold references to temporaries that don't
live long enough.

**Fix**: Move the temporary into a `let` binding outside the closure:
```rust
// Wrong
b.iter(|| {
    let result = executor.run_playbook(&parse_playbook("...").unwrap(), &mut ctx);
});

// Correct
let playbook = parse_playbook("...").unwrap();
b.iter(|| executor.run_playbook(&playbook, &mut ctx));
```

---

## Test Failures

### `snapshot mismatch` from `cargo insta`

**Cause**: A code change altered the serialised output of a tested function.

**Fix**:
```bash
# Review the diff
cargo insta review

# Accept all pending changes (after verifying they are correct)
cargo insta accept

# Reject changes and revert
cargo insta reject
```

---

### Tests fail with `No such file or directory: fixtures/...`

**Cause**: Test fixture paths are resolved relative to `CARGO_MANIFEST_DIR` which
is `src/ansiblers/tests/`, not the workspace root.

**Fix**: Always use `CARGO_MANIFEST_DIR` to build fixture paths:
```rust
let fixture_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
```

---

### Coverage below threshold: `FAILED: line coverage 68% < 75%`

**Cause**: New code was added without sufficient tests.

**Fix**:
```bash
# Generate HTML report to find uncovered lines
cargo +nightly llvm-cov --all --html
open target/llvm-cov/html/index.html

# Check per-crate coverage
cargo +nightly llvm-cov -p ansiblers-executor report
```

Coverage targets per component:

| Crate | Line | Branch |
|-------|------|--------|
| `ansiblers-core` | 85% | 75% |
| `ansiblers-executor` | 80% | 70% |
| `ansiblers-parser` | 80% | 60% |
| `ansiblers-vars` | 85% | 75% |
| Minimum (all crates) | 75% | 60% |

---

## Performance Issues

### Playbook is slower than expected

1. Check the strategy — `linear` is serial by default.  Try `strategy: free`.
2. Check if `stat`/`setup` are called multiple times — enable `CachingModuleRegistry`.
3. Profile with `cargo flamegraph --bin ransible-playbook -- -i inv pb.yml`.

### Memory usage grows with large inventories

`InMemoryCache` stores results for the lifetime of the registry.  For very large
inventories, switch to `SqliteCache` which uses an on-disk store:

```rust
let cache = Arc::new(SqliteCache::open("/tmp/ansiblers-cache.db")?);
```

---

## Getting Help

- Open an issue with a minimal reproducer and `-vvv` output.
- See [MIGRATION_GUIDE.md](MIGRATION_GUIDE.md) for Ansible compatibility notes.
- See [PERFORMANCE_TUNING.md](PERFORMANCE_TUNING.md) for tuning advice.
- See [TESTING_STRATEGY.md](TESTING_STRATEGY.md) for how to add or fix tests.
