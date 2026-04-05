#!/usr/bin/env bash
set -euo pipefail

# Autonomous Claude Code loop for LSP refactorings plan
# Usage: ./scripts/run-lsp-refactorings-loop.sh [max_sessions]
#
# Runs Claude Code sessions in a loop. Each session:
# 1. Reads the plan to find the next incomplete phase
# 2. Writes failing (red) tests first
# 3. Implements code to make them pass (green)
# 4. Commits and pushes progress
# 5. Updates the plan
#
# Stops when all phases are complete or max_sessions reached.

MAX_SESSIONS="${1:-15}"
PLAN_FILE="docs/plans/20260405-lsp-refactorings.md"
LOG_DIR="logs/lsp-refactorings-loop"
BRANCH="lsp-refactorings"

mkdir -p "$LOG_DIR"

echo "=== LSP Refactorings & Code Actions Loop ==="
echo "Max sessions: $MAX_SESSIONS"
echo "Plan: $PLAN_FILE"
echo "Branch: $BRANCH"
echo "Logs: $LOG_DIR/"
echo ""

# Ensure we're on the right branch
current_branch=$(git branch --show-current)
if [[ "$current_branch" != "$BRANCH" ]]; then
    echo "ERROR: Expected branch '$BRANCH', got '$current_branch'"
    echo "Please checkout the correct branch first:"
    echo "  git checkout -b $BRANCH"
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
        "You are executing an autonomous implementation loop for the LSP refactorings plan.

PLAN FILE: $PLAN_FILE
RESEARCH: docs/research/2026-04-05-lsp-refactorings.md

YOUR TASK:
1. Read the plan file to find the next phase that is '[ ]' (not started) or '[~]' (in progress).
2. Update that phase's status to '[~]' (in progress) in the plan.
3. Follow the RED-GREEN testing discipline:
   a. FIRST write the failing tests listed in the 'Red tests' section of the phase.
   b. Run 'cargo test' to confirm they fail (red).
   c. THEN implement the code listed in the 'Green implementation' section.
   d. Run 'cargo test' to confirm they pass (green).
   e. If any test still fails, debug and fix until green.
4. Run 'cargo fmt --all' and 'cargo clippy --all-targets' — fix any issues.
5. Run 'cargo test' to ensure nothing is broken.
6. Run 'cargo test -p smelt-cli --test example_diagnostics' to verify examples are clean.
7. Mark the phase as '[x]' (complete) in the plan. Check off individual work items with [x].
8. Add a session log entry at the bottom of the plan with: date, session number ($i), what was done, any decisions made.
9. Stage all changed files, commit with a descriptive message, and push to origin/$BRANCH.

RED-GREEN TESTING RULES:
- ALWAYS write tests BEFORE implementation code.
- Tests should be specific and test one behavior each.
- Run tests after writing them to confirm they fail for the right reason (not a compile error — the test should compile but the assertion should fail or the feature should not exist yet).
- If a test can't compile yet (e.g., references a struct that doesn't exist), that's OK for Phase 0 — write the struct first, then the test, then the implementation.
- After implementation, ALL tests must pass. No skipping or ignoring.

IMPORTANT RULES:
- Work on exactly ONE phase per session (the next incomplete one).
- If a phase is too large, do as much as you can, leave it as '[~]', and note progress in the session log.
- If you make design decisions, document them in the 'Decisions Log' section of the plan.
- If you hit a blocker that requires human input, mark the phase as '[!]' and explain in the session log. Then stop.
- Always push after committing so the PR stays up to date.
- Run tests before committing — do not commit broken code.
- Follow the pure function rule: analysis logic as pure functions, Salsa queries as thin wrappers.
- Read CLAUDE.md for project conventions before starting.
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
