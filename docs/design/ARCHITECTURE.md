# Ansiblers: Rust Implementation of Ansible - Architecture & Plan

## Vision

**Ansiblers** is a Rust-based, drop-in-compatible reimplementation of Ansible that maintains 100% compatibility with standard Ansible playbooks, inventory files, and the ansible-galaxy ecosystem. The primary goal is to create high-performance Rust binaries (`ransible-playbook`, `ransible-test`, `ransible-molecule`) that can be used as direct replacements for their Python counterparts, with eventual optimization and integration back into Ansible core.

### Key Objectives

1. **Drop-in Compatibility**: Standard Ansible playbooks, roles, inventories, and galaxy modules work unchanged
2. **Performance**: Rust implementation provides significant speedup for I/O-bound operations and task execution
3. **Modular Design**: Clearly separated concerns enabling incremental development and testing
4. **Testability**: Comprehensive test coverage with branch coverage analysis (cargo-llvm-cov)
5. **Integration**: Eventually integrate back into Ansible core for performance-critical paths

---

## Project Structure

```
ansiblers/
├── src/
│   ├── ansiblers/                    # Main Rust workspace root
│   │   ├── Cargo.toml               # Workspace root manifest
│   │   ├── Cargo.lock               # Dependency lock file
│   │   └── crates/                  # Individual crates
│   │       ├── ansiblers-core/      # Core execution engine
│   │       ├── ansiblers-playbook/  # ransible-playbook binary
│   │       ├── ansiblers-test/      # ransible-test binary
│   │       ├── ansiblers-molecule/  # ransible-molecule binary (future)
│   │       ├── ansiblers-parser/    # YAML/playbook parsing
│   │       ├── ansiblers-inventory/ # Inventory management
│   │       ├── ansiblers-modules/   # Module invocation & loading
│   │       ├── ansiblers-templates/ # Jinja2/template rendering
│   │       ├── ansiblers-vars/      # Variable resolution
│   │       ├── ansiblers-executor/  # Task execution engine
│   │       └── ansiblers-compat/    # Python compatibility layer (via PyO3/Maturin)
│   └── ansible/                     # Original Ansible source (for reference)
├── docs/
│   └── design/                      # Design documents
│       ├── ARCHITECTURE.md          # This file
│       ├── PHASES.md                # Development phases
│       ├── TESTING_STRATEGY.md      # Testing approach
│       └── MODULES_MAPPING.md       # Ansible modules mapping
├── tests/                           # Integration tests
├── reports/                         # Test results and coverage reports
└── tmp/                             # Temporary files during development
```

---

## Core Components

### 1. **ansiblers-core** (Foundation)
**Responsibility**: Central coordination, context management, and core abstractions

**Key Types**:
- `ExecutionContext`: Global state (inventory, vars, facts, connection pool)
- `TaskResult`: Task execution result with status, stdout, stderr, changed flag
- `HostState`: Per-host state tracking (facts, var overrides, connection info)

**Dependencies**: Minimal (serde, anyhow for error handling)

---

### 2. **ansiblers-parser** (Input Processing)
**Responsibility**: Parse Ansible playbooks, roles, and inventory files

**Key Features**:
- YAML parsing (using `serde_yaml` or `yaml-rust`)
- Playbook AST construction
- Role directory traversal and loading
- Include/import directive resolution
- Variable file parsing (group_vars, host_vars, defaults, vars)

**Dependencies**: `serde_yaml`, `anyhow`, glob patterns

---

### 3. **ansiblers-inventory** (Host Management)
**Responsibility**: Load and manage inventory files (INI, YAML, dynamic)

**Key Features**:
- Inventory file parsing (INI, YAML formats)
- Group/host management with hierarchy
- Variable merging (inventory vars + group_vars + host_vars)
- Dynamic inventory support (stub for Python scripts)
- Hostvars/groupvars resolution

**Dependencies**: `serde_yaml`, `regex`, ansiblers-parser

---

### 4. **ansiblers-vars** (Variable Resolution)
**Responsibility**: Handle variable resolution, templating context, and precedence

