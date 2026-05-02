---
description: Execute a spec-anchored plan phase by phase using implementer + reviewer subagents
model: opus
---

# Implement a Spec-Anchored Plan

You are tasked with executing a plan produced by `/smelt:plan` to completion. The spec referenced by the plan is the correctness oracle — when in doubt, the spec wins, not the code that exists today.

## Input

$ARGUMENTS

A path to a plan file (e.g., `docs/plans/20260427-incremental_models.md`).

## Process

### Step 1: Load plan and spec

1. Read the plan file completely.
2. Read the spec referenced in the plan header (`docs/specs/<slug>.md`) completely. This is the oracle.
3. Confirm the working tree is clean and you're on the tracking branch named in the plan header. If not, stop and ask.

### Step 2: Find the next phase

Look at the Progress tracking table. Pick the first phase with status `pending`. If all phases are `done`, jump to "Final verification" below.

### Step 3: Per-phase loop

For each `pending` phase:

#### 3a. Implementer subagent (red-green TDD)

Spawn a fresh `general-purpose` subagent with a self-contained brief. The brief must include:

- The phase's section verbatim (Goal, Pre-conditions, TDD tests, Implementation shape, Critical files, Docs touched, Commit message).
- The spec sections the phase implements (paste them, don't link only — the subagent has no other context).
- The standing conventions from the plan's Execution prompt (red-green TDD, real-fixture tests, scope discipline, architectural invariants from `CLAUDE.md`).
- An explicit instruction: write the listed TDD tests **as failing tests first**, then implement until green. Tree must be left passing:
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets` (zero warnings)
  - `cargo test`
  - `cargo test -p smelt-cli --test example_diagnostics`
- The list of files the subagent is **allowed to touch** (Critical files + Docs touched). Out-of-scope edits should be reported, not made.

Wait for the subagent to report. The expected report is: tests written, tests now green, all CI checks pass, commit ready (do **not** have the subagent commit — the main session commits in step 3d).

#### 3b. Reviewer subagent (material findings only)

Spawn a fresh `general-purpose` subagent as reviewer. Its brief:

- The phase's Review checklist verbatim.
- The spec sections the phase implements (paste them).
- The full diff produced by the implementer: `git diff` against the last phase's commit (or against the plan's commit for Phase 1).
- An instruction to report **only material findings**: correctness against spec, architectural invariants violated, missing TDD coverage, scope creep beyond the phase's stated files. Style nits and naming preferences are out of scope.

Wait for the reviewer to report.

#### 3c. Iterate

If the reviewer flags material findings, dispatch the implementer again with the findings and the requirement to keep tests green. Repeat 3a → 3b until the reviewer comes back clean. Do not advance with open material findings.

If the reviewer surfaces the same finding twice across implementer passes, **pause and ask the user**. Don't loop indefinitely.

#### 3d. Record + commit + push

1. Update the Progress tracking row for this phase: `status: pending` → `status: done`, fill `Date` (use `date -I`), leave `Commit` empty for now.
2. Stage the implementer's changes plus the plan-file edit.
3. Commit using the phase's `Commit.` line verbatim, with the standard Claude Code co-author trailer.
4. Capture the resulting commit sha and update the Progress tracking row.
5. Push to the tracking branch.

#### 3e. Advance

Proceed to the next `pending` phase immediately. Do not pause between phases unless a pause condition fired.

### Step 4: Pause conditions

Stop the loop and ask the user when:

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule (likely the spec is wrong — run `/smelt:spec` to update before continuing).
- A pre-existing failure on the branch surfaces that is not caused by this phase's changes.
- A phase's required files don't exist and the plan didn't anticipate creating them.

### Step 5: Final verification

Once every phase is `done`:

1. Run the plan's Verification section commands.
2. Run `/smelt:validate <plan>` and report the drift report to the user.
3. Confirm the user docs in the plan have all been updated (cross-check spec Surface section vs `docs-site/`).

Report:
```
Plan complete: docs/plans/{file}.md
Phases: {N}/{N} done
Validation: {pass | drift report attached}
Tracking PR: {URL or branch}
```

## Important Rules

1. **The spec is the oracle.** When the plan and the code disagree, follow the plan; when the plan and the spec disagree, stop and ask.
2. **Implementer and reviewer are separate subagents.** Never let the implementer self-review.
3. **Real-fixture tests are mandatory.** Unit tests on AST nodes are not enough — the phase must exercise its feature in `examples/`.
4. **Atomic commits.** One commit per phase, never amend a prior phase.
5. **Never `--no-verify`, never force-push the tracking PR.**
6. **Don't widen the implementer's allowed-files list.** If the phase scope is wrong, fix the plan, don't sneak edits in.
7. **Update the plan file in the same commit as the code.** Progress tracking lives with the change.
