#!/bin/bash
# Build script for Ansiblers dev container with BuildKit caching enabled
# BuildKit enables volume cache mounts for faster rebuilds

set -e

echo "🐳 Building Ansiblers Dev Container with BuildKit..."
echo "======================================================"

# Enable BuildKit for this build
export DOCKER_BUILDKIT=1

# Get the directory of this script
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
CONTEXT_DIR="$( dirname "$SCRIPT_DIR" )"

# Build arguments
IMAGE_NAME="${1:-ansiblers-dev}"
IMAGE_TAG="${2:-latest}"
DOCKERFILE="${SCRIPT_DIR}/Dockerfile"

echo "📋 Build Configuration:"
echo "  Context: $CONTEXT_DIR"
echo "  Dockerfile: $DOCKERFILE"
echo "  Image: $IMAGE_NAME:$IMAGE_TAG"
echo "  BuildKit: Enabled"
echo "  Cache Mounts:"
echo "    • APT cache: /var/cache/apt, /var/lib/apt"
echo "    • Cargo cache: /usr/local/cargo/registry, /usr/local/cargo/git"
echo ""

# Build with BuildKit enabled
docker build \
  --progress=plain \
  --build-arg BUILDKIT_INLINE_CACHE=1 \
  -f "$DOCKERFILE" \
  -t "$IMAGE_NAME:$IMAGE_TAG" \
  -t "$IMAGE_NAME:latest" \
  "$CONTEXT_DIR"

echo ""
echo "✅ Build complete!"
echo ""
echo "Usage with VS Code:"
echo "  1. Install 'Dev Containers' extension"
echo "  2. Press Ctrl+Shift+P and select 'Dev Containers: Rebuild Container'"
echo ""
echo "Or rebuild with docker directly:"
echo "  docker run --rm -it $IMAGE_NAME:$IMAGE_TAG /bin/bash"
echo ""