**Key Features**:
- Variable precedence implementation (defaults > inventory > vars > host_vars > role_defaults)
- Fact caching per-host
- Variable merging and override strategies
- Expression evaluation for conditional logic

**Dependencies**: ansiblers-core, regex

---

### 5. **ansiblers-templates** (Jinja2 Rendering)
**Responsibility**: Template rendering compatible with Ansible's Jinja2 filters

**Key Features**:
- Jinja2 template rendering (using `minijinja` crate or full `jinja2` via PyO3)
- Ansible custom filters (default, bool, etc.)
- Variable interpolation in strings, task parameters, conditions
- Loop context management (item, index)

**Tech Decision**: 
- **minijinja**: Pure Rust, faster, ~95% Jinja2 compatible
- **PyO3 binding**: Full compatibility with Python Jinja2 (slower but guaranteed compatibility)
- **Start with minijinja**, fallback to PyO3 for unsupported cases

---

### 6. **ansiblers-executor** (Task Execution)
**Responsibility**: Execute individual tasks and manage task flow

**Key Features**:
- Task parameter resolution
- Conditional evaluation (when, failed_when, changed_when)
- Block/rescue/always support
- Loop expansion (with_items, loop)
- Delegation support
- Connection pooling
- Register variable capture

**Dependencies**: All above crates

---

### 7. **ansiblers-modules** (Module Bridge)
**Responsibility**: Invoke Ansible modules (initially shell/command, gradually convert)

**Design Philosophy**:
- **Phase 1**: Shell out to Python modules with direct invocation
- **Phase 2**: High-value modules in Rust (command, shell, copy, file, etc.)
- **Phase 3**: Advanced modules (package managers, cloud providers)

**Key Types**:
- `ModuleRegistry`: Maps module names to implementations
- `ModuleInvoker`: Abstract trait for module execution
- `PythonModuleWrapper`: Wraps Python modules via subprocess

**Dependencies**: Python subprocess interaction initially

---

### 8. **ansiblers-playbook** (Binary)
**Responsibility**: `ransible-playbook` CLI entry point

**Features**:
- Full argument parsing compatible with `ansible-playbook`
- Playbook loading and execution coordination
- Output formatting (verbose, quiet, JSON)
- Exit codes matching Ansible
- Connection plugin management

---

### 9. **ansiblers-test** (Binary)
**Responsibility**: `ransible-test` CLI entry point

**Scope** (Phase 1):
- Sanity test runner
- Unit test coordination
- Basic test discovery

**Features**:
- Docker/container management (via existing ansible-test)
- Test isolation
- Result aggregation

---

### 10. **ansiblers-compat** (PyO3 Bridge)
**Responsibility**: Python interoperability for:
- Full Jinja2 compatibility (fallback rendering)
- Dynamic inventory Python scripts
- Custom module execution

**Tech**: PyO3 + Maturin for seamless Python integration

---

## Development Phases

### Phase 1: Foundation & Proof of Concept (Months 1-2)
**Goals**: Basic playbook execution with simple tasks

**Deliverables**:
- [ ] Core data structures (ExecutionContext, TaskResult)
- [ ] YAML playbook parsing
- [ ] Inventory loading (INI format)
- [ ] Variable resolution engine
- [ ] Template rendering (minijinja)
- [ ] Shell command module (Rust native)
- [ ] ransible-playbook binary (basic functionality)
- [ ] Comprehensive unit tests with rstest fixtures

**Success Criteria**:
- Execute simple playbook with shell tasks
- Variable substitution working
- Proper exit codes

---

### Phase 2: Core Module Support (Months 2-3)
**Goals**: Essential modules for common use cases

**Deliverables**:
- [ ] Python module wrapper (subprocess invocation)
- [ ] Command, shell, copy, file modules (Rust or wrapped)
- [ ] Support for blocks, rescue, always
- [ ] Handler support
- [ ] Registered variables and variable capture
- [ ] include_tasks, import_tasks directives
- [ ] Integration tests with test fixtures

**Success Criteria**:
- Run standard Ansible playbooks without modification
- Results match Python Ansible
- All test modules pass through wrapper

---

### Phase 3: Inventory & Role Support (Month 3)
**Goals**: Full role and dynamic inventory support

