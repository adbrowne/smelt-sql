#!/usr/bin/env bash
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="$SCRIPT_DIR/autonomy-loop.sh"

# Halt after this many consecutive fast failures (loop died within
# FAST_FAIL_SECS). Guards against the 2026-07-05 pattern: claude crashing
# instantly every restart, burning a retry every 10 minutes indefinitely.
MAX_FAST_FAILS="${MAX_FAST_FAILS:-3}"
FAST_FAIL_SECS="${FAST_FAIL_SECS:-120}"

fast_fails=0
while true; do
    start=$(date +%s)
    "$TARGET"
    status=$?
    elapsed=$(( $(date +%s) - start ))

    case "$status" in
        0)  # All done — master backlog fully remediated. Nothing to restart for.
            echo "[autonomy-loop-forever] master backlog complete (exit 0) — not restarting." >&2
            exit 0 ;;
        2)  # Master exhausted — needs a human to scaffold the next sub-plan.
            # Restarting would just re-discover the same state every 10 minutes.
            echo "[autonomy-loop-forever] master exhausted (exit 2) — needs a human; not restarting." >&2
            exit 2 ;;
        3)  # Graceful stop requested (.claude/autonomy.stop). Honour it.
            echo "[autonomy-loop-forever] graceful stop requested (exit 3) — not restarting." >&2
            exit 0 ;;
        4)  # Session/usage limit hit. Expected, not a crash — the account is
            # just out of budget until the window resets. Never counts toward
            # the fast-fail crash-loop guard below (so it can recur any
            # number of times in a row while waiting out the reset without
            # tripping a permanent halt) and always retries.
            fast_fails=0
            echo "[autonomy-loop-forever] session/usage limit hit (exit 4); sleeping 10m before retry (not counted as a failure)" >&2
            sleep 600
            continue ;;
    esac

    if [ "$elapsed" -lt "$FAST_FAIL_SECS" ]; then
        fast_fails=$((fast_fails + 1))
        if [ "$fast_fails" -ge "$MAX_FAST_FAILS" ]; then
            echo "[autonomy-loop-forever] ${fast_fails} consecutive failures within ${FAST_FAIL_SECS}s — something is broken (claude crash? bad config); halting instead of retrying forever." >&2
            exit 1
        fi
    else
        fast_fails=0
    fi

    echo "[autonomy-loop-forever] autonomy-loop.sh exited with status $status after ${elapsed}s; sleeping 10m before restart (fast-fail ${fast_fails}/${MAX_FAST_FAILS})" >&2
    sleep 600
done
