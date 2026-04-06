#!/usr/bin/env bash
set -euo pipefail

# Autonomous Claude Code loop for LSP visual documentation plan
# Usage: ./scripts/run-lsp-visual-docs-loop.sh [max_sessions]
#
# Runs Claude Code sessions in a loop. Each session:
# 1. Reads the plan to find the next incomplete phase
# 2. Implements that phase (infra, demo scripts, media capture, docs)
# 3. Commits and pushes progress
# 4. Updates the plan
#
# Stops when all phases are complete or max_sessions reached.

MAX_SESSIONS="${1:-15}"
PLAN_FILE="docs/plans/20260406-lsp-visual-docs.md"
LOG_DIR="logs/lsp-visual-docs-loop"
BRANCH="lsp-visual-docs"

mkdir -p "$LOG_DIR"

echo "=== LSP Visual Documentation Loop ==="
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

    # Run Claude Code headlessly (stream-json shows real-time progress)
    claude --print --verbose \
        --output-format stream-json \
        --dangerously-skip-permissions \
        --max-turns 100 \
        "You are executing an autonomous implementation loop for the LSP visual documentation plan.

PLAN FILE: $PLAN_FILE

YOUR TASK:
1. Read the plan file to find the next phase that is '[ ]' (not started) or '[~]' (in progress).
2. Update that phase's status to '[~]' (in progress) in the plan.
3. Implement the work items listed in that phase:
   - For Phase 0: Set up Playwright, code-server helpers, demo workspace with all models, and smoke test.
   - For Phases 1-7: Write Playwright test scripts that open the demo workspace in code-server,
     perform the described interactions, and capture screenshots/videos to media/ directories.
     Each test file should be self-contained and idempotent.
   - For Phase 8: Assemble documentation from generated media.
   - For Phase 9: Create the run-all script and CI integration.
4. After implementing, run the relevant tests to verify:
   - Phase 0: \`cd docs/demos && npx playwright test tests/smoke.spec.ts\`
   - Phases 1-7: \`cd docs/demos && npx playwright test tests/<feature>.spec.ts\`
   - Phase 8: Verify markdown renders and links resolve.
   - Phase 9: Run \`docs/demos/run-all.sh\` end-to-end.
5. Mark the phase as '[x]' (complete) in the plan. Check off individual work items with [x].
6. Add a session log entry at the bottom of the plan with: date, session number ($i), what was done, any decisions made.
7. Stage all changed files, commit with a descriptive message, and push to origin/$BRANCH.

IMPORTANT CONTEXT:
- The smelt LSP server binary is built with: cargo build -p smelt-lsp
- The VS Code extension is in editors/vscode/ — package with: cd editors/vscode && npm run package
- The demo workspace should be at examples/demo_workspace/
- Playwright tests go in docs/demos/tests/
- Generated media goes in docs/demos/media/<feature-name>/
- Use TypeScript for all Playwright code.
- code-server can be installed via: npm install -g @coder/code-server (or use npx)
- The smelt extension VSIX is produced by: cd editors/vscode && npx vsce package
- For screenshots, use page.screenshot() with clip options to focus on the relevant UI area.
- For videos, use Playwright's built-in video recording (recordVideo in browser context options).
- Make demo scripts robust: wait for LSP to initialize (watch for diagnostics appearing) before capturing.

DESIGN GOALS:
- Each demo should tell a story that a data engineer would relate to.
- Screenshots should be clean, focused, and annotated (use CSS overlays or post-processing if needed).
- Videos should be short (5-15 seconds) showing a single interaction.
- The demo workspace models should be realistic but minimal (5-15 lines each).
- Prefer dark theme for screenshots (it's what most developers use and looks better in docs).

RULES:
- Work on exactly ONE phase per session (the next incomplete one).
- If a phase is too large, do as much as you can, leave it as '[~]', and note progress in the session log.
- If you hit a blocker requiring human input, mark the phase as '[!]' and explain in the session log.
- Always push after committing so the branch stays up to date.
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

    # Commit and push any changes from this session (safety net)
    if ! git diff --quiet HEAD 2>/dev/null || [ -n "$(git ls-files --others --exclude-standard)" ]; then
        echo "Committing and pushing changes from session $i..."
        git add -A
        git commit -m "Session $i: progress on LSP visual docs" --allow-empty-message || true
        git push origin "$BRANCH" || echo "WARNING: push failed, continuing..."
    else
        echo "No uncommitted changes after session $i."
    fi

    # Brief pause between sessions
    sleep 2
done

echo ""
echo "=== Loop complete ==="
echo "Check the plan: $PLAN_FILE"
echo "Check the branch '$BRANCH' for full progress."