**Deliverables**:
- [ ] YAML inventory format
- [ ] Group/host variable files (group_vars, host_vars)
- [ ] Role loading and execution
- [ ] Role dependencies (meta/main.yml)
- [ ] Dynamic inventory stub (shell scripts)
- [ ] ansible-galaxy integration (basic)

---

### Phase 4: Ransible-Test Implementation (Month 3-4)
**Goals**: Test runner compatible with ansible-test

**Deliverables**:
- [ ] Test discovery and organization
- [ ] Docker container management
- [ ] Sanity test coordination
- [ ] Unit test running
- [ ] Coverage collection with cargo-llvm-cov

---

### Phase 5: High-Value Module Rewrites (Months 4-6)
**Goals**: Rust-native implementations of slow/critical modules

**Candidates** (by profiling):
- [ ] Package managers (apt, yum, dnf, zypper)
- [ ] File operations (file, find, template)
- [ ] Facts gathering (setup module)
- [ ] Git operations (git module)

**Tech**: Measure Python vs Rust performance gains

---

### Phase 6: Performance Optimization & Integration (Months 6+)
**Goals**: Production-ready performance, integration with Ansible core

**Deliverables**:
- [ ] Connection pooling optimization
- [ ] Parallel task execution (fan-out patterns)
- [ ] Module execution batching
- [ ] Integration tests with real Ansible workflows
- [ ] Benchmarking suite
- [ ] Documentation and adoption guide

---

## Testing Strategy

### Test Structure

```
tests/
├── fixtures/                        # Shared test fixtures
│   ├── playbooks/                  # Test playbooks
│   ├── inventories/                # Test inventories
│   ├── roles/                      # Test roles
│   └── modules/                    # Test modules
├── integration/                     # Integration tests
│   ├── test_basic_playbook.rs
│   ├── test_modules.rs
│   └── test_roles.rs
├── unit/                           # Unit tests (in-crate, via #[cfg(test)])
└── coverage/                       # Coverage reports
```

### Testing Tools & Practices

1. **rstest**: Fixtures, parametrization, and mocking
   ```rust
   #[fixture]
   fn execution_context() -> ExecutionContext { ... }
   
   #[rstest]
   #[case("playbook1.yml")]
   #[case("playbook2.yml")]
   fn test_playbook_execution(#[from(execution_context)] ctx: ExecutionContext, #[case] playbook: &str) { ... }
   ```

2. **cargo-insta**: Snapshot testing for complex outputs
   ```rust
   let output = execute_playbook("test.yml");
   insta::assert_json_snapshot!(output);
   ```

3. **cargo-llvm-cov**: Branch coverage analysis
   ```bash
   cargo llvm-cov --out Lcov
   cargo llvm-cov report --html
   ```

4. **Fixture-based approach**:
   - Reusable test playbooks in `tests/fixtures/playbooks/`
   - Standard test inventories and role structures
   - Mocked modules for isolated testing

### Coverage Targets

- **Overall**: Minimum 75% line coverage, 60% branch coverage
- **Core modules**: 85% line, 75% branch coverage
- **Executor**: 80% line, 70% branch coverage

### CI/CD Integration

```bash
# Full test suite with coverage
cargo llvm-cov --out Lcov -- --test-threads=1

# Branch coverage report
cargo llvm-cov report --fail-under-lines 75 --fail-under-branches 60

# Snapshot testing
cargo insta test --review

# Benchmarking
cargo bench --no-fail-fast
```

---

## Rust & Dependency Choices

### Core Dependencies

| Crate | Purpose | Alternative | Rationale |
|-------|---------|-------------|-----------|
| `serde` + `serde_yaml` | Serialization | `yaml-rust` | Standard, well-maintained |
| `anyhow` | Error handling | `thiserror` | Context preservation, simple errors |
| `tokio` | Async runtime | `async-std` | Wide ecosystem support |
| `minijinja` | Templates | `jinja2-rs`, PyO3 | Pure Rust, fast; PyO3 fallback |
| `regex` | Pattern matching | — | Standard |
| `clap` | CLI args | `structopt` | Modern builder API |
| `PyO3` | Python interop | `cpython` | Modern, well-maintained |
| `rstest` | Testing | — | Best-in-class fixtures |
| `cargo-llvm-cov` | Coverage | — | LLVM-based, reliable |
| `cargo-insta` | Snapshots | `similars` | Excellent UX, diff review |

