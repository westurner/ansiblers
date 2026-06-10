# Ansiblers Development Container
# Includes Rust toolchain, testing tools, benchmarking, and Docker-in-Docker

FROM mcr.microsoft.com/devcontainers/rust:2-trixie
#FROM mcr.microsoft.com/devcontainers/rust:latest

# Set environment variables
ENV DEBIAN_FRONTEND=noninteractive \
    CARGO_TERM_COLOR=always \
    RUST_BACKTRACE=1

# Enable BuildKit for cache mounts (requires: DOCKER_BUILDKIT=1)
# Syntax: RUN --mount=type=cache,target=/path,sharing=locked
#
# Cache Mounts Included:
#   • APT: /var/cache/apt, /var/lib/apt (system packages)
#   • Rustup: /root/.rustup (Rust toolchains, components, downloads)
#   • Cargo: /usr/local/cargo/registry, /usr/local/cargo/git (crates, dependencies)

# Install additional system packages (with APT cache)
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && apt-get install -y --no-install-recommends \
    # Docker CLI for integration tests
    docker.io \
    podman \
    # Python for Ansible compatibility testing
    python3 \
    python3-pip \
    python3-venv \
    # Build tools
    build-essential \
    pkg-config \
    libssl-dev \
    libffi-dev \
    # Git (usually pre-installed but ensure)
    git \
    # Performance analysis
    #perf-tools \
    #flamegraph \
    # Documentation generation
    graphviz \
    # Utility tools
    curl \
    wget \
    jq \
    ripgrep \
    procps \
    bash-completion \
    make \
    && apt-get clean && rm -rf /var/lib/apt/lists/*

    # TODO: sudo

# Install Rust toolchain components for stable (with rustup cache)
RUN --mount=type=cache,target=/root/.rustup,sharing=locked \
    rustup component add rustfmt clippy

# Install nightly toolchain (required for llvm-cov branch coverage, with rustup cache)
RUN --mount=type=cache,target=/root/.rustup,sharing=locked \
    rustup toolchain install nightly && \
    rustup component add rustfmt clippy --toolchain nightly

# Install cargo tools for testing and benchmarking (with cargo cache)
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo install \
    cargo-llvm-cov \
    cargo-insta \
    cargo-criterion \
    cargo-tarpaulin

# Fix sudo permissions for rootless podman (--userns=keep-id)
# The docker-init.sh script in the base image tries to use sudo, but it needs
# to be properly configured in rootless mode. Allow vscode user to use sudo
# for docker-init.sh and other initialization without password.
RUN echo "vscode ALL=(ALL) NOPASSWD: ALL" >> /etc/sudoers.d/vscode && \
    chmod 0440 /etc/sudoers.d/vscode

# Create non-root user with Docker socket access (handled by devcontainer.json)
# Verify installations
RUN rustc --version && \
    cargo --version && \
    rustup toolchain list && \
    rustfmt --version && \
    clippy-driver --version && \
    cargo llvm-cov --version && \
    cargo insta --version && \
    python3 --version

COPY .devcontainer/post-create.sh .devcontainer/on-create.sh /usr/local/bin/

# Set working directory
WORKDIR /workspace
