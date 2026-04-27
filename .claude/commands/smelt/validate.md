---
description: Verify that implementation and user docs match a spec
model: opus
---

# Validate Implementation Against a Spec

You are tasked with producing a drift report comparing a `docs/specs/<feature>.md` spec against the current implementation and user docs. Be honest — drift caught early is cheap.

## Input

$ARGUMENTS

This may be:
- A spec slug (e.g., `incremental_models`) — implies `docs/specs/<slug>.md`
- A path to a spec file
- A path to a plan file — implies validating the spec referenced by that plan

## Process

### Step 1: Load the spec

1. Read the spec file completely. This is the oracle.
2. Note the `last_reviewed` date and the References block (Code, Tests, User docs).

### Step 2: Run automated checks

```bash
cargo fmt --all -- --check
cargo clippy --all-targets
cargo test
cargo test -p smelt-cli --test example_diagnostics
```

Capture pass/fail for each. A FAIL here doesn't necessarily mean spec drift, but it must be reported.

### Step 3: Surface drift check

For each item in the spec's **Surface** section:
- Confirm it exists in the code (search for the YAML field, CLI flag, syntax token, error code, etc.).
- Confirm the corresponding `docs-site/` page documents it consistently.

For each item in `docs-site/` referenced by the spec:
- Confirm everything documented there is present in the spec's Surface section. Pages can document something the spec doesn't if it's marked deprecated — flag that explicitly.

### Step 4: Semantics drift check

For each normative rule in the spec's **Semantics** section:
- Identify the test(s) that exercise it (use the spec's References → Tests as the starting point).
- Confirm the test exists and asserts the rule.
- If the rule is not test-covered, flag it.

For each invariant in **Constraints & Invariants**:
- Confirm the codebase still upholds it. Use grep / inspection. (Cannot prove all properties — prove the ones you can, flag the ones you can't.)

### Step 5: Freshness check

Compare `last_reviewed` to the most recent commit touching the spec's Reference → Code paths:

```bash
git log -1 --format=%cI -- crates/.../src/incremental.rs ...
```

If the code has changed substantively since `last_reviewed`, flag the spec as stale and recommend running `/smelt:spec`.

### Step 6: Generate the drift report

Write to stdout (and optionally to `docs/validations/YYYY-MM-DD-<slug>.md` if the user wants it persisted — ask):

```markdown
## Drift Report: {slug}

**Spec**: docs/specs/{slug}.md (last_reviewed: {date})
**Date**: {YYYY-MM-DD}

### Automated checks
- cargo fmt — {PASS/FAIL}
- cargo clippy — {PASS/FAIL}
- cargo test — {PASS/FAIL with brief detail}
- example_diagnostics — {PASS/FAIL}

### Surface drift
- ✅ {Item that matches spec, code, and docs}
- ❌ {Item missing from code: spec says X, code does Y at file:line}
- ❌ {Item missing from docs-site: spec says X, docs-site/.../page.md doesn't mention it}
- ⚠️  {Item present in docs but not in spec — likely undocumented spec change}

### Semantics drift
- ✅ {Rule covered by test_name at file:line}
- ❌ {Rule not test-covered: spec says X under Semantics § Y}
- ❌ {Implementation diverges from rule: spec says X, code at file:line does Y}

### Invariant drift
- ✅ {Invariant verifiably upheld}
- ⚠️  {Invariant not verifiable from inspection — flag for manual review}
- ❌ {Invariant violated at file:line}

### Freshness
- last_reviewed: {date}
- most recent code change: {date} at {path}
- Verdict: {fresh | stale — recommend /smelt:spec}

### Summary
- Drift items: {N total — X surface, Y semantics, Z invariants}
- Recommended next step: {/smelt:spec to update spec | /smelt:plan {slug} to fix drift | none}
```

### Step 7: Suggest next steps

Based on the report:
- **Spec is stale** → recommend `/smelt:spec <slug>`.
- **Code drifted from spec** → recommend `/smelt:plan <slug>` with the drift as the change scope.
- **Docs drifted from spec** → recommend a small targeted plan (or a quick docs-only fix) and flag it.
- **No drift** → say so and stop.

## Important Rules

1. **Be specific.** Every drift item points to a file:line and a spec section. "Things look wrong" is not useful.
2. **Don't fix drift in this command.** This is a report, not a remediation. Recommend `/smelt:plan` for fixes.
3. **Test coverage matters.** A spec rule with no test is itself a drift item — the spec is unenforced.
4. **Honest verdict.** If you can't verify an invariant from inspection, say so. Don't paper over.
5. **Validate against the spec, not the code.** The spec is the oracle. If the code does something the spec doesn't say, that's drift, not "extra functionality".
