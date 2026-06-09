# Ansiblers Testing Strategy

## Overview

Ansiblers employs a comprehensive, fixture-based testing strategy using modern Rust testing tools:

- **rstest**: Parametrized fixtures and mocking for unit tests
- **cargo-insta**: Snapshot testing for complex outputs
- **cargo-llvm-cov**: Branch coverage analysis
- **Fixture-based approach**: Reusable test playbooks, inventories, and modules

**Goal**: 75% line coverage, 60% branch coverage minimum, with emphasis on branch coverage for conditional logic.

---

## Test Organization

```
ansiblers/
├── crates/
│   ├── ansiblers-core/
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   └── context.rs
│   │   └── tests/              # Unit tests
│   │       └── test_*.rs
│   ├── ansiblers-parser/
│   │   └── tests/
│   │       ├── fixtures/       # Shared test files
│   │       │   ├── playbooks/
│   │       │   └── inventories/
│   │       └── test_*.rs
│   └── ...
├── tests/                      # Integration tests (crate-level)
│   ├── common/
│   │   ├── mod.rs             # Shared helpers
│   │   └── fixtures.rs        # Global fixtures
│   ├── fixtures/              # Test data
│   │   ├── playbooks/
│   │   ├── inventories/
│   │   ├── roles/
│   │   └── modules/
│   ├── integration/           # Integration test files
│   │   ├── test_playbook_execution.rs
│   │   ├── test_role_loading.rs
│   │   └── test_module_invocation.rs
│   └── snapshots/            # cargo-insta snapshots
└── reports/                  # Test & coverage reports
    ├── coverage/
    ├── benchmarks/
    └── test-results/
```

---

## Testing Layers

### 1. Unit Tests (In-Crate)

Each crate contains unit tests in `#[cfg(test)]` modules.

**Location**: `src/lib.rs` or dedicated `tests/` directory within crate

**Example**:
```rust
// In crates/ansiblers-vars/src/lib.rs

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[fixture]
    fn var_resolver() -> VariableResolver {
        VariableResolver::new()
    }

    #[rstest]
    #[case("{{ var }}", ["var" => "value"], "value")]
    #[case("{{ a }} + {{ b }}", ["a" => "1", "b" => "2"], "1 + 2")]
    fn test_simple_interpolation(
        #[from(var_resolver)] mut resolver: VariableResolver,
        #[case] template: &str,
        #[case] vars: &[(&str, &str)],
        #[case] expected: &str,
    ) {
        for (k, v) in vars {
            resolver.set(k, Value::String(v.to_string()));
        }
        let result = resolver.render(template).unwrap();
        assert_eq!(result, expected);
    }
}
```

### 2. Integration Tests

Test interactions between multiple crates at the feature level.

**Location**: `tests/integration/test_*.rs`

**Example**:
```rust
// tests/integration/test_playbook_execution.rs

mod common;

use ansiblers_playbook::Playbook;
use rstest::rstest;

#[rstest]
#[case("simple_shell.yml")]
#[case("with_variables.yml")]
#[case("with_blocks.yml")]
#[tokio::test]
async fn test_playbook_execution(
    #[case] playbook_file: &str,
    #[from(common::fixtures::execution_context)]
    ctx: ExecutionContext,
) {
    let playbook = Playbook::from_file(
        format!("tests/fixtures/playbooks/{}", playbook_file)
    ).unwrap();
    
    let result = playbook.execute(&ctx).await.unwrap();
    assert!(result.success);
}
```

### 3. Snapshot Tests

Use cargo-insta for complex outputs, AST structures, and formatted results.

**Location**: Same as integration tests; snapshots in `tests/snapshots/`

**Example**:
```rust
// tests/integration/test_playbook_parsing.rs

#[rstest]
#[case("complex_roles.yml")]
fn test_playbook_ast(#[case] playbook: &str) {
    let path = format!("tests/fixtures/playbooks/{}", playbook);
    let parsed = parse_playbook(&path).unwrap();
    
    insta::assert_json_snapshot!(parsed);
}
```

**Workflow**:
```bash
# First run creates snapshot
cargo insta test

# Review and approve snapshots
cargo insta review

# Snapshots stored in tests/snapshots/
```

