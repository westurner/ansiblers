# Ansiblers Development Phases - Detailed Roadmap

## Phase 1: Foundation & Proof of Concept (Weeks 1-8) ✅ COMPLETE

### Goals
- ✅ Establish Rust workspace structure
- ✅ Parse basic Ansible playbooks
- ✅ Load inventories with variables
- ✅ Execute simple shell tasks
- ✅ Verify output compatibility with Ansible

### Milestones

#### Week 1-2: Project Setup
- [x] Create Cargo workspace with initial crates
- [x] Set up CI/CD with GitHub Actions
- [x] Configure cargo-llvm-cov for coverage tracking
- [x] Create test fixture directories
- [x] Define coding standards and design patterns

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
- [x] Define ExecutionContext (inventory, vars, facts, connections)
- [x] Define TaskResult with all Ansible-compatible fields
- [x] Define HostState (facts, var overrides)
- [x] Error types and handling strategy
- [x] Initial unit tests with rstest fixtures

#### Week 3-4: Playbook Parser (ansiblers-parser)
- [x] YAML parsing with serde_yaml
- [x] Playbook AST structure
- [x] Play and Task representation
- [x] Handler support
- [x] Block/rescue/always parsing
- [ ] Include/import directive parsing (deferred to Phase 3)

**Test Coverage Target**: 80% line coverage

#### Week 4-5: Inventory System (ansiblers-inventory)
- [x] INI format parsing
- [x] Host and group management
- [x] Variable merging (inventory vars)
- [x] Host/group fact storage
- [x] Inventory validation (basic)

#### Week 5-6: Variable Resolution (ansiblers-vars)
- [x] Precedence implementation (9-tier VarScope)
- [x] Variable interpolation in strings
- [x] Jinja2 template context preparation
- [x] Fact caching per-host
- [x] Variable merging strategies

#### Week 6-7: Template Rendering (ansiblers-templates)
- [x] jinja2rs/minijinja integration
- [x] Ansible filter implementation (combine, regex_*, to_nice_json, quote, from_json, from_yaml, path_join)
- [x] Variable substitution in task parameters
- [x] Conditional expression evaluation
- [x] Error handling for invalid templates

#### Week 7-8: Shell Module & Executor (ansiblers-executor, ansiblers-modules)
- [x] Shell module native implementation (with creates/removes/chdir)
- [x] Command module native implementation
- [x] debug, set_fact, fail modules
- [x] Task parameter resolution
- [x] When condition evaluation
- [x] Register variable capture
- [x] Loop (`loop:` / `with_items:`) execution
- [x] Block/rescue/always execution

#### Week 8: Integration & Polish
- [x] ransible-playbook binary with basic CLI args
- [x] Output formatting (human, JSON)
- [x] Exit code compatibility
- [x] CI/CD pipeline validation

### Success Criteria for Phase 1

- ✅ Execute basic playbook with shell tasks
- ✅ Variable substitution works end-to-end
- ✅ Output matches Ansible formatting
- ✅ Block/rescue/always fully functional
- ✅ Loop execution working
- ✅ 83 tests passing (unit + integration), 0 failures
- ✅ Zero unsafe code
- ✅ Snapshot tests with cargo-insta

---

## Phase 2: Core Module Support, Molecule & Sandboxing (Weeks 9-14) ✅ COMPLETE

### Goals
- ✅ Develop `ansiblers-molecule` early for multi-node test isolation and parametrization
- ⏳ Cross-compile "Rustball" transit payloads (`amd64` / `musl` / `wasm32-wasi`) — deferred to Phase 5
- ✅ Essential modules in Rust (file, copy, stat, debug, set_fact)
- ✅ Execute modules in Preview Mode OS sandboxes (`bubblewrap`, `crun`, `podman`)
- ✅ Full task control flow (blocks, handlers)
- ✅ Multi-host execution coordination (linear + free strategies)
- ✅ PyO3 PlaybookRunner bindings (`ansiblers-compat`)

### Milestones

