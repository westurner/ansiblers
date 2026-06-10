# Migration Guide: ansible-playbook → ransible-playbook

This guide helps teams switch from the standard Python `ansible-playbook` binary
to the Rust `ransible-playbook` drop-in replacement.

---

## Installation

### From source (Cargo)

```bash
cd src/ansiblers
cargo build --release
# Binary at: target/release/ransible-playbook

# Optional: install system-wide
cargo install --path crates/ansiblers-playbook
```

### PATH alias (non-destructive)

Place `ransible-playbook` earlier in `$PATH` than `ansible-playbook`, or add a
shell alias during evaluation:

```bash
# ~/.bashrc or ~/.zshrc
alias ansible-playbook=ransible-playbook
```

Prefer the alias approach during evaluation so you can switch back instantly.

---

## CLI Compatibility

`ransible-playbook` accepts the same flags as `ansible-playbook`.  The table
below shows the current support status.

| Flag | Status | Notes |
|------|--------|-------|
| `playbook.yml` (positional) | ✅ | One or more playbooks |
| `-i / --inventory` | ✅ | INI, YAML, and dynamic inventory |
| `-e / --extra-vars` | ✅ | `key=value` and JSON/YAML blobs |
| `-l / --limit` | ✅ | Subset of hosts |
| `-v / -vv / -vvv` | ✅ | Verbosity levels |
| `--json` | ✅ | Machine-readable output |
| `-C / --check` | ⚠️ | Accepted but runs normally (no dry-run yet) |
| `--tags / --skip-tags` | ⏳ | Planned |
| `--start-at-task` | ⏳ | Planned |
| `--become / -b` | ⏳ | Planned (SSH provider supports become via inventory vars) |
| `-K / --ask-become-pass` | ⏳ | Planned |
| `--vault-password-file` | ⏳ | Planned |
| `-f / --forks` | ⏳ | Use `strategy: batch_N` in the play instead |

### Exit codes (identical to Ansible)

| Code | Meaning |
|------|---------|
| 0 | All plays and tasks succeeded |
| 1 | Fatal error (parse failure, I/O error, missing inventory) |
| 2 | One or more tasks failed |

---

## Inventory Compatibility

Both INI and YAML inventory formats are supported.

### INI inventory

```ini
[webservers]
web1 ansible_host=10.0.0.1 ansible_user=deploy
web2 ansible_host=10.0.0.2

[databases]
db1 ansible_host=10.0.1.1

[production:children]
webservers
databases

[webservers:vars]
http_port=80
```

### YAML inventory

```yaml
all:
  children:
    webservers:
      hosts:
        web1:
          ansible_host: 10.0.0.1
          ansible_user: deploy
        web2:
          ansible_host: 10.0.0.2
    databases:
      hosts:
        db1:
          ansible_host: 10.0.1.1
```

### group_vars / host_vars

Directory-based variable files are fully supported:

```
inventory/
├── hosts.ini
├── group_vars/
│   ├── all.yml
│   └── webservers.yml
└── host_vars/
    └── web1.yml
```

### Dynamic inventory

Pass a script that outputs JSON in the Ansible `--list` format:

```bash
ransible-playbook -i ./inventory.py playbook.yml
```

---

## Playbook Compatibility

### Fully supported features

- Plays with `hosts:`, `gather_facts:`, `vars:`, `tasks:`, `handlers:`
- Task keywords: `name`, `when`, `register`, `loop` / `with_items`, `notify`,
  `ignore_errors`, `changed_when`, `failed_when`
- `block:` / `rescue:` / `always:` control flow
- Role loading: `roles/` directory with `tasks/`, `defaults/`, `vars/`,
  `handlers/`, `meta/`
- Role dependencies (`meta/main.yml`)
- `include_vars:`, `set_fact:`, `debug:`, `fail:`
- Jinja2 templating (minijinja — ~95% compatible)
- Ansible filters: `combine`, `regex_replace`, `regex_search`, `to_nice_json`,
  `quote`, `from_json`, `from_yaml`, `path_join`, `default`, `bool`, `upper`,
  `lower`, `join`, `split`, `trim`, and more