---

## rstest: Fixtures & Parametrization

### Fixture Patterns

#### 1. Basic Fixtures
```rust
#[fixture]
fn execution_context() -> ExecutionContext {
    ExecutionContext::new(
        Arc::new(Inventory::default()),
        HashMap::new(),
    )
}

#[fixture]
fn test_inventory() -> Inventory {
    Inventory::from_file("tests/fixtures/inventories/test.ini").unwrap()
}
```

#### 2. Parametrized Fixtures
```rust
#[rstest]
#[case("192.168.1.1")]
#[case("example.com")]
#[case("localhost")]
fn test_host_resolution(#[case] host: &str) {
    let result = resolve_host(host);
    assert!(result.is_ok());
}
```

#### 3. Fixture Combinations
```rust
#[rstest]
#[case("web1", "webservers")]
#[case("db1", "databases")]
#[case("app1", "appservers")]
fn test_group_membership(
    #[from(test_inventory)] inventory: Inventory,
    #[case] hostname: &str,
    #[case] group: &str,
) {
    let host = inventory.get_host(hostname).unwrap();
    assert!(host.in_group(group));
}
```

#### 4. Parametrized Playbooks (via Fixture)
```rust
#[fixture]
#[once]  // Cached across tests
fn playbook_files() -> Vec<String> {
    let path = "tests/fixtures/playbooks/";
    std::fs::read_dir(path)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            if path.extension().map(|e| e == "yml").unwrap_or(false) {
                path.file_name().and_then(|n| n.to_str()).map(String::from)
            } else {
                None
            }
        })
        .collect()
}

#[rstest]
fn test_all_playbooks(
    #[from(playbook_files)] files: Vec<String>,
) {
    for file in files {
        let playbook = Playbook::from_file(
            format!("tests/fixtures/playbooks/{}", file)
        ).unwrap();
        // Verify basic parsing succeeds
        assert!(!playbook.plays.is_empty());
    }
}
```

### Mocking with rstest

```rust
use mockall::predicate::*;
use mockall::mock;

mock! {
    pub Connector {
        pub fn connect(&self, host: &str) -> Result<Connection>;
        pub fn execute(&self, cmd: &str) -> Result<String>;
    }
}

#[rstest]
fn test_task_with_mocked_connection(
    #[from(execution_context)] ctx: ExecutionContext,
) {
    let mut mock_connector = MockConnector::new();
    mock_connector
        .expect_execute()
        .with(eq("echo hello"))
        .times(1)
        .returning(|_| Ok("hello".to_string()));
    
    let task = Task::shell("echo hello");
    let result = task.execute("localhost", &mock_connector).unwrap();
    assert_eq!(result.stdout, "hello");
}
```

---

## Shared Test Fixtures

### Fixture Structure

```
tests/fixtures/
├── playbooks/
│   ├── simple_shell.yml           # Basic shell task
│   ├── with_variables.yml         # Variable substitution
│   ├── with_blocks.yml            # Block/rescue/always
│   ├── with_roles.yml             # Role inclusion
│   ├── with_handlers.yml          # Handler execution
│   ├── multi_host.yml             # Multiple hosts
│   ├── complex_roles.yml          # Complex role structure
│   └── performance_baseline.yml   # Benchmarking baseline
│
├── inventories/
│   ├── simple.ini                 # Basic INI format
│   ├── with_groups.ini            # Host groups
│   ├── with_vars.ini              # Group/host variables
│   ├── simple.yml                 # YAML format
│   ├── with_group_vars.yml        # group_vars structure
│   └── with_host_vars.yml         # host_vars structure
│
├── roles/
│   ├── simple_role/               # Basic role
│   │   ├── tasks/
│   │   │   └── main.yml
│   │   ├── vars/
│   │   │   └── main.yml
│   │   └── handlers/
│   │       └── main.yml
│   ├── with_dependencies/         # Role with dependencies
│   │   ├── meta/
│   │   │   └── main.yml
│   │   └── tasks/
│   │       └── main.yml
│   └── complex_role/              # Complex role
│       ├── defaults/
│       │   └── main.yml
│       ├── vars/
│       │   └── main.yml
│       ├── tasks/
│       │   ├── main.yml
│       │   └── subtasks.yml
│       ├── handlers/
│       │   └── main.yml
│       ├── templates/
│       │   └── config.j2
│       └── files/
│           └── script.sh
│
├── modules/
│   ├── mock_module.py            # Mock Python module
│   └── test_module.py            # Test module
│
└── group_vars/                    # Shared group variables
    ├── all.yml
    ├── webservers.yml
    └── databases.yml
```

