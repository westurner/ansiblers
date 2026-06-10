#!/bin/bash
# Ansiblers Development Container - On Create Script
# Runs during container build (before post-create, can take longer)

set +e  # Don't fail on warnings

echo "🔨 Ansiblers Dev Container - Build Setup"
echo "=========================================="

# Update package lists
echo "✓ Updating package lists..."
apt-get update -qq 2>/dev/null

# Verify base Rust installation
echo "✓ Verifying Rust installation..."
rustc --version 2>/dev/null || echo "  warning: Rust not in PATH"
cargo --version 2>/dev/null || echo "  warning: Cargo not in PATH"

# Note: Cargo cache warming removed - it causes OOMKill on resource-constrained systems.
# Compilation cache will be warmed on first `cargo build/test` after container starts.

echo ""
echo "✅ Build setup complete"
echo ""
