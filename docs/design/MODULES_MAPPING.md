# Ansiblers Module Mapping & Prioritization

## Module Implementation Strategy

### Phase 1: Foundation (Weeks 1-8)
**Approach**: Subprocess wrapper for Python modules; shell/command in Rust

- ✅ **shell** → Rust native
- ✅ **command** → Rust native
- ➡️ **All others** → Python subprocess wrapper

### Phase 2: High-Value (Weeks 9-14)
**Approach**: Identify performance bottlenecks; rewrite high-impact modules

- Candidates: debug, file, copy, stat, set_fact (already in Python wrapper)

### Phase 3-5: Gradual Migration
**Approach**: Profile real workloads; rewrite modules showing 2x+ speedup potential

---

## Module Categories & Prioritization

### Essential Modules (Phase 1-2)

| Module | Category | Phase | Priority | Rationale | Wrapper Strategy |
|--------|----------|-------|----------|-----------|-------------------|
| command | Execution | 1 | Critical | Core task execution | Rust native |
| shell | Execution | 1 | Critical | Core task execution | Rust native |
| debug | Utility | 2 | High | Common for output | Rust native (simple) |
| set_fact | Variable | 2 | High | Variable registration | Rust native (simple) |
| copy | File | 2 | High | Common file transfer | Python wrapper (Phase 3 rewrite) |
| file | File | 2 | High | File operations | Python wrapper (Phase 3 rewrite) |
| stat | File | 2 | High | File info | Python wrapper (Phase 2 rewrite) |
| include_tasks | Flow | 1 | Critical | Task orchestration | Rust native (executor) |
| import_tasks | Flow | 1 | Critical | Task orchestration | Rust native (executor) |
| set_fact | Variable | 2 | High | Variable registration | Rust native |

### Common Modules (Phase 2-3)

| Module | Category | Phase | Priority | Linux Focus | Windows |
|--------|----------|-------|----------|-------------|---------|
| apt | Package | 3 | High | ✓ Ubuntu/Debian | ✗ |
| yum | Package | 3 | High | ✓ RHEL/CentOS | ✗ |
| dnf | Package | 3 | Medium | ✓ Fedora 22+ | ✗ |
| zypper | Package | 4 | Low | ✓ openSUSE | ✗ |
| pacman | Package | 4 | Low | ✓ Arch | ✗ |
| pip | Package | 3 | High | ✓ All | ✓ |
| git | SCM | 3 | High | ✓ All | ✓ |
| template | File | 2 | High | ✓ All | ✓ |
| lineinfile | File | 3 | High | ✓ All | ✗ |
| blockinfile | File | 3 | Medium | ✓ All | ✗ |
| service | Service | 3 | High | ✓ All | ✓ |
| systemd | Service | 3 | Medium | ✓ Modern Linux | ✗ |
| user | User | 3 | Medium | ✓ All | ✓ |
| group | User | 3 | Medium | ✓ All | ✓ |
| setup | Facts | 2 | High | ✓ All | ✓ |
| pause | Control | 2 | Low | ✓ All | ✓ |
| wait_for | Network | 3 | Medium | ✓ All | ✓ |

### Advanced Modules (Phase 4-5)

- **Cloud**: ec2, azure_rm_*, gcp_* (focus on Python wrapper)
- **Database**: postgresql_db, mysql_db (Python wrapper)
- **Network**: network_interface, etc. (Python wrapper)
- **Containerization**: docker_* (Python wrapper)

---

## Module Implementation Roadmap

### Phase 1: Subprocess Wrapper

**Infrastructure**:
```rust
pub struct PythonModuleInvoker {
    python_bin: PathBuf,
    module_path: PathBuf,
}

impl ModuleInvoker for PythonModuleInvoker {
    fn invoke(&self, task: &Task, host: &str, ctx: &ExecutionContext) 
        -> Result<TaskResult> 
    {
        // 1. Serialize task args to JSON (module_args)
        // 2. Write to tempfile or stdin
        // 3. Exec: python -m ansible.module_utils.basic <module_name>
        // 4. Parse JSON result
        // 5. Return TaskResult
    }
}
```

