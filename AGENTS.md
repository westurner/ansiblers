# AGENTS.md

This file provides guidance to Claude Code (claude.ai/code) and other compatible agentic tools when working with code in this repository.

**Note:** This file is for AI assistant use only. For human developers, see the design documentation in `docs/design/` (start with [README.md](docs/design/README.md)).

## ⚠️ IMPORTANT: Always Start Here

**BEFORE starting any development or code review task:**

1. **Read this file first** - Don't work from memory or assumptions
2. **Review relevant design docs** - See `docs/design/` for architecture and phase details
3. **Use manage_todo_list** - Create and track progress systematically
4. **Follow the patterns** - Reference Quick Reference for correct commands and conventions
5. **Check the current phase** - Different phases have different priorities

## ⚠️ CRITICAL: Licensing Requirements

**NEVER suggest, recommend, or approve code that violates these requirements:**

- **ansiblers (all code)**: Must be **GPLv3 compatible**
- **External dependencies**: Only recommend crates compatible with GPLv3
- **PR reviews**: Always verify new dependencies are license-compatible
- **When in doubt**: Ask about licensing compatibility rather than assuming

**This is non-negotiable** - licensing violations can create serious legal issues for the project.

## Quick Reference

Most commonly used commands and patterns for ansiblers development:

```bash
# Testing
cargo test --all                                          # Run all unit tests
cargo llvm-cov --all --lcov                              # Test with coverage (branch coverage)
cargo llvm-cov report --html                             # Generate HTML coverage report
cargo test --all -- --nocapture --test-threads=1        # Debug tests with output

# Linting & Formatting
cargo fmt --all -- --check                               # Check formatting
cargo fmt --all                                          # Auto-format all code
cargo clippy --all-targets -- -D warnings                # Lint check (deny warnings)

# Building
cargo build --release                                    # Release build with O3/LTO
cargo build --profile release                            # Same as above
cargo check                                              # Fast syntax check

# Testing Snapshots (cargo-insta)
cargo insta test                                         # Run tests, create/update snapshots
cargo insta review                                       # Review pending snapshot changes
cargo insta accept                                       # Accept snapshot changes

# Benchmarking
cargo bench --all                                        # Run all benchmarks
cargo bench --bench playbook_execution                  # Run specific benchmark

# Documentation
cargo doc --all --no-deps --open                         # Generate & view docs
cargo test --doc                                         # Run doc tests

# Workspace Info
cargo tree                                               # Show dependency tree
cargo metadata --format-version 1                        # Get workspace metadata
```

**Container/Docker** (for integration tests):

```bash
docker build -f Dockerfile.test -t ansiblers-test .    # Build test container
docker run --rm -v $(pwd):/work ansiblers-test cargo test
```

**Critical Reminders:**

- **Licensing**: GPLv3 compatible only - check new dependencies
- **Coverage**: 75% line, 60% branch minimum (checked in CI)
- **No trailing whitespace**: Ansible convention; enforced in PRs
- **Line limit**: 160 characters (standard Rust is 100, but we match Ansible)
- **rstest fixtures**: Use for parametrization and test data reuse

## Development Environment Setup

### Prerequisites

```bash
# Rust toolchain (1.70+)
rustup update
rustup component add rustfmt clippy
rustup toolchain install stable

# Coverage tools
cargo install cargo-llvm-cov
cargo install cargo-insta

# Optional: Benchmarking and profiling
cargo install cargo-criterion
cargo install cargo-flamegraph

# Docker/Podman (for integration tests)
docker --version || podman --version

# Python 3.10+ (for Ansible compatibility testing)
python3 --version
```

### Project Setup

After cloning:

```bash
cd ansiblers/src/ansiblers

# Verify workspace
cargo check --all

# Run tests to verify setup
cargo test --all -- --nocapture

# Generate docs
cargo doc --all --no-deps --open

# Check code formatting
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
```

### Useful VS Code Extensions (Optional)

