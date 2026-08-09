#!/usr/bin/env bash
# Restart wrapper for outcome-loop.sh — same policy as autonomy-loop-forever.sh:
# exit 0/2/3 halt (complete / needs-human / graceful stop), exit 4 sleeps 10m
# and retries without counting as a failure, anything else restarts with a
# fast-fail crash-loop guard.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="$SCRIPT_DIR/outcome-loop.sh"

MAX_FAST_FAILS="${MAX_FAST_FAILS:-3}"
FAST_FAIL_SECS="${FAST_FAIL_SECS:-120}"

fast_fails=0
while true; do
    start=$(date +%s)
    bash "$TARGET"
    status=$?
    elapsed=$(( $(date +%s) - start ))

    case "$status" in
        0)  echo "[outcome-loop-forever] outcome complete (exit 0) — not restarting." >&2; exit 0 ;;
        2)  echo "[outcome-loop-forever] outcome blocked (exit 2) — needs a human; not restarting." >&2; exit 2 ;;
        3)  echo "[outcome-loop-forever] graceful stop (exit 3) — not restarting." >&2; exit 0 ;;
        4)  fast_fails=0
            echo "[outcome-loop-forever] session/usage limit (exit 4); sleeping 10m before retry" >&2
            sleep 600
            continue ;;
    esac

    if [ "$elapsed" -lt "$FAST_FAIL_SECS" ]; then
        fast_fails=$((fast_fails + 1))
        if [ "$fast_fails" -ge "$MAX_FAST_FAILS" ]; then
            echo "[outcome-loop-forever] ${fast_fails} consecutive fast failures — halting instead of retrying forever." >&2
            exit 1
        fi
    else
        fast_fails=0
    fi
    echo "[outcome-loop-forever] exit ${status} after ${elapsed}s; sleeping 10m before restart (fast-fail ${fast_fails}/${MAX_FAST_FAILS})" >&2
    sleep 600
done