### Known gaps / deferred features

| Feature | Status |
|---------|--------|
| `include_tasks` / `import_tasks` directives | ⏳ Planned (Phase 3+) |
| `include_role` / `import_role` | ⏳ Planned |
| Vault-encrypted variables | ⏳ Planned |
| `ansible.cfg` file support | ⏳ Planned |
| `--check` (dry-run) mode | ⏳ Planned |
| `--diff` mode | ⏳ Planned |
| Callback plugins | ⏳ Planned |
| Connection plugins (beyond local/ssh) | ⏳ Planned |

If you hit an unsupported feature, `ransible-playbook` will either emit a
warning and skip it (soft incompatibility) or exit with code 1 and a clear
error message (hard incompatibility).

---

## Module Compatibility

### Rust-native modules (recommended — full compatibility)

| Module | Notes |
|--------|-------|
| `shell` | `creates`, `removes`, `chdir`, `stdin` |
| `command` | Same as `shell` without shell expansion |
| `debug` | `msg`, `var` |
| `set_fact` | Sets host facts |
| `fail` | `msg` |
| `file` | `state`, `mode`, `owner`, `group`, `recurse`, `src`, `dest` |
| `copy` | `src`, `dest`, `content`, `mode`, `backup` |
| `stat` | Returns file metadata |
| `find` | Directory traversal with glob, age, size filters |
| `template` | Jinja2 `.j2` → file |
| `lineinfile` | `regexp`, `line`, `insertafter`, `insertbefore`, lookahead/lookbehind |
| `apt` | `name`, `state`, `update_cache`, `autoremove`, `purge` |
| `yum` / `dnf` | `name`, `state`, `enablerepo`, `disablerepo` |
| `setup` / `gather_facts` | `ansible_*` facts from `/proc`, `/sys`, `uname` |
| `git` | `repo`, `dest`, `version`, `depth`, `update`, `ssh_opts` |

### Python fallback (via `PythonModuleWrapper`)

Modules not listed above are transparently executed via the Python Ansible module
wrapper, so existing playbooks using less-common modules continue to work:

```
ransible-playbook: module 'aws_s3' not found in Rust registry; falling back to Python
```

Ensure the target host has `ansible` Python package installed for fallback to
work.

---

## Performance Expectations

Typical improvements over Python `ansible-playbook`:

| Operation | Expected speedup |
|-----------|-----------------|
| Playbook parse + variable resolution | 5–10× |
| Simple `shell` / `command` tasks | 2–4× |
| `stat` / `setup` on many hosts (cached) | 10–50× |
| Large inventory loading | 3–7× |

Speedup varies by workload.  Use the benchmarking guide
([PERFORMANCE_TUNING.md](PERFORMANCE_TUNING.md)) to measure your specific case.

---

## Evaluation Checklist

Use this checklist when piloting `ransible-playbook` on a real project:

```
□ Install ransible-playbook alongside ansible-playbook (don't replace yet)
□ Run: ransible-playbook -i inventory site.yml --json > ransible-out.json
□ Run: ansible-playbook -i inventory site.yml --json > ansible-out.json
□ Compare outputs with: diff ansible-out.json ransible-out.json
□ Verify exit codes match
□ Check no tasks were silently skipped (count changed/ok/failed lines)
□ Test with --limit to validate a single host first
□ Run the benchmark suite to quantify speedup
□ Replace alias or PATH entry once outputs match
```

---

## Reporting Issues

When opening a bug report, include:

1. The playbook (or a minimal reproducer)
2. The inventory (anonymised)
3. `ransible-playbook --version`
4. Output with `-vvv`
5. Expected vs actual behaviour

See the [Troubleshooting Guide](TROUBLESHOOTING.md) for common issues.