### Common Fixture Helpers

```rust
// tests/common/fixtures.rs

use rstest::fixture;

#[fixture]
pub fn execution_context() -> ExecutionContext {
    ExecutionContext::new(
        Arc::new(test_inventory()),
        HashMap::from([
            ("var1".to_string(), Value::String("value1".to_string())),
        ]),
    )
}

#[fixture]
pub fn test_inventory() -> Inventory {
    Inventory::from_file("tests/fixtures/inventories/simple.yml")
        .expect("Failed to load test inventory")
}

#[fixture]
pub fn test_playbook(#[case] name: &str) -> Playbook {
    let path = format!("tests/fixtures/playbooks/{}.yml", name);
    Playbook::from_file(&path)
        .expect("Failed to load test playbook")
}

pub fn load_fixture_file(name: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/{}", name))
        .expect("Failed to load fixture file")
}
```

---

## Coverage Strategy

### cargo-llvm-cov Usage

```bash
# Full test suite with coverage
cargo llvm-cov --all --out Lcov

# Coverage report with branch coverage
cargo llvm-cov report --branches

# HTML report
cargo llvm-cov report --html

# Coverage thresholds
cargo llvm-cov report --fail-under-lines 75 --fail-under-branches 60

# Per-crate coverage
cargo llvm-cov --package ansiblers-executor report
```

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

### Branch Coverage Focus

**Why Branch Coverage?**: Line coverage misses conditional logic. Branch coverage identifies untested code paths:

```rust
// Line coverage: 100% (one line executed)
// Branch coverage: 50% (only true branch tested)
if condition {
    success_path();  // Tested
} else {
    error_path();    // Not tested
}
```

**Strategy**:
1. Identify high-branch-count functions
2. Parametrize tests to cover both branches
3. Use `cargo-llvm-cov --branches` in CI
4. Monthly branch coverage reports

---

## Snapshot Testing with cargo-insta

### When to Use Snapshots

✅ **Good Candidates**:
- Parsed AST structures
- Complex formatted output
- JSON serialization of objects
- Multi-line error messages

❌ **Poor Candidates**:
- Simple boolean assertions
- Numeric comparisons
- Performance-dependent values

### Snapshot Workflow

```bash
# Run tests (first time creates snapshots)
cargo test

# Review snapshots interactively
cargo insta review

# Accept changes
cargo insta accept

# Reject and revert
cargo insta reject
```

### Example Snapshots

```rust
// tests/integration/test_parser.rs

#[test]
fn snapshot_playbook_ast() {
    let playbook = Playbook::from_file(
        "tests/fixtures/playbooks/complex_roles.yml"
    ).unwrap();
    
    // Snapshots stored in tests/snapshots/
    insta::assert_json_snapshot!(playbook);
}

#[test]
fn snapshot_inventory_structure() {
    let inventory = Inventory::from_file(
        "tests/fixtures/inventories/with_vars.yml"
    ).unwrap();
    
    insta::assert_yaml_snapshot!(inventory);
}
```

---

## Performance Testing

### Benchmarking with Criterion

```rust
// benches/playbook_execution.rs

use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn benchmark_simple_playbook(c: &mut Criterion) {
    c.bench_function("parse_simple_playbook", |b| {
        b.iter(|| {
            Playbook::from_file(
                black_box("tests/fixtures/playbooks/simple_shell.yml")
            )
        })
    });
}

criterion_group!(benches, benchmark_simple_playbook);
criterion_main!(benches);
```

**Run**:
```bash
cargo bench --bench playbook_execution

# Output stored in target/criterion/
# Generate HTML reports
```

### Performance Targets

Track in `reports/benchmarks/`:
- Playbook parsing time
- Inventory loading time
- Variable resolution time
- Task startup overhead
- Template rendering time