**Supports**: All existing Python modules without modification

---

### Phase 2: Rust Native Implementations

#### 1. **shell & command** (Week 7)

```rust
// crates/ansiblers-modules/src/command.rs

pub struct CommandModule;

impl ModuleInvoker for CommandModule {
    fn invoke(&self, task: &Task, host: &str, ctx: &ExecutionContext) -> Result<TaskResult> {
        let cmd = task.args.get("_raw_params").and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("command requires _raw_params"))?;
        
        // Execute: subprocess with optional stdin, timeout, cwd, env
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()?;
        
        Ok(TaskResult {
            status: if output.status.success() { "ok" } else { "failed" },
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            rc: output.status.code().unwrap_or(-1),
            changed: true,
            ..Default::default()
        })
    }
}
```

**Test Fixtures**:
```yaml
# tests/fixtures/playbooks/test_command.yml
- hosts: localhost
  tasks:
    - name: Simple command
      command: echo "hello world"
      register: result
    
    - name: Verify
      assert:
        that:
          - result.stdout == "hello world"
```

#### 2. **debug** (Week 10)

```rust
pub struct DebugModule;

impl ModuleInvoker for DebugModule {
    fn invoke(&self, task: &Task, host: &str, ctx: &ExecutionContext) -> Result<TaskResult> {
        // Output: msg, var, var_name
        let output = if let Some(msg) = task.args.get("msg") {
            render_template(msg, ctx)?
        } else if let Some(var) = task.args.get("var") {
            ctx.vars.get(var).map(|v| v.to_string()).unwrap_or_default()
        } else {
            "".to_string()
        };
        
        // Print or log output
        println!("{}", output);
        
        Ok(TaskResult {
            status: "ok",
            stdout: output,
            changed: false,
            ..Default::default()
        })
    }
}
```

#### 3. **set_fact** (Week 10)

```rust
pub struct SetFactModule;

impl ModuleInvoker for SetFactModule {
    fn invoke(&self, task: &Task, host: &str, ctx: &ExecutionContext) -> Result<TaskResult> {
        // Set facts (variables) for the host
        for (key, value) in task.args.iter() {
            if key.starts_with('_') { continue; } // Skip metadata
            ctx.facts.insert(host.to_string(), key.clone(), value.clone());
        }
        
        Ok(TaskResult {
            status: "ok",
            changed: false,
            vars_set: task.args.clone(),
            ..Default::default()
        })
    }
}
```

#### 4. **file** (Week 11, partial)

```rust
pub struct FileModule;

impl ModuleInvoker for FileModule {
    fn invoke(&self, task: &Task, host: &str, ctx: &ExecutionContext) -> Result<TaskResult> {
        let path = task.args.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("file requires path"))?;
        
        let state = task.args.get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("file");
        
        match state {
            "file" => {
                std::fs::File::create(path)?;
                Ok(TaskResult {
                    status: "ok",
                    changed: true,
                    msg: format!("File {} created", path),
                    ..Default::default()
                })
            },
            "directory" => {
                std::fs::create_dir_all(path)?;
                Ok(TaskResult {
                    status: "ok",
                    changed: true,
                    msg: format!("Directory {} created", path),
                    ..Default::default()
                })
            },
            "absent" => {
                if Path::new(path).is_file() {
                    std::fs::remove_file(path)?;
                } else if Path::new(path).is_dir() {
                    std::fs::remove_dir_all(path)?;
                }
                Ok(TaskResult {
                    status: "ok",
                    changed: true,
                    msg: format!("File {} removed", path),
                    ..Default::default()
                })
            },
            _ => Err(anyhow!("Invalid state: {}", state)),
        }
    }
}
```

---

### Phase 3: Performance-Critical Modules (Weeks 23-30)

Target modules with measurement-backed prioritization:

#### Measurement Approach
```bash
# 1. Profile Ansible execution
python -m cProfile -s cumulative $(which ansible-playbook) playbook.yml

# 2. Identify modules consuming >10% of time
# 3. Rewrite and benchmark:
cargo bench --bench module_execution

# 4. Compare performance
# 5. If >2x speedup, accept; otherwise, defer
```

