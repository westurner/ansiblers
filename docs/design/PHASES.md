# Ansiblers Development Phases - Detailed Roadmap

## Phase 1: Foundation & Proof of Concept (Weeks 1-8)

### Goals
- Establish Rust workspace structure
- Parse basic Ansible playbooks
- Load inventories with variables
- Execute simple shell tasks
- Verify output compatibility with Ansible

### Milestones

#### Week 1-2: Project Setup
- [ ] Create Cargo workspace with initial crates
- [ ] Set up CI/CD with GitHub Actions
- [ ] Configure cargo-llvm-cov for coverage tracking
- [ ] Create test fixture directories
- [ ] Define coding standards and design patterns

**Deliverables**:
```
Cargo.toml (workspace)
crates/
  ├── ansiblers-core/Cargo.toml
  ├── ansiblers-parser/Cargo.toml
  ├── ansiblers-inventory/Cargo.toml
  ├── ansiblers-vars/Cargo.toml
  ├── ansiblers-templates/Cargo.toml
  ├── ansiblers-executor/Cargo.toml
  ├── ansiblers-modules/Cargo.toml
  └── ansiblers-playbook/Cargo.toml

GitHub Actions:
  ├── test.yml (run tests with coverage)
  ├── lint.yml (clippy, fmt)
  └── bench.yml (performance tracking)
```

#### Week 2-3: Core Data Structures (ansiblers-core)
- [ ] Define ExecutionContext (inventory, vars, facts, connections)
- [ ] Define TaskResult with all Ansible-compatible fields
- [ ] Define HostState (facts, var overrides)
- [ ] Error types and handling strategy
- [ ] Initial unit tests with rstest fixtures

**Rust Types**:
```rust
pub struct ExecutionContext {
    pub inventory: Arc<Inventory>,
    pub playbook_vars: HashMap<String, Value>,
    pub registered_vars: HashMap<String, Value>,
    pub facts: HashMap<String, HashMap<String, Value>>,
    pub connection_pool: ConnectionPool,
}

pub struct TaskResult {
    pub host: String,
    pub status: TaskStatus,      // ok, failed, skipped, unreachable
    pub stdout: String,
    pub stderr: String,
    pub rc: i32,
    pub changed: bool,
    pub msg: String,
    pub vars_set: HashMap<String, Value>,
}
```

#### Week 3-4: Playbook Parser (ansiblers-parser)
- [ ] YAML parsing with serde_yaml
- [ ] Playbook AST structure
- [ ] Play and Task representation
- [ ] Handler support
- [ ] Block/rescue/always parsing
- [ ] Include/import directive parsing

**Test Coverage Target**: 80% line coverage

#### Week 4-5: Inventory System (ansiblers-inventory)
- [ ] INI format parsing
- [ ] Host and group management
- [ ] Variable merging (inventory vars)
- [ ] Host/group fact storage
- [ ] Inventory validation

**Fixtures**:
```
tests/fixtures/inventories/
  ├── simple.ini
  ├── with_groups.ini
  ├── with_vars.ini
  └── complex.ini
```

#### Week 5-6: Variable Resolution (ansiblers-vars)
- [ ] Precedence implementation
- [ ] Variable interpolation in strings
- [ ] Jinja2 template context preparation
- [ ] Fact caching per-host
- [ ] Variable merging strategies

**rstest Parametrization**:
```rust
#[rstest]
#[case("{{ var }}", "value")]
#[case("{{ nested.var }}", "nested_value")]
#[case("{% for i in items %}...{% endfor %}", "loop_output")]
fn test_variable_resolution(#[case] input: &str, #[case] expected: &str) { }
```

#### Week 6-7: Template Rendering (ansiblers-templates)
- [ ] minijinja integration
- [ ] Ansible filter implementation (default, bool, etc.)
- [ ] Variable substitution in task parameters
- [ ] Conditional expression evaluation
- [ ] Error handling for invalid templates

**Supported Filters** (Phase 1):
- `default(value)`
- `bool`
- `quote`
- `length`, `count`
- `upper`, `lower`
- `from_json`, `to_json`

