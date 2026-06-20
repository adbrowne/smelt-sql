#!/usr/bin/env bash
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="$SCRIPT_DIR/autonomy-loop.sh"

while true; do
    "$TARGET"
    status=$?
    # Exit 3 = graceful stop requested (the inner loop saw .claude/autonomy.stop
    # and finished its current iteration). Honour it: do not restart.
    if [ "$status" -eq 3 ]; then
        echo "[autonomy-loop-forever] graceful stop requested (exit 3) — not restarting." >&2
        exit 0
    fi
    echo "[autonomy-loop-forever] autonomy-loop.sh exited with status $status; sleeping 10m before restart" >&2
    sleep 600
done
