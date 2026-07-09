#!/usr/bin/env bash
#
# Forever wrapper for the property-discovery loop.
#
# Runs property-loop.sh; on any non-terminal exit (credit exhaustion, rate-limit,
# crash, max-iterations) it sleeps 10 MINUTES and retries — so the loop
# self-starts when credits return next session. This is the behaviour the loop
# was asked for: "try again in 10 minutes so it can run and start when we get
# credits again."
#
# Terminal exits that do NOT retry:
#   3  graceful stop requested (.claude/property-loop.stop) — exit cleanly.
#   2  catalog exhausted — no pending cell; a human must seed the next tranche.
#
# Everything else (1 = infra failure / max-iter, non-zero from a killed/credit-
# exhausted claude) sleeps 600s and restarts.
#
# Run it detached so it outlives the session:
#   tmux new-session -d -s property-loop \
#     "cd <worktree> && DUCKDB_LIB_DIR=/usr/local/lib LD_LIBRARY_PATH=/usr/local/lib \
#        bash .claude/scripts/property-loop-forever.sh"

set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="${SCRIPT_DIR}/property-loop.sh"
RETRY_SLEEP="${RETRY_SLEEP:-600}"

while true; do
  "${TARGET}"
  status=$?
  case "${status}" in
    3)
      echo "[property-loop-forever] graceful stop (exit 3) — not restarting." >&2
      exit 0
      ;;
    2)
      echo "[property-loop-forever] catalog exhausted (exit 2) — a human must seed the next tranche. Not restarting." >&2
      exit 2
      ;;
    *)
      echo "[property-loop-forever] property-loop.sh exited ${status}; sleeping ${RETRY_SLEEP}s before retry (credits/rate-limit will recover)." >&2
      sleep "${RETRY_SLEEP}"
      ;;
  esac
done