#### Week 9: ansiblers-molecule & Multi-Node Testing
- [x] Develop `ansiblers-molecule` driver lifecycle states (Create, Converge, Verify, Destroy)
- [x] Docker/Podman container creation/cleanup, networking, and volume isolation for multi-node tests
- [x] NoneDriver for CI testing without containers

#### Week 10: PyO3 Bindings & Python Module Wrapper (ansiblers-modules, ansiblers-compat)
- [ ] Implement `build.rs` to cross-compile modules to `x86_64-unknown-linux-musl` and `wasm32-wasi` (deferred)
- [ ] Zero-extraction mechanics via `artifact-fs` FUSE-mounts (deferred to Phase 5)
- [x] Expose PlaybookRunner + InventoryLoader through PyO3 (`ansiblers-compat`)
- [x] Module argument JSON bridging & environment setup (`PythonModuleWrapper`)
- [x] `AnsiblePythonModuleInvoker` for broad Python module compatibility

#### Week 11: High-Value Modules & Preview Sandbox
- [x] Implement `file`, `copy`, `stat` modules in Rust.
- [x] `PreviewModeWrapper` wrapping any `ModuleInvoker` with bubblewrap sandbox.
- [x] OverlayFS diff collection (changed paths → `_diff` variable).
- [x] Fallback to direct execution when `bwrap` is unavailable.

#### Week 11-12: Task Control Flow
- [x] Block support with nested tasks (Phase 1)
- [x] Rescue block execution (Phase 1)
- [x] Always block guarantee (Phase 1)
- [x] Handler registration and execution (Phase 1)
- [x] Multi-host block execution via `execute_block_single_host`

#### Week 12-13: Multi-Host Execution
- [x] Parallel task execution via `Strategy::Free` (thread-per-host)
- [x] Linear strategy (default): task-by-task across all hosts
- [x] Fact gathering per-host (`set_fact` + context)
- [x] Register variable isolation per-host
- [x] Failure handling (fail-fast via `HostState::failed`)

#### Week 13-14: Integration & Testing
- [x] Integration test suite with Phase 2 fixtures (`file_operations`, `copy_operations`, `multi_host`)
- [x] Molecule lifecycle integration tests (None driver)
- [x] Python module wrapper implementation
- [x] 120 tests passing (unit + integration), 0 failures

### Deliverables
- ✅ `ansiblers-molecule` crate with Docker/Podman/None drivers
- ✅ `ansiblers-compat` crate: PyO3 PlaybookRunner + InventoryLoader
- ✅ `file`, `copy`, `stat` Rust-native modules
- ✅ `PythonModuleWrapper` and `AnsiblePythonModuleInvoker`
- ✅ `PreviewModeWrapper` with bwrap + OverlayFS diff
- ✅ Linear + Free multi-host strategies
- ✅ `ansiblers-sandbox` crate: three-layer defense-in-depth security
  - Layer 1: `jinja2rs::SandboxedEnvironment` + `TrustLevel` / `TemplateEngineConfig` config
  - Layer 2: `SandboxedModuleRegistry` — bwrap + seccomp BPF injection + OverlayFS diff
  - Layer 3: `apply_process_seccomp` — coordinator deny-list + strict allow-list profiles
- ✅ `ConfigurablePythonInvoker` with Subprocess / Native (PyO3) / SubInterpreter backends
- ✅ Inline `//!` crate docs + `///` item docs across all crates (doc tests pass)
- ✅ 173 tests, 0 failures

---

## Phase 3: Inventory & Role Support (Weeks 15-18) ✅ COMPLETE

### Goals
- ✅ Full inventory format support (YAML) — completed in Phase 1
- ✅ `group_vars` / `host_vars` directory loading
- ✅ Role loading and execution (`ansiblers-roles` crate)
- ✅ Role dependencies via `meta/main.yml` with cycle detection
- ✅ Basic ansible-galaxy `requirements.yml` support
- ✅ Dynamic inventory (JSON `--list` format)

### Milestones

#### Week 15: YAML Inventory & Variables
- [x] YAML inventory format parsing (Phase 1)
- [x] `group_vars/` directory loading (all.yml, <group>.yml, split-file dirs)
- [x] `host_vars/` directory loading (single file and split-file dirs)
- [x] Variable merging and precedence
- [x] Dynamic inventory stub — `DynamicInventoryScript` + `parse_list_output`

