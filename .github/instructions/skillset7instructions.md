
- Develop plans in docs/design/ as markdown documents.
- Instead of /tmp, always use tmp/ as a tmpdir.
- Store test run outputs in reports/ for reference and re-reference.
- Use test coverage to target test case development.
  If possible, use *branch* coverage.
  - rust/cargo: Use cargo-llvm-cov to improve test coverage by targeting lines and or branches.
  - python: Use pytest-cov with branch coverage
- Use fixtures, parametrization, and mocks to write good tests.