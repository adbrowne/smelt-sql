#!/usr/bin/env bash
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="$SCRIPT_DIR/autonomy-loop.sh"

while true; do
    "$TARGET"
    status=$?
    echo "[autonomy-loop-forever] autonomy-loop.sh exited with status $status; sleeping 1h before restart" >&2
    sleep 3600
done