#### Priority 1: apt (Week 23)

```rust
pub struct AptModule;

impl ModuleInvoker for AptModule {
    fn invoke(&self, task: &Task, host: &str, ctx: &ExecutionContext) -> Result<TaskResult> {
        // Package management for Debian/Ubuntu
        // Operations: present, absent, latest, removed, purged
        
        let name = task.args.get("name").and_then(|v| v.as_str())?;
        let state = task.args.get("state").and_then(|v| v.as_str()).unwrap_or("present");
        let update_cache = task.args.get("update_cache").and_then(|v| v.as_bool()).unwrap_or(false);
        
        // 1. Update cache if requested: apt-get update
        // 2. Install/remove packages: apt-get install/remove
        // 3. Return result with package list and changed status
        
        Ok(TaskResult {
            status: "ok",
            changed: false,  // Determine based on actual changes
            ..Default::default()
        })
    }
}
```

**Test Fixtures**:
```yaml
# tests/fixtures/playbooks/test_apt.yml
- hosts: ubuntu_hosts
  tasks:
    - name: Install package
      apt:
        name: curl
        state: present
      register: apt_result
    
    - name: Assert changed
      assert:
        that:
          - apt_result.changed or "already the newest version" in apt_result.stdout
```

#### Priority 2: yum (Week 24)

```rust
pub struct YumModule;
// Similar structure to apt, for RHEL/CentOS
```

#### Priority 3: git (Week 27)

```rust
pub struct GitModule;

impl ModuleInvoker for GitModule {
    fn invoke(&self, task: &Task, host: &str, ctx: &ExecutionContext) -> Result<TaskResult> {
        let repo = task.args.get("repo").and_then(|v| v.as_str())?;
        let dest = task.args.get("dest").and_then(|v| v.as_str())?;
        let version = task.args.get("version").and_then(|v| v.as_str()).unwrap_or("HEAD");
        
        // 1. Check if repo exists
        // 2. Clone or pull: git clone / git pull
        // 3. Checkout version: git checkout
        // 4. Return changed status and revision
        
        Ok(TaskResult {
            status: "ok",
            changed: false,
            ..Default::default()
        })
    }
}
```

#### Priority 4: template (Week 25)

```rust
pub struct TemplateModule;

impl ModuleInvoker for TemplateModule {
    fn invoke(&self, task: &Task, host: &str, ctx: &ExecutionContext) -> Result<TaskResult> {
        let src = task.args.get("src").and_then(|v| v.as_str())?;
        let dest = task.args.get("dest").and_then(|v| v.as_str())?;
        
        // 1. Read template file
        // 2. Render with Jinja2 (using minijinja or PyO3)
        // 3. Write to destination
        // 4. Set permissions/ownership if specified
        // 5. Return changed status
        
        Ok(TaskResult {
            status: "ok",
            changed: false,
            ..Default::default()
        })
    }
}
```

---

## Module Implementation Checklist

For each module, ensure:

- [ ] **Functionality**
  - [ ] Core operations implemented
  - [ ] Error handling for invalid inputs
  - [ ] Return values match Ansible spec

- [ ] **Testing**
  - [ ] Fixture playbook created
  - [ ] Unit tests with rstest parametrization
  - [ ] Edge case tests (empty inputs, missing args, etc.)
  - [ ] 80%+ coverage with branch coverage analysis

- [ ] **Documentation**
  - [ ] Docstring with module description
  - [ ] Supported operations listed
  - [ ] Known limitations noted
  - [ ] Performance comparison with Python version

- [ ] **Performance**
  - [ ] Benchmark vs Python version
  - [ ] Document speedup or justify Python wrapper use
  - [ ] Profile for memory usage

- [ ] **Integration**
  - [ ] Works with task registration (register_var)
  - [ ] Works with conditionals (when)
  - [ ] Works with loops (loop, with_items)
  - [ ] Works with delegation (delegate_to)