#### Week 7-8: Shell Module & Executor (ansiblers-executor, ansiblers-modules)
- [ ] Shell module native implementation
- [ ] Command module native implementation
- [ ] Task parameter resolution
- [ ] When condition evaluation
- [ ] Register variable capture
- [ ] Simple playbook execution coordination

**Test Fixtures**:
```yaml
# tests/fixtures/playbooks/simple_shell.yml
- hosts: all
  tasks:
    - name: Run shell command
      shell: echo "hello world"
      register: result
    
    - name: Display output
      debug:
        msg: "{{ result.stdout }}"
```

#### Week 8: Integration & Polish
- [ ] ransible-playbook binary with basic CLI args
- [ ] Output formatting (human, JSON)
- [ ] Exit code compatibility
- [ ] Documentation and README
- [ ] CI/CD pipeline validation

### Success Criteria for Phase 1

- ✅ Execute basic playbook with shell tasks
- ✅ Variable substitution works end-to-end
- ✅ Output matches Ansible formatting
- ✅ 75% line coverage on all crates
- ✅ Zero unsafe code (or well-justified)
- ✅ All tests pass with cargo-llvm-cov coverage data

---

## Phase 2: Core Module Support (Weeks 9-14)

### Goals
- Python module wrapper for broad compatibility
- Essential modules in Rust (file, copy, debug)
- Full task control flow (blocks, handlers)
- Multi-host execution coordination

### Milestones

#### Week 9: Python Module Wrapper (ansiblers-modules)
- [ ] Subprocess module invocation
- [ ] Module argument JSON passing
- [ ] Result parsing (JSON)
- [ ] Environment variable setup
- [ ] Error handling and timeouts

**Implementation**:
```rust
pub struct PythonModuleInvoker {
    module_path: PathBuf,
    python_bin: PathBuf,
}

impl ModuleInvoker for PythonModuleInvoker {
    fn invoke(&self, args: &ModuleArgs) -> Result<TaskResult> {
        // Serialize args to JSON
        // Invoke python module
        // Parse JSON result
    }
}
```

#### Week 10-11: High-Value Modules in Rust
- [ ] debug module (display variables)
- [ ] file module (create, delete, permissions)
- [ ] copy module (file transfer basics)
- [ ] stat module (file information)
- [ ] set_fact module (variable registration)

**Coverage Target**: 80% branch coverage

#### Week 11-12: Task Control Flow
- [ ] Block support with nested tasks
- [ ] Rescue block execution
- [ ] Always block guarantee
- [ ] Handler registration and execution
- [ ] Task dependency ordering

**Test Cases**:
```rust
#[rstest]
#[case("blocks_with_rescue.yml")]
#[case("handlers_execution.yml")]
#[case("nested_blocks.yml")]
fn test_control_flow(playbook: &str) { }
```

#### Week 12-13: Multi-Host Execution
- [ ] Parallel task execution (fan-out)
- [ ] Host-by-host execution strategy
- [ ] Fact gathering per-host
- [ ] Register variable isolation
- [ ] Failure handling strategies (fail-fast vs continue)

#### Week 13-14: Integration & Testing
- [ ] Integration test suite with fixtures
- [ ] Python module wrapper stability
- [ ] Performance benchmarking vs Ansible
- [ ] Documentation updates

### Deliverables
- Complete module wrapper for Python modules
- 5+ Rust-native modules
- Block/rescue/always fully functional
- Multi-host execution working
- Benchmark reports in `reports/benchmarks/`

---

## Phase 3: Inventory & Role Support (Weeks 15-18)

### Goals
- Full inventory format support (YAML)
- Group and host variable files
- Role loading and execution
- Role dependencies
- Basic ansible-galaxy support

### Milestones

#### Week 15: YAML Inventory & Variables
- [ ] YAML inventory format parsing
- [ ] Group/host metadata support
- [ ] group_vars directory loading
- [ ] host_vars directory loading
- [ ] Variable merging and precedence
- [ ] Dynamic inventory stub (shell scripts)

