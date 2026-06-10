# Ansiblers Dev Container

This directory contains the development container configuration for Ansiblers (Rust implementation of Ansible).

## Quick Start

### Option 1: VS Code Dev Containers (Recommended)

1. **Install the extension**:
   - Install "Dev Containers" by Microsoft from VS Code Extensions

2. **Open in container**:
   - Press `Ctrl+Shift+P` (Windows/Linux) or `Cmd+Shift+P` (macOS)
   - Select "Dev Containers: Open Folder in Container"
   - Select this workspace folder

3. **First build** (2-3 minutes):
   - Container builds with full Rust toolchain + nightly
   - All cargo tools installed (llvm-cov, insta, criterion, flamegraph)
   - Docker CLI available for integration tests

### Option 2: Command Line Build

```bash
# Enable BuildKit for cache mounts (IMPORTANT for fast rebuilds)
export DOCKER_BUILDKIT=1

# Build the container
cd .devcontainer
bash build.sh ansiblers-dev latest

# Or manually
docker build -f Dockerfile -t ansiblers-dev:latest ..
```

## What's Included

### Rust Toolchain
- **Stable**: Latest stable Rust (1.70+) with rustfmt, clippy
- **Nightly**: Full nightly toolchain (required for branch coverage)
- **MSRV**: Supports 1.70+

### Cargo Tools
- `cargo-llvm-cov`: Coverage with branch analysis
- `cargo-insta`: Snapshot testing
- `cargo-criterion`: Benchmarking
- `cargo-flamegraph`: Performance profiling
- `cargo-tarpaulin`: Alternative coverage tool

### Development Tools
- **Docker CLI**: Docker-in-Docker for integration tests
- **Python 3.13**: For Ansible compatibility testing
- **Build Tools**: gcc, pkg-config, libssl-dev, libffi-dev
- **Performance**: perf-tools, flamegraph, graphviz
- **Utilities**: curl, wget, jq, ripgrep, procps

### VS Code Extensions
- rust-analyzer
- LLDB debugger
- crates (dependency management)
- Better TOML (syntax highlighting)
- GitLens (version control)
- GitHub integration
- Todo Tree (task tracking)

### Pre-configured Settings
- Rust analyzer with clippy linting enabled
- Format on save (cargo fmt)
- 160-character ruler (Ansible convention)
- No trailing whitespace
- Pre-commit hooks (format + lint checks)

## Performance Optimization: BuildKit Cache Mounts

The Dockerfile uses **Docker BuildKit cache mounts** for faster rebuilds:

```dockerfile
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && apt-get install ...

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo install ...
```

**Benefits**:
- APT packages not re-downloaded on rebuilds
- Cargo crates not re-downloaded on rebuilds
- First build: ~2-3 minutes
- Subsequent builds: ~30 seconds (with cache)

**Enable BuildKit**:
```bash
# One-time setup
export DOCKER_BUILDKIT=1

# Or add to ~/.bashrc / ~/.zshrc
echo 'export DOCKER_BUILDKIT=1' >> ~/.bashrc
```

## Initial Setup After Container Launch

The dev container automatically runs initialization scripts:

1. **on-create.sh**: Runs during build
   - Updates apt package lists
   - Verifies base installation

2. **post-create.sh**: Runs after container creation
   - Verifies Rust installation and nightly toolchain
   - Creates test fixture directories
   - Sets up git pre-commit hooks
   - Prints quick start guide

## Common Commands

### Testing & Coverage

```bash
# Run all tests
cargo test --all

# Run with branch coverage (requires nightly)
cargo +nightly llvm-cov --all --lcov

# HTML coverage report
cargo +nightly llvm-cov report --html

# Check branch coverage threshold
cargo +nightly llvm-cov report --fail-under-branches 60

# Snapshot tests
cargo insta test
cargo insta review
```

### Code Quality

```bash
# Format check
cargo fmt --all -- --check

# Apply formatting
cargo fmt --all

# Lint with clippy
cargo clippy --all-targets -- -D warnings

# Generate docs
cargo doc --all --no-deps --open
```

### Benchmarking

```bash
# Run all benchmarks
cargo bench --all

# Run specific benchmark
cargo bench --bench playbook_execution

# With profiling
cargo flamegraph --bench playbook_execution
```

## File Structure

```
.devcontainer/
├── Dockerfile           # Container definition with BuildKit caching
├── devcontainer.json    # VS Code container configuration
├── post-create.sh       # Setup script (runs after container creation)
├── on-create.sh         # Build script (runs during container build)
├── build.sh             # Manual build script with BuildKit enabled
└── README.md            # This file
```

## Troubleshooting

### Container rebuild is slow (not using cache)

**Problem**: BuildKit cache not being used
**Solution**: Enable BuildKit:
```bash
export DOCKER_BUILDKIT=1
```

Or in `~/.docker/daemon.json`:
```json
{
  "features": {
    "buildkit": true
  }
}
```

### Cargo build fails in container

**Problem**: Cargo can't find dependencies
**Solution**: Cache may need to be cleared:
```bash
# In container
cargo clean
cargo build
```

### Docker-in-Docker not working

**Problem**: `docker` command fails in container
**Solution**:
1. Verify Docker is running on host
2. Rebuild container: `Dev Containers: Rebuild Container`
3. Check that DinD feature is enabled in devcontainer.json

### Nightly toolchain issues

**Problem**: `cargo +nightly` command not found
**Solution**:
```bash
# In container, verify nightly is installed
rustup toolchain list

# Should show: nightly-x86_64-unknown-linux-gnu (default or installed)

# If missing, install:
rustup toolchain install nightly
```

## Development Workflow

### 1. Local Development (in container)

```bash
# Edit code (VS Code automatically reformats on save)
# Run tests frequently
cargo test --all

# Before commit
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo llvm-cov --all
```

### 2. With Git Hooks

The post-create script installs git pre-commit hooks:

```bash
# Auto-runs on `git commit`:
# 1. cargo fmt --all -- --check
# 2. cargo clippy --all-targets -- -D warnings
```

### 3. Integration Tests

```bash
# Run integration tests in Docker
cargo test --test integration --all

# The container has DinD, so test infrastructure works
```

## Performance Tips

1. **Use BuildKit caching**: Set `DOCKER_BUILDKIT=1` before building
2. **Keep cargo cache**: Don't `cargo clean` unless necessary
3. **Incremental builds**: Use `cargo check` for fast feedback
4. **Parallel tests**: `cargo test --all` runs multiple tests in parallel
5. **Profiling**: Use `cargo flamegraph` for performance analysis

## Further Reading

- [AGENTS.md](../AGENTS.md): Development guidelines and commands
- [docs/design/ARCHITECTURE.md](../docs/design/ARCHITECTURE.md): Technical design
- [docs/design/TESTING_STRATEGY.md](../docs/design/TESTING_STRATEGY.md): Testing approach
- [Rust Book](https://doc.rust-lang.org/book/): Rust language guide
- [Cargo Book](https://doc.rust-lang.org/cargo/): Cargo documentation

## Support

For issues or questions:
1. Check [AGENTS.md](../AGENTS.md) for command reference
2. Review [docs/design/README.md](../docs/design/README.md) for project overview
3. Run `cargo doc --all --no-deps --open` for API documentation

---

**Happy coding!** 🚀
