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
USER_DATA_DIR=""
WORKSPACE_COPY=""

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
  # Clean up temporary directories
  if [ -n "$USER_DATA_DIR" ] && [ -d "$USER_DATA_DIR" ]; then
    rm -rf "$USER_DATA_DIR"
  fi
  if [ -n "$WORKSPACE_COPY" ] && [ -d "$WORKSPACE_COPY" ]; then
    rm -rf "$WORKSPACE_COPY"
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

# Create a fresh user-data-dir with demo settings
USER_DATA_DIR="$(mktemp -d /tmp/code-server-demo-XXXXXX)"
mkdir -p "$USER_DATA_DIR/User"
cat > "$USER_DATA_DIR/User/settings.json" << 'SETTINGS'
{
  "workbench.startupEditor": "none",
  "workbench.tips.enabled": false,
  "workbench.welcomePage.walkthroughs.openOnInstall": false,
  "chat.commandCenter.enabled": false,
  "chat.editor.enabled": false,
  "security.workspace.trust.enabled": false,
  "security.workspace.trust.startupPrompt": "never",
  "screencastMode.fontSize": 28,
  "screencastMode.verticalOffset": 5,
  "screencastMode.mouseIndicatorSize": 30
}
SETTINGS

# Create a disposable copy of the demo workspace
# (some tests create/rename files — we never want to modify the source)
WORKSPACE_COPY="$(mktemp -d /tmp/smelt-demo-ws-XXXXXX)/demo_workspace"
cp -r "$REPO_ROOT/examples/demo_workspace" "$WORKSPACE_COPY"

# Install the extension into the fresh user-data-dir
echo "Installing smelt extension into code-server..."
code-server --user-data-dir "$USER_DATA_DIR" --install-extension "$VSIX_PATH" --force

code-server \
  --user-data-dir "$USER_DATA_DIR" \
  --port "$PORT" \
  --auth none \
  --disable-telemetry \
  --disable-update-check \
  --disable-workspace-trust \
  --disable-getting-started-override \
  "$WORKSPACE_COPY" &
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
export CODE_SERVER_URL="http://localhost:$PORT"
(cd "$DEMO_DIR" && npx playwright test)

echo ""
echo "=== Step 6: Generate documentation ==="
(cd "$DEMO_DIR" && npx tsx generate-docs.ts)

echo ""
echo "=== Done ==="
echo "Media:  $DEMO_DIR/media/"
echo "Docs:   $DEMO_DIR/output/"