---

## Module Registry Pattern

```rust
// crates/ansiblers-modules/src/lib.rs

pub trait ModuleInvoker: Send + Sync {
    fn invoke(&self, task: &Task, host: &str, ctx: &ExecutionContext) 
        -> Result<TaskResult>;
}

pub struct ModuleRegistry {
    modules: HashMap<String, Arc<dyn ModuleInvoker>>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        let mut registry = ModuleRegistry {
            modules: HashMap::new(),
        };
        
        // Register Rust native modules
        registry.register("shell", Arc::new(ShellModule));
        registry.register("command", Arc::new(CommandModule));
        registry.register("debug", Arc::new(DebugModule));
        registry.register("set_fact", Arc::new(SetFactModule));
        registry.register("file", Arc::new(FileModule));
        
        // Default to Python wrapper for all others
        registry.default_invoker = Arc::new(PythonModuleInvoker::default());
        
        registry
    }
    
    pub fn register(&mut self, name: &str, invoker: Arc<dyn ModuleInvoker>) {
        self.modules.insert(name.to_string(), invoker);
    }
    
    pub fn get(&self, name: &str) -> Arc<dyn ModuleInvoker> {
        self.modules.get(name)
            .cloned()
            .unwrap_or_else(|| self.default_invoker.clone())
    }
}
```

---

## Performance Benchmarking Template

For each rewritten module:

```rust
// benches/module_apt.rs

use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_apt_install(c: &mut Criterion) {
    c.bench_function("apt_install_single_package", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| async {
                let module = AptModule;
                let task = Task {
                    module: "apt".to_string(),
                    args: black_box(vec![
                        ("name", "curl"),
                        ("state", "present"),
                    ].into_iter().collect()),
                };
                module.invoke(&task, "localhost", &default_context()).await
            })
    });
}

criterion_group!(benches, bench_apt_install);
criterion_main!(benches);
```

Store results in `reports/benchmarks/module_performance.csv`:

```csv
module,operation,python_ms,rust_ms,speedup
apt,install_single,250,100,2.5x
apt,install_multiple,500,150,3.3x
command,simple_echo,50,10,5x
shell,complex_script,200,80,2.5x
```

---

## Migration Timeline

| Phase | Weeks | Modules | Approach |
|-------|-------|---------|----------|
| 1 | 1-8 | shell, command | Rust native |
| 2 | 9-14 | debug, set_fact, (file/copy wrapper) | Rust + wrapper |
| 3 | 15-18 | (role support) | No new modules |
| 4 | 19-22 | (test runner) | No new modules |
| 5 | 23-30 | apt, yum, git, template, setup | Profiled rewrites |
| 5 | 31-32 | packaging, service | Continued rewrites |
| 6+ | Ongoing | Advanced modules | Strategic rewrites |

---

## Decision Tree: Rewrite vs Wrapper

```
New Module Requested
    ↓
1. Does Python module exist?
    ↓
    YES: Can we use subprocess wrapper?
    │   ├─ YES: Use Python wrapper (fast to implement)
    │   └─ NO: Move to "Rewrite" decision
    │
    NO: Implement in Rust
    ↓
2. Is module performance-critical?
    │   (profiling shows >10% of execution time)
    │
    ├─ YES: Rewrite in Rust (aim for 2x+ speedup)
    │
    └─ NO: Use Python wrapper (reduce maintenance burden)
    
3. Validate with benchmarking
    ├─ >2x speedup: Accept Rust version
    └─ <2x speedup: Revert to Python wrapper
```

---

## Ansible Module Compatibility Notes

- **Version Pinning**: Target Ansible 2.14+ module API
- **Argument Validation**: Implement same validation as Python modules
- **Return Values**: Match Ansible spec exactly
- **Deprecations**: Track and implement deprecation warnings

### Known Limitations (Phase 1-2)

- No Windows PowerShell support (Linux focus)
- No cloud provider modules (Phase 4+)
- Dynamic inventory only via shell scripts (not Python)
- No custom callback plugins (future)