---

## CI/CD Integration

### GitHub Actions Workflow

```yaml
# .github/workflows/test.yml

name: Tests & Coverage

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      
      - uses: dtolnay/rust-toolchain@stable
      
      - uses: taiki-e/install-action@cargo-llvm-cov
      
      - name: Run tests with coverage
        run: cargo llvm-cov --all --lcov --output-path lcov.info
      
      - name: Upload coverage to Codecov
        uses: codecov/codecov-action@v3
        with:
          files: ./lcov.info
          flags: rust
          fail_ci_if_error: true
      
      - name: Check branch coverage
        run: cargo llvm-cov report --fail-under-branches 60
      
      - name: Generate coverage report
        run: cargo llvm-cov report --html
      
      - name: Upload coverage report
        uses: actions/upload-artifact@v3
        with:
          name: coverage-report
          path: target/llvm-cov/html/

  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo fmt -- --check
      - run: cargo clippy --all-targets -- -D warnings
```

---

## Test Data Management

### Version Control

✅ Store in Git:
- `tests/fixtures/playbooks/*.yml` (test playbooks)
- `tests/fixtures/inventories/*.yml` (test inventories)
- `tests/snapshots/*` (insta snapshots)

❌ Generate/Don't Store:
- `target/llvm-cov/` (coverage reports)
- `reports/benchmarks/*.json` (benchmark results)

### Fixture Updates

When Ansible updates:
1. Update affected fixtures in `tests/fixtures/`
2. Re-generate snapshots: `cargo insta test && cargo insta accept`
3. Commit fixture changes to Git
4. Document breaking changes in PR

---

## Testing Best Practices

1. **Use rstest for Parametrization**
   - Reduces test code duplication
   - Enables systematic testing of variations

2. **Leverage Fixtures**
   - Shared fixtures reduce setup code
   - Fixtures are composable and cacheable

3. **Name Tests Clearly**
   ```rust
   #[test]
   fn test_variable_resolution_with_nested_dict() { }  // ✓ Clear
   
   #[test]
   fn test_vars() { }  // ✗ Vague
   ```

4. **One Assertion Per Test (When Possible)**
   - Faster to identify failure cause
   - Snapshot tests can assert multiple aspects

5. **Use `#[tokio::test]` for Async**
   ```rust
   #[rstest]
   #[tokio::test]
   async fn test_async_playbook_execution(
       #[from(execution_context)] ctx: ExecutionContext,
   ) {
       // async test code
   }
   ```

6. **Group Related Tests**
   ```rust
   mod variable_resolution {
       use super::*;
       use rstest::rstest;
       
       #[rstest]
       fn test_simple_interpolation() { }
       
       #[rstest]
       fn test_nested_dict_access() { }
   }
   ```

7. **Test Edge Cases**
   ```rust
   #[rstest]
   #[case("")]                      // Empty string
   #[case("{{ }}"]]                 // Empty template
   #[case("{{ undefined_var }}"]]  // Missing variable
   #[case("{{ var | unknown_filter }}"]] // Unknown filter
   fn test_template_edge_cases(#[case] template: &str) { }
   ```

---

## Maintaining Test Quality

### Monthly Review

- Analyze branch coverage reports
- Identify uncovered conditional branches
- Add parametrized tests for missing cases
- Update coverage targets if needed

### Regression Prevention

- Run full test suite before commits
- Keep snapshots up-to-date in Git
- Document why complex mocks exist

### Performance Regression Detection

```bash
# Store baseline benchmark results
cargo bench > reports/benchmarks/baseline.txt

# Compare against baseline on each run
cargo bench > reports/benchmarks/current.txt
diff reports/benchmarks/{baseline,current}.txt
```

---

## Test Coverage Reports

Store in `reports/`:

```
reports/
├── coverage/
│   ├── coverage-YYYY-MM-DD.html
│   └── latest -> coverage-YYYY-MM-DD.html
├── benchmarks/
│   ├── playbook_execution-YYYY-MM-DD.json
│   └── results.csv
└── test-results/
    └── test-run-YYYY-MM-DD.xml
```

**Tracking**:
- Monthly coverage trends
- Performance regression detection
- Test execution time tracking