- **rust-analyzer**: IntelliSense, navigation, refactoring
- **CodeLLDB**: Debugging Rust code
- **Cargo**: Task integration
- **crates**: Dependency management UI
- **Even Better TOML**: Cargo.toml syntax highlighting

### Workspace Structure

```
ansiblers/src/ansiblers/
├── crates/                          # Individual workspace crates
│   ├── ansiblers-core/              # Core data structures
│   ├── ansiblers-parser/            # Playbook parsing
│   ├── ansiblers-inventory/         # Inventory management
│   ├── ansiblers-vars/              # Variable resolution
│   ├── ansiblers-templates/         # Jinja2 rendering
│   ├── ansiblers-executor/          # Task execution
│   ├── ansiblers-modules/           # Module invocation
│   ├── ansiblers-playbook/          # Binary: ransible-playbook
│   └── ansiblers-test/              # Binary: ransible-test (Phase 4+)
├── tests/                           # Integration tests
│   ├── fixtures/                    # Test data (playbooks, inventories, roles)
│   ├── integration/                 # Integration test files
│   ├── common/                      # Shared test utilities
│   └── snapshots/                   # cargo-insta snapshots
├── reports/                         # Generated reports
│   ├── coverage/                    # Coverage reports
│   └── benchmarks/                  # Benchmark results
├── docs/design/                     # Architecture & planning docs
└── Cargo.toml                       # Workspace manifest
```

**Note**: ansiblers-core and all CLIs require a POSIX OS. On Windows, use WSL (Windows Subsystem for Linux).

## Testing and CI

### Basic Testing Commands

```bash
# Run unit tests (all crates)
cargo test --all

# Run tests in specific crate
cargo test -p ansiblers-executor

# Run tests with output captured
cargo test --all -- --nocapture

# Run single test
cargo test test_playbook_execution -- --exact

# Run tests matching pattern
cargo test variable_resolution

# Run with single thread (deterministic, slower)
cargo test --all -- --test-threads=1

# Doc tests
cargo test --doc
```

### Coverage Analysis

```bash
# Generate coverage report with branch coverage
cargo llvm-cov --all --lcov --output-path lcov.info

# View HTML coverage report
cargo llvm-cov report --html
open target/llvm-cov/html/index.html

# Check coverage thresholds
cargo llvm-cov report --fail-under-lines 75 --fail-under-branches 60

# Per-crate coverage
cargo llvm-cov --package ansiblers-core report --html

# Coverage with specific tests
cargo llvm-cov --all --test integration_tests --lcov
```

### Snapshot Testing (cargo-insta)

```bash
# Run tests and create/review snapshots
cargo insta test

# Review all pending snapshot changes interactively
cargo insta review

# Accept all pending changes
cargo insta accept

# Reject changes and revert
cargo insta reject

# Review specific snapshot
cargo insta review --snapshot test_name
```

### Benchmarking

```bash
# Run all benchmarks
cargo bench --all

# Run specific benchmark
cargo bench --bench playbook_execution

# Store baseline for comparison
cargo bench --all -- --save-baseline main

# Compare against baseline
cargo bench --all -- --baseline main

# With profiling (flamegraph)
cargo flamegraph --bench playbook_execution
```

### Test Organization

Tests are organized using **rstest fixtures** for parametrization and reusability:

```bash
# Test structure
tests/
├── fixtures/                    # Test data
│   ├── playbooks/              # .yml test playbooks
│   ├── inventories/            # .ini/.yml test inventories
│   └── roles/                  # Test role structures
├── common/                     # Shared test utilities
│   ├── mod.rs
│   └── fixtures.rs             # rstest @fixture definitions
├── integration/                # Integration tests
│   ├── test_playbook_execution.rs
│   ├── test_module_invocation.rs
│   └── test_role_loading.rs
└── snapshots/                  # Insta snapshots (auto-generated)
```

**Test Fixtures Best Practices**:

1. **Use rstest @fixture for reusable setup**:
   ```rust
   #[fixture]
   fn execution_context() -> ExecutionContext { ... }
   ```

