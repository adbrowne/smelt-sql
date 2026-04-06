#!/usr/bin/env bash
# Quick rebuild + reinstall cycle for developing the smelt LSP extension.
# Rebuilds the LSP binary, repackages the VSIX, installs it, and reloads VSCode.
#
# Best run from a VSCode integrated terminal (Ctrl+`), where the IPC socket
# is set automatically. Also works from a regular SSH session by auto-detecting
# the most recent IPC socket (may pick a stale one if you've reconnected).
#
# Usage: ./scripts/dev-reload.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# 1. Build the LSP binary (debug for speed)
echo "==> Building smelt-lsp..."
cargo build -p smelt-lsp --manifest-path "$REPO_ROOT/Cargo.toml" 2>&1 | tail -3

# 2. Package the VSIX
echo "==> Packaging extension..."
cd "$REPO_ROOT/editors/vscode"
npm run compile --silent 2>&1
npx vsce package -o "$REPO_ROOT/target/smelt.vsix" 2>&1 | tail -1

# 3. Find VSCode CLI
CODE_CLI=$(ls -t ~/.vscode-server/cli/servers/Stable-*/server/bin/remote-cli/code 2>/dev/null | head -1)
if [[ -z "$CODE_CLI" ]]; then
    echo "Error: No VSCode server CLI found (is VSCode connected via SSH?)" >&2
    exit 1
fi

# 4. Find IPC socket
if [[ -z "${VSCODE_IPC_HOOK_CLI:-}" ]]; then
    SOCK=$(ls -t /run/user/$(id -u)/vscode-ipc-*.sock 2>/dev/null | head -1)
    if [[ -z "$SOCK" ]]; then
        echo "Error: No VSCode IPC socket found." >&2
        echo "Run this from a VSCode integrated terminal, or set VSCODE_IPC_HOOK_CLI." >&2
        exit 1
    fi
    echo "    (Not in a VSCode terminal — using newest IPC socket: ${SOCK##*/})"
    export VSCODE_IPC_HOOK_CLI="$SOCK"
fi

# 5. Install and reload
echo "==> Installing extension..."
"$CODE_CLI" --install-extension "$REPO_ROOT/target/smelt.vsix" --force 2>&1 | tail -2

echo "==> Reloading VSCode..."
"$CODE_CLI" --command workbench.action.reloadWindow 2>/dev/null || \
    echo "    (Auto-reload failed. Ctrl+Shift+P → 'Reload Window')"

echo "==> Done."
