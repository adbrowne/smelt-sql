---
description: Generate an implementation plan from a spec diff
model: opus
---

# Generate an Implementation Plan from a Spec

You are tasked with producing a phased implementation plan that drives the smelt code (and user docs) to match a spec. The spec is the source of truth — the plan is just an execution script.

## Input

$ARGUMENTS

This may be:
- A spec slug (e.g., `incremental_models`) — implies `docs/specs/<slug>.md`
- A path to a spec file
- A spec slug + a free-form note about scope (e.g., "incremental_models, just the partition rewrite")

## Process

### Step 1: Load the spec as primary context

1. Read `docs/specs/<slug>.md` **completely** in your own context. This is the brief — do not re-derive it from code.
2. Read the spec's References → Code paths to ground yourself in current implementation.
3. If the spec has a `last_reviewed` date older than 30 days, warn the user and offer to run `/smelt:spec` first.

### Step 2: Compute the spec diff

The change description is the spec diff, not a free-form prompt:

```bash
git log -1 --format=%H -- docs/specs/<slug>.md
git diff <last-spec-commit>..HEAD -- docs/specs/<slug>.md
```

If the spec was just updated (uncommitted), use the working-tree diff. If the spec is brand new, the "diff" is the entire spec (this is a greenfield plan).

If the spec hasn't changed and the user is planning anyway, ask what change they're after — they probably need to run `/smelt:spec` first.

### Step 3: Present understanding before writing

```
**Spec**: docs/specs/{slug}.md
**Spec diff scope**: {1-2 sentences on what the spec change requires}
**Code areas affected**: {paths from spec References → Code}
**User docs to update**: {paths from spec References → User docs}
**Proposed phase shape**: {brief outline}

Anything missing before I write the plan?
```

Wait for confirmation.

### Step 4: Write the plan

Create the plan at `docs/plans/YYYYMMDD-<slug>.md`. Use the **mandatory plan structure** below. The template is encoded here; do not improvise.

#### Mandatory plan header

```markdown
# Plan: {Title}

**Date**: {YYYY-MM-DD}
**Spec**: [`docs/specs/{slug}.md`](../specs/{slug}.md)
**Spec diff**: {commit range, "uncommitted working tree", or "new spec"}
**Tracking PR / branch**: {PR # or branch name once known; placeholder ok}
**Docs**: code+docs   <!-- or "code-only" if no user-visible surface changes; see CLAUDE.md -->

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the spec at `docs/specs/{slug}.md` — it is the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `{tracking branch}`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature in `examples/`.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md` (e.g., `type_inference.rs` purity).
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Edits to `docs/specs/<slug>.md` and `docs-site/docs/...` describe the feature as if it has always existed — no `### Phase A — …` headings, no `(Phase B)` inline labels, no `[deferred to Phase E1]` callouts in spec/user-doc body. If a phase ships an incomplete surface, the *spec* records the gap under **Known Divergences** in behavioural terms (not phase terms). The plan's Progress tracking table is where "what landed when" lives.

---

## Context

{≤1 paragraph. Why this change, anchored to spec section names. Do not re-derive current state — point to spec.}

## Scope

### In scope (spec coverage)
- {Spec section: short description of what this plan implements}
- ...

### Explicitly deferred
- {Thing not in this plan, with the reason}
- ...

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | pending  |        |      |
| ...   | pending  |        |      |
```

#### Mandatory per-phase section

For each phase:

```markdown
### Phase N: {Name}

**Goal.** {1–2 sentences.}

**Pre-conditions.** {What must already be true. Reference earlier phases if needed.}

**TDD tests to write first.** Listed verbatim — write these as failing tests before any implementation:
- `crates/.../tests/...rs::test_name` — {what it asserts, including a real-fixture case from `examples/...`}
- ...

**Implementation shape.** {Sketch — function names, module boundaries, key signatures. Not prescriptive line-by-line.}

**Critical files (allowed to touch in this phase).**
- `crates/.../src/...rs` — {what changes}
- ...

**Docs touched (default, unless plan header is `Docs: code-only`).** *Write these as if the feature has always existed — no phase headings, no `(Phase X)` labels, no plan vocabulary. See Timeless-oracle rule.*
- `docs/specs/{slug}.md` — {Surface/Semantics section update — describe behaviour, not "this phase adds"}
- `docs-site/docs/...md` — {user-facing change, written as a feature description}

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Spec rules from {Semantics § X} are satisfied
- [ ] Architectural invariants honored
- [ ] No scope creep into later phases
- [ ] User docs updated to match Surface
- [ ] Spec + docs-site edits are timeless — no `Phase X` headings, no `(Phase X)` labels, no `[deferred to Phase Y]` callouts in body

**Commit.** `{short type-prefixed message — used verbatim}`
```

#### Mandatory tail sections

```markdown
## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:
- {Real-fixture test or example workspace command}
- `cargo test -p smelt-cli --test example_diagnostics`
- `/smelt:validate {slug}` reports zero drift
```

### Step 5: Present the plan

```
Plan written: docs/plans/{YYYYMMDD-slug}.md ({N} phases, {M} lines)

Phases:
1. {name}
2. {name}
...

Affects:
- Code: {top-level paths}
- Specs: docs/specs/{slug}.md (Surface § {sections})
- User docs: docs-site/docs/{paths}

Run /smelt:implement docs/plans/{file}.md to start execution.
```

## Important Rules

1. **Spec is primary context.** Plans cite spec sections; they do not restate the spec.
2. **No competitive analysis or "current state" prose.** That belongs in research / spec, not the plan.
3. **Every phase has TDD tests listed verbatim.** Red-green is non-negotiable.
4. **Every phase touches docs by default.** Opt out with `Docs: code-only` in the header — only for pure refactors / internal changes.
5. **Phases are atomic.** One commit per phase, never amend a prior phase.
6. **Don't widen scope.** If a phase needs something from a later phase to pass tests, the phase boundaries are wrong — fix the plan, don't reach forward.
7. **Plans should be ~30–50% shorter than pre-spec plans of the same scope.** If your plan is bloated with context, you're re-deriving the spec instead of citing it.
8. **Phase vocabulary stays in the plan.** Spec and user-doc edits ride alongside each phase but must read as timeless feature descriptions. See the Timeless-oracle rule in `CLAUDE.md`. The plan file is allowed (and expected) to use phase vocabulary; the artifacts it touches are not.
