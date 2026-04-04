#!/usr/bin/env bash
set -euo pipefail

# Autonomous Claude Code loop for parser-type-testing-completeness plan
# Usage: ./scripts/run-parser-type-loop.sh [max_sessions]
#
# Runs Claude Code sessions in a loop. Each session:
# 1. Reads the plan to find the next incomplete phase
# 2. Implements it
# 3. Commits and pushes progress
# 4. Updates the plan
#
# Stops when all phases are complete or max_sessions reached.

MAX_SESSIONS="${1:-20}"
PLAN_FILE="docs/plans/20260404-parser-type-testing-completeness.md"
LOG_DIR="logs/parser-type-loop"
BRANCH="parser-type-testing-completeness"

mkdir -p "$LOG_DIR"

echo "=== Parser & Type Testing Completeness Loop ==="
echo "Max sessions: $MAX_SESSIONS"
echo "Plan: $PLAN_FILE"
echo "Branch: $BRANCH"
echo "Logs: $LOG_DIR/"
echo ""

# Ensure we're on the right branch
current_branch=$(git branch --show-current)
if [[ "$current_branch" != "$BRANCH" ]]; then
    echo "ERROR: Expected branch '$BRANCH', got '$current_branch'"
    echo "Please checkout the correct branch first."
    exit 1
fi

for i in $(seq 1 "$MAX_SESSIONS"); do
    echo ""
    echo "================================================"
    echo "Session $i of $MAX_SESSIONS — $(date '+%Y-%m-%d %H:%M:%S')"
    echo "================================================"

    # Check if all phases are complete
    remaining=$(grep -c '## Phase.*\[ \]' "$PLAN_FILE" 2>/dev/null || true)
    in_progress=$(grep -c '## Phase.*\[~\]' "$PLAN_FILE" 2>/dev/null || true)

    if [[ "$remaining" -eq 0 && "$in_progress" -eq 0 ]]; then
        echo "All phases complete! Stopping loop."
        break
    fi

    echo "Remaining phases: $remaining (+ $in_progress in progress)"

    SESSION_LOG="$LOG_DIR/session-$(printf '%02d' "$i")-$(date '+%Y%m%d-%H%M%S').log"

    # Run Claude Code headlessly
    claude --print --verbose \
        --dangerously-skip-permissions \
        --max-turns 100 \
        "You are executing an autonomous implementation loop for the parser & type system testing completeness plan.

PLAN FILE: $PLAN_FILE

YOUR TASK:
1. Read the plan file to find the next phase that is '[ ]' (not started) or '[~]' (in progress).
2. Update that phase's status to '[~]' (in progress) in the plan.
3. Implement ALL the work items for that phase.
4. Run the verification command specified in the phase.
5. If tests fail, fix the issues. If you discover new work needed, add sub-items to the phase or create a new phase.
6. Run 'cargo fmt --all' and 'cargo clippy --all-targets' — fix any issues.
7. Run 'cargo test' to ensure nothing is broken.
8. Mark the phase as '[x]' (complete) in the plan. Also check off individual work items with [x].
9. Add a session log entry at the bottom of the plan with: date, session number ($i), what was done, any decisions made.
10. Stage all changed files, commit with a descriptive message, and push to origin/$BRANCH.

IMPORTANT RULES:
- Work on exactly ONE phase per session (the next incomplete one).
- If a phase is too large, do as much as you can, leave it as '[~]', and note progress in the session log.
- If you make design decisions, document them in the 'Decisions Log' section of the plan.
- If you hit a blocker that requires human input, mark the phase as '[!]' and explain in the session log. Then move to the next phase.
- Always push after committing so the PR stays up to date.
- Run tests before committing — do not commit broken code.
- If 'cargo test' takes too long, you can run just the relevant package tests.
" \
        2>&1 | tee "$SESSION_LOG"

    exit_code=${PIPESTATUS[0]}
    echo ""
    echo "Session $i finished with exit code: $exit_code"

    if [[ $exit_code -ne 0 ]]; then
        echo "WARNING: Session exited with non-zero code. Check log: $SESSION_LOG"
        echo "Continuing to next session..."
    fi

    # Brief pause between sessions
    sleep 2
done

echo ""
echo "=== Loop complete ==="
echo "Check the plan: $PLAN_FILE"
echo "Check the PR on GitHub for full progress."