**Test Fixtures**:
```
tests/fixtures/inventories/
  ├── group_vars/
  │   ├── all.yml
  │   ├── webservers.yml
  │   └── databases.yml
  └── host_vars/
      ├── web1.example.com.yml
      └── db1.example.com.yml
```

#### Week 16: Role Loading & Execution
- [ ] Role directory structure validation
- [ ] Tasks, handlers, vars, defaults loading
- [ ] Role variable precedence
- [ ] Role include/import in playbooks
- [ ] Role metadata parsing

#### Week 17: Role Dependencies & Galaxy
- [ ] meta/main.yml parsing for role dependencies
- [ ] Recursive role dependency resolution
- [ ] Basic galaxy metadata support
- [ ] Role path configuration (ansible.cfg)
- [ ] Galaxy requirements.yml parsing (stub)

#### Week 18: Integration & Validation
- [ ] Test with real roles from galaxy
- [ ] Compatibility with standard role structures
- [ ] Performance analysis of role loading
- [ ] Documentation

### Deliverables
- Full inventory support (INI + YAML)
- Role loading and execution
- Role dependency resolution
- Snapshot tests for role loading outputs

---

## Phase 4: ransible-test Implementation (Weeks 19-22)

### Goals
- ansible-test compatible test runner
- Container management for test isolation
- Coverage collection integration
- Sanity test coordination

### Milestones

#### Week 19-20: Test Discovery & Organization
- [ ] Test target directory structure understanding
- [ ] Test type classification (sanity, unit, integration)
- [ ] Test discovery logic
- [ ] Test execution planning
- [ ] Result aggregation

#### Week 20-21: Container Management & Isolation
- [ ] Docker container creation/cleanup
- [ ] Test environment setup
- [ ] Container networking
- [ ] Volume mounting for tests
- [ ] Container resource limits

#### Week 21-22: Coverage & Reporting
- [ ] cargo-llvm-cov integration
- [ ] HTML report generation
- [ ] Coverage thresholds
- [ ] CI reporting

### Deliverables
- ransible-test binary with basic functionality
- Container-based test isolation
- Coverage reports integrated with CI
- Documentation of test running

---

## Phase 5: High-Value Module Rewrites (Weeks 23-32)

### Prioritization Strategy

**Measure ansible-playbook execution profiling**:
1. Run on representative workloads
2. Identify slowest modules
3. Calculate rewrite ROI (time saved vs effort)
4. Prioritize >2x speedup candidates

### Week 23-24: Package Managers (apt, yum)
- [ ] apt module implementation
- [ ] yum/dnf module implementation
- [ ] Package caching
- [ ] Repository management
- [ ] Performance comparison

### Week 25-26: File Operations (file, find, template)
- [ ] file module (create, delete, permissions, ownership)
- [ ] find module (directory traversal)
- [ ] template module (Jinja2 rendering to file)
- [ ] lineinfile module (text manipulation)
- [ ] Performance benchmarking

### Week 27-28: Facts Gathering (setup module)
- [ ] setup module in Rust
- [ ] Fact collection optimization
- [ ] Fact caching
- [ ] Fact merging

### Week 29-30: Git Operations
- [ ] git module implementation
- [ ] Clone/pull optimization
- [ ] Performance comparison with Python

### Week 31-32: Testing & Benchmarking
- [ ] Comprehensive test suite for rewritten modules
- [ ] Benchmarks vs Python Ansible
- [ ] Integration tests with playbooks
- [ ] Performance reports in reports/

### Coverage Target
- 85% line coverage for rewritten modules
- 75% branch coverage

---

## Phase 6: Performance Optimization & Integration (Weeks 33+)

### Goals
- Production-ready performance
- Integration opportunities with Ansible core
- Documentation and adoption guide

### Milestones

#### Weeks 33-34: Connection Pooling & Optimization
- [ ] Connection reuse across tasks
- [ ] Connection timeout handling
- [ ] SSH keepalive configuration
- [ ] Performance analysis

