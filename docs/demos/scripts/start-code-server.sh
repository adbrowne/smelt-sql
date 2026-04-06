#!/usr/bin/env bash
#
# Launch code-server with the smelt extension for demo/testing purposes.
# Usage: bash scripts/start-code-server.sh [PORT]
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
DEMO_WORKSPACE="$REPO_ROOT/examples/demo_workspace"
VSIX_PATH="$REPO_ROOT/editors/vscode/smelt-0.1.0.vsix"
PORT="${1:-18080}"

# Ensure the LSP binary is built
echo "Building smelt-lsp..."
(cd "$REPO_ROOT" && cargo build -p smelt-lsp)

# Ensure the VSIX is packaged
if [ ! -f "$VSIX_PATH" ]; then
  echo "Packaging VS Code extension..."
  (cd "$REPO_ROOT/editors/vscode" && npm install && npx vsce package -o smelt-0.1.0.vsix)
fi

# Add the debug binary to PATH so the extension can find it
export PATH="$REPO_ROOT/target/debug:$PATH"

# Install extension first (code-server exits after --install-extension)
echo "Installing smelt extension..."
code-server --install-extension "$VSIX_PATH"

echo "Starting code-server on port $PORT with workspace $DEMO_WORKSPACE"
exec code-server \
  --port "$PORT" \
  --auth none \
  --disable-telemetry \
  --disable-update-check \
  --disable-workspace-trust \
  --disable-getting-started-override \
  "$DEMO_WORKSPACE"
