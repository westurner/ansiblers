#!/bin/bash
# Rebuild the dev container from scratch

set -e

echo "🔨 Rebuilding Ansiblers Dev Container..."
echo "=========================================="

# Get the directory of this script
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "📁 Workspace root: $WORKSPACE_ROOT"

# Clean up old container and images
echo "🗑️  Cleaning up old containers and images..."
docker container prune -f 2>/dev/null || true
docker image prune -f 2>/dev/null || true

# Rebuild the image
echo "🏗️  Building new dev container image..."
cd "$SCRIPT_DIR"

DOCKER_BUILDKIT=1 docker build \
  --file Dockerfile \
  --tag ansiblers-dev:latest \
  --cache-from ansiblers-dev:latest \
  . || {
    echo "❌ Build failed. Trying without cache..."
    DOCKER_BUILDKIT=1 docker build \
      --file Dockerfile \
      --no-cache \
      --tag ansiblers-dev:latest \
      .
  }

echo ""
echo "✅ Container rebuild complete!"
echo ""
echo "To reopen in the container, run:"
echo "  1. Open Command Palette (Ctrl+Shift+P)"
echo "  2. Run: Dev Containers: Rebuild and Reopen in Container"
echo ""