#### Weeks 35-36: Parallel Execution
- [ ] Fan-out parallelization
- [ ] Host-by-host serial execution
- [ ] Batch execution strategies
- [ ] Tokio async runtime optimization

#### Weeks 37-38: Module Caching & Precompilation
- [ ] Module compilation caching
- [ ] Module dependency resolution
- [ ] Fast module loading
- [ ] Benchmark improvements

#### Weeks 39-40: Benchmarking Suite
- [ ] Standard workload benchmarks
- [ ] Comparison with Python Ansible
- [ ] Performance regression detection
- [ ] Reports and dashboards

#### Weeks 41-42: Documentation & Adoption
- [ ] Architecture documentation
- [ ] Performance tuning guide
- [ ] Migration guide from ansible-playbook
- [ ] Troubleshooting guide
- [ ] Community outreach

#### Weeks 43+: Integration with Ansible Core
- [ ] Identify hot paths for PyO3 optimization
- [ ] Prototype template rendering backend switch
- [ ] Module execution acceleration
- [ ] Upstream contribution discussions

---

## Cross-Phase Activities

### Testing Continuous Effort
- **Weekly**: Run full test suite with coverage analysis
- **Per PR**: Coverage report, branch coverage targets
- **Monthly**: Performance regression analysis

### Documentation Continuous Effort
- **Per Feature**: Update architecture docs
- **Per Release**: Update user-facing docs
- **Quarterly**: Major documentation reviews

### Benchmarking Continuous Effort
- **Per Phase**: Establish baseline benchmarks
- **Per Optimization**: Before/after comparison
- **Monthly**: Trend analysis and reports

---

## Risk Checkpoints

### Phase 1 → Phase 2 Gate
- ✅ Basic playbooks execute successfully
- ✅ Core architecture validated
- ✅ No critical bugs in foundation
- ✅ Community feedback positive

### Phase 2 → Phase 3 Gate
- ✅ Module wrapper stable
- ✅ Multi-host execution reliable
- ✅ Performance meets targets (50%+ speedup)
- ✅ Test coverage >75%

### Phase 3 → Phase 4 Gate
- ✅ Role support complete and tested
- ✅ Inventory handling matches Ansible
- ✅ No breaking incompatibilities discovered

### Phase 4 → Phase 5 Gate
- ✅ ransible-test usable for basic workflows
- ✅ Container isolation working reliably

### Phase 5 → Phase 6 Gate
- ✅ Key modules rewritten and tested
- ✅ Performance targets being met
- ✅ Community interest growing

---

## Effort Estimate

| Phase | Duration | Team | Focus |
|-------|----------|------|-------|
| 1 | 8 weeks | 1 | Architecture & foundations |
| 2 | 6 weeks | 1-2 | Modules & compatibility |
| 3 | 4 weeks | 1 | Roles & inventory |
| 4 | 4 weeks | 1 | Test runner |
| 5 | 10 weeks | 2 | Module rewrites, benchmarking |
| 6+ | Ongoing | 1+ | Optimization & integration |

**Total Estimated Effort**: 32+ weeks (8 months) to production readiness

---

## Contingency Plans

### If Python Module Wrapper Problematic
- **Contingency**: Rewrite critical modules first (command, shell, file)
- **Risk**: Longer phase 1-2, delayed compatibility
- **Mitigation**: Start module rewrites earlier than planned

### If Jinja2 Compatibility Issues
- **Contingency**: Use PyO3 binding to Python jinja2 for all rendering
- **Risk**: Performance hit, Python dependency
- **Mitigation**: Identify gaps early, prioritize fixes

### If Performance Targets Missed
- **Contingency**: Focus on specific hot paths identified by profiling
- **Risk**: Limited speedup, reduced adoption
- **Mitigation**: Monthly benchmarking to catch regressions early

---

## Success Metrics

- **Functionality**: All standard playbooks execute without modification
- **Performance**: 5-10x speedup on core operations
- **Quality**: 75%+ line coverage, 60%+ branch coverage
- **Adoption**: Used successfully in production by 5+ organizations
- **Integration**: Identified optimization opportunities for Ansible core
