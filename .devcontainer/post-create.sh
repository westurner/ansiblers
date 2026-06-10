#!/bin/bash
# Ansiblers Development Container - Post Create Script
# Runs after the container is created (keep this FAST < 30 seconds!)

# Don't exit on error
set +e

cd /workspace

echo "🚀 Ansiblers Dev Container - Post Create"
echo "=========================================="

# Quick checks only - no heavy compilation
echo "✓ Quick environment check..."
rustc --version 2>/dev/null | head -1
cargo --version 2>/dev/null | head -1
python3 --version 2>/dev/null | head -1

# Create necessary directories quickly
echo "✓ Setting up directories..."
mkdir -p reports/{coverage,benchmarks,test-results}
mkdir -p tests/{fixtures/{playbooks,inventories,roles},integration,snapshots}
mkdir -p tmp

# Set up git hooks (if .git exists)
if [ -d ".git" ]; then
    echo "✓ Setting up git hooks..."
    mkdir -p .git/hooks
    
    # Create pre-commit hook for formatting check
    cat > .git/hooks/pre-commit << 'EOF'
#!/bin/bash
echo "Running pre-commit checks..."
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
EOF
    chmod +x .git/hooks/pre-commit
    echo "  Pre-commit hook installed"
fi

# Initialize .vscode/settings.json if needed
if [ ! -f ".vscode/settings.json" ]; then
    mkdir -p .vscode
    cat > .vscode/settings.json << 'EOF'
{
  "rust-analyzer.checkOnSave.command": "clippy",
  "editor.formatOnSave": true,
  "editor.rulers": [100, 160]
}
EOF
    echo "✓ VS Code settings created"
fi

echo ""
echo "=========================================="
echo "✅ Setup Complete"
echo "=========================================="
echo ""
echo "Note: Heavy compilation tasks run during build"
echo "      (See .devcontainer/on-create.sh)"
echo ""
echo "Ready to develop! Try:"
echo "  cargo test --lib         # Quick library tests"
echo "  cargo build --release    # Release build"
echo ""
