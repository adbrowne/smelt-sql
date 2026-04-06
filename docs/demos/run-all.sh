#!/usr/bin/env bash
#
# Run the full LSP demo pipeline end-to-end:
#   1. Build the smelt-lsp binary
#   2. Package the VS Code extension
#   3. Start code-server with the extension
#   4. Run all Playwright tests (generating media)
#   5. Generate markdown documentation
#   6. Shut down code-server
#
# Usage:
#   bash docs/demos/run-all.sh                  # from repo root
#   bash docs/demos/run-all.sh --update-snapshots  # clear old media first
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DEMO_DIR="$SCRIPT_DIR"
PORT="${CODE_SERVER_PORT:-18080}"
CODE_SERVER_PID=""

UPDATE_SNAPSHOTS=false
for arg in "$@"; do
  case "$arg" in
    --update-snapshots)
      UPDATE_SNAPSHOTS=true
      ;;
    *)
      echo "Unknown argument: $arg"
      echo "Usage: $0 [--update-snapshots]"
      exit 1
      ;;
  esac
done

cleanup() {
  if [ -n "$CODE_SERVER_PID" ]; then
    echo "Stopping code-server (PID $CODE_SERVER_PID)..."
    kill "$CODE_SERVER_PID" 2>/dev/null || true
    wait "$CODE_SERVER_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

echo "=== Step 1: Build smelt-lsp ==="
(cd "$REPO_ROOT" && cargo build -p smelt-lsp)

echo ""
echo "=== Step 2: Package VS Code extension ==="
VSIX_PATH="$REPO_ROOT/editors/vscode/smelt-0.1.0.vsix"
(cd "$REPO_ROOT/editors/vscode" && npm install && npx vsce package -o smelt-0.1.0.vsix)

echo ""
echo "=== Step 3: Install npm deps for demos ==="
(cd "$DEMO_DIR" && npm install)

echo ""
echo "=== Step 4: Start code-server ==="
export PATH="$REPO_ROOT/target/debug:$PATH"

# Install the extension
echo "Installing smelt extension into code-server..."
code-server --install-extension "$VSIX_PATH"

DEMO_WORKSPACE="$REPO_ROOT/examples/demo_workspace"
code-server \
  --port "$PORT" \
  --auth none \
  --disable-telemetry \
  --disable-update-check \
  --disable-workspace-trust \
  --disable-getting-started-override \
  "$DEMO_WORKSPACE" &
CODE_SERVER_PID=$!

# Wait for code-server to be ready
echo "Waiting for code-server on port $PORT..."
for i in $(seq 1 30); do
  if curl -sf "http://localhost:$PORT" > /dev/null 2>&1; then
    echo "code-server is ready."
    break
  fi
  if [ "$i" -eq 30 ]; then
    echo "ERROR: code-server did not start within 30 seconds."
    exit 1
  fi
  sleep 1
done

echo ""
if $UPDATE_SNAPSHOTS; then
  echo "=== Step 5: Clear old media (--update-snapshots) ==="
  rm -rf "$DEMO_DIR/media"
  mkdir -p "$DEMO_DIR/media"
fi

echo "=== Step 5: Run Playwright tests ==="
(cd "$DEMO_DIR" && npx playwright test)

echo ""
echo "=== Step 6: Generate documentation ==="
(cd "$DEMO_DIR" && npx tsx generate-docs.ts)

echo ""
echo "=== Done ==="
echo "Media:  $DEMO_DIR/media/"
echo "Docs:   $DEMO_DIR/output/"