### Compile Optimization

In `Cargo.toml`:
```toml
[profile.release]
opt-level = 3              # Maximum optimization
lto = true                 # Link-time optimization
codegen-units = 1         # Single codegen unit for better optimization
strip = true              # Strip symbols for smaller binary
```

---

## Compatibility Matrix

### Ansible Version Target
- **Start**: Ansible 2.14+
- **Expand to**: Full 2.x compatibility
- **Future**: 2.x to 3.x bridge

### Python Modules Compatibility
- **Phase 1**: Shell subprocess wrapper (100% compatible)
- **Phase 2**: Gradually migrate high-value modules to Rust
- **Strategy**: Measure adoption; rewrite only if >2x speedup

### Playbook Compatibility
- Supported: All standard playbooks (within Python module scope)
- Not initially supported: Dynamic inventory from Python (but shell support)

---

## Performance Goals

### Target Improvements (vs Python Ansible)

| Operation | Python Time | Target Rust | Speedup |
|-----------|------------|-------------|---------|
| Playbook parsing | 500ms | 50ms | 10x |
| Inventory loading | 300ms | 30ms | 10x |
| Task startup overhead | 200ms | 20ms | 10x |
| Variable resolution | 100ms | 10ms | 10x |
| Template rendering | 150ms | 15ms | 10x |
| File operations | 1000ms | 100ms | 10x |

### Benchmarking Strategy

```bash
# Compare ransible vs ansible on standard workloads
cargo bench --bench playbook_execution
cargo bench --bench inventory_loading
```

---

## Integration with Ansible Core

### Phase 1: Standalone Tools
- Parallel ransible-playbook for performance-critical workloads
- Independent test runner (ransible-test)

### Phase 2: Hybrid Approach
- Ansible CLI can invoke ransible-playbook for specific plays
- Module caching and precompilation

### Phase 3: Deep Integration
- Optimize hot paths in ansible-core with Rust via PyO3
- Template rendering backend switchable
- Module executor with optional Rust acceleration

---

## Key Design Decisions

1. **Subprocess Modules (Phase 1)**: Leverage existing Python modules to accelerate development
2. **minijinja by Default**: Balance speed and compatibility; PyO3 fallback for edge cases
3. **Modular Crates**: Each major component as separate crate for testability and reusability
4. **rstest for Fixtures**: Fixture-based testing enables DRY test code and parametrization
5. **Branch Coverage Priority**: Better test quality signals than line coverage
6. **Cargo-insta for Snapshots**: Easier review and maintenance of complex output expectations

---

## Risk Mitigation

| Risk | Mitigation |
|------|-----------|
| Jinja2 compatibility gaps | Start with minijinja, comprehensive test suite for edge cases, PyO3 fallback |
| Module API changes in Ansible | Pin to stable versions (2.14+), monitor upstream changes |
| Performance regressions | Continuous benchmarking, cargo-llvm-cov to detect dead code paths |
| Python interop complexity | Start small (shell modules only), expand gradually |
| Maintenance burden | Clear separation of concerns, comprehensive documentation |

---

## Success Metrics

- ✅ All standard Ansible playbooks execute successfully
- ✅ 10x speedup on core operations (parsing, variable resolution, task startup)
- ✅ 75% line coverage, 60% branch coverage minimum
- ✅ Drop-in replacement for `ansible-playbook` (different binary name for safety)
- ✅ Community adoption in performance-critical environments
- ✅ Integration opportunities with Ansible core identified

---

## Next Steps

1. **Read & Review**: PHASES.md for detailed development roadmap
2. **Reference**: TESTING_STRATEGY.md for testing architecture
3. **Map Modules**: MODULES_MAPPING.md for module prioritization
4. **Start Development**: Create Cargo workspace with phase 1 crates
5. **Establish CI/CD**: Set up coverage tracking and benchmarking
