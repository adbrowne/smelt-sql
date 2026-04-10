---
description: Only invoke when /rpi-handoff is explicitly requested by the user. Validate that an implementation plan was correctly executed
model: opus
---

# Validate Plan Implementation

You are tasked with verifying that an implementation plan was correctly executed. Be thorough — good validation catches issues before they compound.

## Input

$ARGUMENTS

This should be a path to a plan file (e.g., `docs/plans/20260331-feature.md`).

## Process

### Step 1: Gather Context

1. **Read the plan file** completely.
2. **Check git history** for implementation commits:
   ```bash
   git log --oneline -n 20
   ```
3. **Run all automated checks**:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --all-targets
   cargo test
   ```

### Step 2: Phase-by-Phase Verification

For each phase in the plan:

1. **Check completion status**: Look for `- [x]` checkmarks.
2. **Verify actual code matches**: Read the modified files and confirm the changes described in the plan are present.
3. **Run phase-specific checks**: Execute any phase-specific verification commands.
4. **Check for regressions**: Look for unintended side effects.

Spawn parallel subagents if multiple phases need independent verification.

### Step 3: Generate Validation Report

Present findings in this format:

```markdown
## Validation Report: {Plan Name}

### Implementation Status
- Phase 1: {Name} — {Fully implemented / Partially implemented / Missing}
- Phase 2: {Name} — {status}
...

### Automated Verification
- `cargo fmt --all -- --check` — {PASS/FAIL}
- `cargo clippy --all-targets` — {PASS/FAIL + detail if failed}
- `cargo test` — {PASS/FAIL + detail if failed}

### Code Review Findings

**Matches Plan**:
- {What was implemented correctly, with file:line refs}

**Deviations from Plan**:
- {Any differences, whether improvements or issues}

**Potential Issues**:
- {Edge cases, missing error handling, etc.}

### Manual Testing Required
- [ ] {Item that needs human verification}
- [ ] {Another item}

### Recommendations
- {Any follow-up work needed}
```

### Step 4: Update Plan Status

If all phases pass validation, update the plan's status:
```
**Status**: Draft  →  **Status**: Validated
```

## Important Rules

1. **Run all automated checks.** Don't skip verification commands.
2. **Verify against the plan, not just "does it work."** The plan is the specification.
3. **Be honest about gaps.** If something is incomplete, say so clearly.
4. **Check the desired end state.** Compare current code against the plan's "Desired End State" section.
