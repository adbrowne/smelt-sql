---
description: Only invoke when /rpi-implement is explicitly requested by the user. Implement a plan phase by phase with verification between each step
model: opus
---

# Implement Plan

You are tasked with executing an implementation plan phase by phase. Follow the plan precisely. When reality diverges from the plan, STOP and discuss — don't improvise.

## Input

$ARGUMENTS

This should be a path to a plan file (e.g., `docs/plans/20260331-feature.md`).

## Process

### Step 1: Read the Plan

1. Read the plan file COMPLETELY.
2. Check for existing checkmarks (`- [x]`) indicating completed phases.
3. Identify the next incomplete phase.
4. If ALL phases are complete, inform the user and suggest running `/validate`.

### Step 2: Confirm Starting Point

```
I've read the plan at `{path}`.

**Completed phases**: {list or "none"}
**Next phase**: Phase {N}: {Name}
**Files to modify**: {list}

Ready to begin Phase {N}?
```

Wait for user confirmation.

### Step 3: Implement Current Phase

For each phase:

1. **Read all files** that will be modified BEFORE making changes.
2. **Implement the changes** as specified in the plan.
3. **Run verification checks** from the plan:
   ```bash
   cargo fmt --all
   cargo clippy --all-targets
   cargo test
   ```
4. **Fix any issues** found by verification.
5. **Update the plan** — check off the completed verification items:
   ```
   - [x] `cargo fmt --all -- --check`
   - [x] `cargo clippy --all-targets` (no warnings)
   - [x] `cargo test` (all pass)
   ```

### Step 4: Report Phase Completion

```
Phase {N} complete.

**Changes made**:
- {file}: {what changed}
- {file}: {what changed}

**Verification**:
- [x] cargo fmt — passed
- [x] cargo clippy — passed
- [x] cargo test — passed
- [x] {phase-specific check} — {result}

Ready to proceed to Phase {N+1}?
```

**STOP here and wait for user confirmation before proceeding to the next phase.**

### Step 5: Repeat for Each Phase

Continue through all phases, pausing for confirmation between each one.

### Step 6: Final Summary

After all phases are complete:

```
All phases complete!

**Summary of changes**:
- {High-level list of what was implemented}

**Suggested next steps**:
- Run `/validate {plan-path}` to verify the full implementation
- Review the changes with `git diff`
- Commit when ready
```

## When Reality Diverges from the Plan

If you encounter something unexpected during implementation:

1. **STOP immediately.** Do not improvise or guess.
2. **Present the issue clearly**:
   ```
   I've hit a divergence from the plan.

   **Plan says**: {what the plan expected}
   **Reality**: {what I actually found}

   Options:
   A) {Suggested adaptation}
   B) {Alternative approach}
   C) Update the plan first with `/iterate-plan`

   How would you like to proceed?
   ```
3. **Wait for user direction** before continuing.

## Important Rules

1. **Follow the plan.** The plan is the source of truth. Don't add extras.
2. **One phase at a time.** Never skip ahead or combine phases.
3. **Verify every phase.** Run ALL verification steps, even if you're confident.
4. **Pause between phases.** Human review between phases catches drift early.
5. **Stop on divergence.** Don't improvise. Ask.
6. **Update checkboxes.** Mark verification items as complete in the plan file.