2. **Parametrize tests with #[case]**:
   ```rust
   #[rstest]
   #[case("simple_playbook.yml")]
   #[case("complex_playbook.yml")]
   fn test_execution(#[case] playbook: &str) { ... }
   ```

3. **Create fixture playbooks in tests/fixtures/playbooks/**

4. **Use cargo-insta for complex output validation**

### Coverage Targets by Component

| Component | Line Coverage | Branch Coverage | Priority |
|-----------|---------------|-----------------|----------|
| ansiblers-core | 85% | 75% | Critical |
| ansiblers-executor | 80% | 70% | Critical |
| ansiblers-parser | 80% | 60% | High |
| ansiblers-inventory | 85% | 70% | High |
| ansiblers-vars | 85% | 75% | High |
| ansiblers-templates | 80% | 70% | High |
| ansiblers-modules | 75% | 60% | Medium |
| ansiblers-playbook | 70% | 50% | Low |

**Minimum across all crates**: 75% line, 60% branch

### CI/CD Pipeline

CI/CD runs on every push and PR:

- ✅ `cargo fmt --all -- --check` (formatting)
- ✅ `cargo clippy --all-targets -- -D warnings` (linting)
- ✅ `cargo test --all` (unit tests)
- ✅ `cargo llvm-cov --all --lcov` (coverage with branch analysis)
- ✅ Benchmark tracking (performance regression detection)
- ✅ Coverage report uploaded to Codecov
- ✅ HTML coverage report as artifact

See `.github/workflows/test.yml` for full CI configuration.

## PR Review & Development Guidelines

### Code Review Checklist

Use this checklist for EVERY PR and code review:

```text
□ Created todo list for review/development steps
□ Code follows Rust idioms and conventions
□ Tests added/updated with rstest parametrization where applicable
□ Snapshot tests reviewed and committed (cargo-insta)
□ Coverage maintained: 75% line, 60% branch minimum
□ No clippy warnings: cargo clippy --all-targets -- -D warnings
□ Formatting correct: cargo fmt --all -- --check
□ Documentation updated (code comments, doc strings, README)
□ Async code reviewed for safety (if applicable)
□ No unsafe code without justification in comments
□ Dependencies reviewed for GPLv3 compatibility
□ Performance impact considered (benchmarks if relevant)
□ Mark each item completed when verified
```

### PR Requirements

- **Coverage**: All code must have tests; 75% line + 60% branch minimum
- **Fixtures**: Use rstest @fixture for reusable test setup
- **Snapshots**: Complex outputs validated with cargo-insta
- **Documentation**: Update docs/design/ if architecture changes
- **Formatting**: Must pass `cargo fmt --all -- --check`
- **Linting**: Must pass `cargo clippy --all-targets -- -D warnings`

### Code Review Process

Follow these steps for thorough reviews:

1. **Get branch context**: Understand the feature/bug from description
2. **Check coverage**: Verify test coverage and snapshot files
3. **Review code changes**: Read implementation against architecture patterns
4. **Run tests locally**: `cargo test --all && cargo llvm-cov report`
5. **Review documentation**: Check for updated docs/design/* files
6. **Check dependencies**: Verify new crates are GPLv3 compatible
7. **Provide feedback**: Specific examples for requested changes
8. **Approve when ready**: All items checked off checklist above

### Common Review Issues to Check

- **Coverage gaps**: Use `cargo llvm-cov report --html` to identify uncovered branches
- **Test isolation**: Ensure tests don't depend on execution order
- **Fixture reuse**: Parametrize tests with `#[rstest]` instead of duplicating
- **Snapshot maintenance**: Review `cargo insta` diffs carefully before accept
- **Performance**: Consider impact of changes on benchmarks
- **Thread safety**: Check for data races with `cargo clippy` and `cargo miri`
- **Error handling**: Explicit error messages using `anyhow::Context`

### Review Tools

- `cargo test --all` - Run all tests
- `cargo llvm-cov report --html` - Coverage analysis with branch details
- `cargo clippy --all-targets -- -D warnings` - Lint check
- `cargo fmt --all -- --check` - Formatting verification
- `cargo bench --all -- --baseline main` - Performance regression detection

## Development Guidelines

### Code Style Notes

- **Edition**: Rust 2021+, MSRV 1.70+
- **Line limit**: 160 characters (matching Ansible conventions, not standard Rust 100)
- **Comments**: Explain "why", not "what" - code should be self-documenting
- **Documentation**: Use doc comments (`///`) for public APIs
- **Error handling**: Use `anyhow::Context` for error messages with context
- **No trailing whitespace**: Ansible convention; enforced in CI
- **Formatting**: Must pass `cargo fmt --all`
- **Linting**: Must pass `cargo clippy --all-targets -- -D warnings`
- **No unsafe code**: Avoid unless necessary; always add `// SAFETY: ...` comment explaining why
- **Type hints**: Use Rust's type system fully; no unnecessary `_` placeholders

### Module Development Guidelines

When implementing modules (in ansiblers-modules):

1. **Trait Implementation**:
   ```rust
   pub struct MyModule;
   
   impl ModuleInvoker for MyModule {
       fn invoke(&self, task: &Task, host: &str, ctx: &ExecutionContext) 
           -> Result<TaskResult> 
       { ... }
   }
   ```

2. **Error Handling**: Use `anyhow::Result<T>` with context:
   ```rust
   let value = task.args.get("key")
       .ok_or_else(|| anyhow!("module requires 'key' parameter"))?;
   ```

3. **Test Coverage**: Minimum 75% line, 60% branch
   ```rust
   #[rstest]
   #[case("success_scenario")]
   #[case("error_scenario")]
   fn test_module(#[case] scenario: &str) { ... }
   ```

4. **Snapshots for Output**: Use `insta::assert_json_snapshot!(result);`

5. **Performance**: Include benchmark if module is performance-critical
   ```bash
   cargo bench --bench module_name
   ```

### Async Code Guidelines

When using `tokio` async runtime:

1. **Use `#[tokio::test]` for async tests**
2. **Avoid `.unwrap()` in async code** - use `?` operator
3. **Spawn tasks carefully** - ensure proper error handling
4. **Test cancellation** - verify cleanup happens on drop

### Dependency Guidelines

- **Prefer stdlib** over external crates when possible
- **GPLv3 compatibility required** - check license before adding
- **Performance cost** - consider compile time vs runtime benefit
- **Maintenance burden** - prefer well-maintained crates with active updates
- **Transitive deps** - understand full dependency tree with `cargo tree`

**Recommended crates** (already approved):
- `serde` + `serde_yaml` - Serialization
- `anyhow` - Error handling
- `tokio` - Async runtime
- `minijinja` - Template rendering
- `regex` - Pattern matching
- `clap` - CLI arguments
- `rstest` - Testing fixtures
- `cargo-insta` - Snapshots
- `cargo-llvm-cov` - Coverage

**Request approval before adding**:
- External cryptographic crates
- Networking crates beyond `tokio`
- Unsafe crates or platform-specific code

## Documentation Standards

### Rust Documentation

All public APIs must have doc comments:

```rust
/// Executes a playbook against the given inventory.
///
/// # Arguments
///
/// * `playbook` - The playbook to execute
/// * `context` - The execution context with inventory and variables
///
/// # Returns
///
/// A `PlaybookResult` containing all task results
///
/// # Errors
///
/// Returns an error if playbook parsing fails or execution is interrupted
pub fn execute_playbook(playbook: &Playbook, context: &ExecutionContext) 
    -> Result<PlaybookResult>
{
    // Implementation
}
```

### Design Documentation

All architectural changes require documentation:

- **File location**: `docs/design/`
- **Format**: Markdown
- **Audience**: Technical (developers, architects)
- **Content**: Design rationale, alternatives considered, trade-offs

Update relevant design docs when:
- Adding new crates or modules
- Changing core data structures
- Modifying execution flow or task handling
- Changing module invocation strategy

### Code Comments

- **Explain why**: Use comments to explain design decisions, not what the code does
- **Justify unsafe**: Always add `// SAFETY: ...` comment for unsafe blocks
- **Document complexity**: Add comments for non-obvious algorithms or patterns
- **Avoid obvious**: Don't comment on straightforward code (`x = x + 1`)

### Module Documentation

For each module implementation in ansiblers-modules:

1. **Module struct documentation**: Describe what the module does
2. **Function documentation**: Document parameters and return values
3. **Example usage**: Show how the module is invoked
4. **Test fixtures**: Create test playbooks in tests/fixtures/playbooks/

Example:
```rust
/// The command module executes arbitrary shell commands.
///
/// Supported parameters:
/// - `_raw_params`: The command to execute (required)
/// - `chdir`: Change directory before execution
/// - `creates`: Skip if this file exists
/// - `removes`: Skip if this file doesn't exist
pub struct CommandModule;
```

## Repository Management

### Development Phases

Ansiblers development follows a phased approach. Check `docs/design/PHASES.md` for detailed roadmap.

**Current Phase**: Check project board or README.md for active phase.

- **Phase 1** (Weeks 1-8): Foundation - core data structures, basic parsing, shell module
- **Phase 2** (Weeks 9-14): Core module support - Python wrapper, essential modules
- **Phase 3** (Weeks 15-18): Role and inventory support
- **Phase 4** (Weeks 19-22): ransible-test implementation
- **Phase 5** (Weeks 23-32): Module rewrites (apt, yum, git, template, setup)
- **Phase 6** (Weeks 33+): Optimization and Ansible core integration

### Branch and Release Management

- **Development**: Work on feature branches
- **Integration**: PR to `main` for code review
- **Testing**: CI/CD runs on all PRs
- **Release**: Version tags follow semantic versioning (v0.1.0, etc.)

### PR Workflow

1. Create feature branch: `git checkout -b feature/my-feature`
2. Implement feature with tests
3. Run: `cargo test --all && cargo llvm-cov report`
4. Commit with clear message: `feat: add variable resolution for dicts`
5. Push and create PR against `main`
6. Address review feedback
7. Maintainer merges when CI is green and reviews approved

### Code Structure Reference

- `crates/ansiblers-core/` - Core types (ExecutionContext, TaskResult, etc.)
- `crates/ansiblers-parser/` - YAML/playbook parsing
- `crates/ansiblers-inventory/` - Inventory loading and management
- `crates/ansiblers-vars/` - Variable resolution engine
- `crates/ansiblers-templates/` - Jinja2 template rendering
- `crates/ansiblers-executor/` - Task execution engine
- `crates/ansiblers-modules/` - Module invocation (Python wrapper + Rust natives)
- `crates/ansiblers-playbook/` - ransible-playbook binary
- `crates/ansiblers-test/` - ransible-test binary (Phase 4+)
- `tests/` - Integration tests with fixtures
- `docs/design/` - Architecture and planning documentation

### Key Design Patterns

**ModuleRegistry**: Dynamic module loading
```rust
pub struct ModuleRegistry {
    modules: HashMap<String, Arc<dyn ModuleInvoker>>,
    default_invoker: Arc<dyn ModuleInvoker>,
}
```

**ExecutionContext**: Global state during playbook execution
```rust
pub struct ExecutionContext {
    pub inventory: Arc<Inventory>,
    pub vars: HashMap<String, Value>,
    pub facts: HashMap<String, HashMap<String, Value>>,
}
```

**rstest Fixtures**: Reusable test setup
```rust
#[fixture]
fn execution_context() -> ExecutionContext { ... }
```

**Snapshot Testing**: Validate complex outputs
```rust
#[test]
fn test_playbook_ast() {
    let parsed = parse_playbook("test.yml").unwrap();
    insta::assert_json_snapshot!(parsed);
}
```
