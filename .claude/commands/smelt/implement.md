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
2. Note the spec path referenced in the plan header (`docs/specs/<slug>.md`) — it is the oracle. Do **not** read it fully into your own context: the subagents read the sections they need themselves (see 3a/3b), and you only consult specific sections when adjudicating a disagreement. Keeping the spec out of the orchestrator context is deliberate — every token you hold is re-read on every subsequent turn by you, and duplicated into nothing; every token you *paste* into a subagent brief is paid twice more.
3. Confirm the working tree is clean and you're on the tracking branch named in the plan header. If not, stop and ask.
4. If you need to locate code (where a function lives, which crates consume an API) beyond what the phase's Critical-files list already states, delegate that to an **Explore** agent and keep only its conclusion — don't grep and read files into your own context.

### Step 2: Find the next phase

Look at the Progress tracking table. Pick the first phase with status `pending`. If all phases are `done`, jump to "Final verification" below.

### Step 3: Per-phase loop

For each `pending` phase:

#### 3a. Implementer subagent (red-green TDD)

Spawn a fresh `general-purpose` subagent (use `model: sonnet` on the Agent tool unless the plan header says otherwise — this command's own `model: opus` is for orchestration, not delegation). **Brief by reference, not by paste** — the subagent reads files itself; pasting the same content into the brief bills it once in your context and again in the subagent's. The brief must include:

- A pointer to the phase: the plan path and the exact phase heading (e.g. `docs/plans/20260707-x.md` § "Phase SA6"). Instruct the subagent to read that section completely — Goal, Pre-conditions, TDD tests, Implementation shape, Critical files, Docs touched, Commit message — plus the plan's "Execution prompt" conventions section (red-green TDD, real-fixture tests, scope discipline, architectural invariants from `CLAUDE.md`).
- A pointer to the spec sections the phase implements: the spec path and the section names (from the phase's Review checklist / your knowledge of the plan). The spec is the correctness oracle; the subagent must read those sections before writing tests.
- An explicit instruction: write the listed TDD tests **as failing tests first**, then implement until green. Finish by running the bundled gate and leaving it green:
  - `bash .claude/scripts/verify-phase.sh` (fmt + clippy zero-warnings + `cargo test` + example_diagnostics, failures-only output)
- The allowed-files rule: only the phase's Critical files + Docs touched may be edited (the subagent reads the list from the phase section). Out-of-scope edits should be reported, not made.
- **Timeless-oracle rule for spec/docs-site edits (CLAUDE.md).** The phase section the subagent reads uses phase vocabulary — that vocabulary belongs to the *plan only*. When the implementer edits `docs/specs/<slug>.md` or `docs-site/docs/...`, those edits must describe the feature as if it has always existed: no `### Phase A — ...` headings, no `(Phase B)` inline labels, no `[deferred to Phase E1]` callouts in body sections. Surface/Semantics/Design entries describe behaviour. Implementation gaps go in the spec's **Known Divergences** in behavioural terms (with a plan-file link), not as plan-phase status notes in body sections. Code-comment edits follow the same rule: describe the code, not which plan phase introduced it.

Wait for the subagent to report. The expected report is: tests written, tests now green, all CI checks pass, commit ready (do **not** have the subagent commit — the main session commits in step 3d).

#### 3b. Reviewer subagent (material findings only)

Spawn a fresh `general-purpose` subagent (use `model: sonnet` on the Agent tool unless the plan header says otherwise) as reviewer. Brief by reference here too — and in particular **do not run `git diff` yourself to paste it**; the diff would sit in your context for every remaining turn. Its brief:

- A pointer to the phase's Review checklist: plan path + phase heading (the reviewer reads it itself).
- A pointer to the spec sections the phase implements (spec path + section names — the reviewer reads them itself).
- The base ref to diff against: instruct the reviewer to run `git diff <last-phase-sha>..HEAD` itself (or against the plan's commit for Phase 1) and review that diff. Give it the sha, not the diff.
- An instruction to report **only material findings**: correctness against spec, architectural invariants violated, missing TDD coverage, scope creep beyond the phase's stated files. Style nits and naming preferences are out of scope.
- An explicit Timeless-oracle check: scan diffs to `docs/specs/` and `docs-site/` for `Phase [A-Z0-9]`, `(Phase X)`, `[deferred to Phase ...]`, `Phase 0 scaffold`, or other plan-vocabulary leakage in body sections. Flag each as a material finding (the rule lives in `CLAUDE.md`). Phase numbers are tolerated in **Known Divergences** when paired with a plan-file link; everywhere else in the spec/docs-site body, they are drift.

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