#### Week 16: Role Loading & Execution
- [x] Role directory structure validation and loading
- [x] `tasks/`, `handlers/`, `vars/`, `defaults/` loading
- [x] Role variable precedence (defaults < play_vars < role_vars)
- [x] `RoleLoader` with configurable `RolePath` (mirrors Ansible search order)
- [x] `meta/main.yml` parsing (`RoleMeta`, `GalaxyInfo`, `RoleDependency`)

#### Week 17: Role Dependencies & Galaxy
- [x] `meta/main.yml` dependency parsing (simple and full spec forms)
- [x] Recursive role dependency resolution with topological ordering
- [x] Cycle detection in dependency graphs
- [x] Deduplication (shared deps loaded only once)
- [x] `requirements.yml` parsing (`GalaxyRequirements`, `RoleRequirement`, `CollectionRequirement`)
- [ ] Role path configuration via `ansible.cfg` (deferred to Phase 5)
- [ ] `ansible-galaxy install` integration (deferred to Phase 5)
- [ ] Cargo.toml metadata schema for ansiblers roles (deferred)

#### Week 18: Integration & Validation
- [x] Integration test suite with Phase 3 fixtures
- [x] `group_vars/` and `host_vars/` fixture files
- [x] `roles/common/` and `roles/webserver/` fixture roles with dependencies
- [x] `requirements.yml` fixture

### Deliverables
- ✅ `ansiblers-inventory`: `group_host_vars` and `dynamic` modules
- ✅ `ansiblers-roles` crate: `RoleLoader`, `RolePath`, `RoleMeta`, `DependencyGraph`, `GalaxyRequirements`
- ✅ 222 tests, 0 failures

---

## Phase 4: Artifact Generation & Testing (Weeks 19-22) ✅ COMPLETE

### Goals
- ✅ `ansiblers-build` target artifact generation
- ✅ `ansible-test` compatible test runner
- Container management for test isolation

### Milestones

#### Week 19-20: ansiblers-build & Artifact generation
- [x] Implement Multi-Stage Dockerfile Builder with BuildKit caching.
- [x] Integrate `c2w` (container2wasm) outputs.
- [x] Scaffold `repo2jupyterlite` static sites for decentralized WASM playbooks.

#### Week 20-21: Test Execution & CI Integration
- [x] Test target directory structure parsing and classification
- [x] Sanity test coordination and unit test aggregation
- [x] Advanced result reporting and evaluation

#### Week 21-22: Coverage & Reporting
- [x] cargo-llvm-cov integration
- [x] HTML report generation
- [x] Coverage thresholds
- [x] CI reporting

### Deliverables
- ✅ `ansiblers-build` crate: `DockerfileBuilder` (multi-stage + BuildKit cache mounts), `C2wConfig` + `C2wBatch` (container2wasm), `JupyterLiteScaffold` (static site skeleton)
- ✅ `ansiblers-test` crate: `ransible-test` binary with `sanity` / `units` / `integration` / `coverage` subcommands; `TestRunner`, `CoverageConfig`, `TestReport` / `CheckResult` types; JSON output
- ✅ Integration test suite: `test_phase4_build` covering Dockerfile, c2w, and JupyterLite
- ✅ 272 tests, 0 failures

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

## Phase 6: Zero-Trust WebRTC & Performance Optimization (Weeks 33+)

### Goals
- Production-ready, zero-trust WebRTC execution layer
- Decentralized signaling and PQ cryptography integration
- Integration opportunities with Ansible core
- Documentation and adoption guide

### Milestones

#### Weeks 33-34: Pluggable Transport & WebRTC
- [ ] Abstract connection providers (`ConnectionProvider` trait)
- [ ] WebRTC Data Channel implementation (`WebRtcPqConnection`)
- [ ] X25519MLKEM768 post-quantum cryptographic handshakes
- [ ] W3C DID document resolution & payload signing (ML-DSA)
- [ ] Embedded (masterless mesh) and external Signaling providers

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
