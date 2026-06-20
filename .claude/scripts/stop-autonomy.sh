#!/usr/bin/env bash
# Request a *graceful* stop of the autonomy loop.
#
# Drops a flag file that the loop checks at the top of each iteration. The
# iteration that is currently running finishes normally — it commits and pushes
# its work — and then the loop exits before starting another (inner-loop exit
# code 3; the forever wrapper sees that and does not restart). Nothing in flight
# is interrupted, so no tokens or work are wasted.
#
# This is not immediate: if an iteration is mid-run it can take several minutes
# to reach the next checkpoint. For an immediate hard stop, Ctrl-C / kill the
# loop (or `tmux kill-session -t autonomy`) — but that discards the in-progress
# iteration's uncommitted work.
#
# The flag lives at .claude/autonomy.stop, which is gitignored by `.claude/*`,
# so it is never committed and the loop's auto-stash/merge leave it untouched.
# The loop removes it automatically when it acts on it. To cancel a pending
# stop request before it takes effect, just delete the flag (printed below).
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STOP_FLAG="${STOP_FLAG:-${SCRIPT_DIR}/../autonomy.stop}"

touch "${STOP_FLAG}"

echo "Graceful stop requested."
echo "  flag: ${STOP_FLAG}"
echo "The current iteration will finish (commit + push), then the loop exits."
echo "Cancel before it takes effect with:  rm '${STOP_FLAG}'"

if ! pgrep -f 'bash .*autonomy-loop' >/dev/null 2>&1; then
    echo
    echo "Note: no 'autonomy-loop' process appears to be running right now."
    echo "If you start the loop later it will stop immediately and clear this flag."
fi
