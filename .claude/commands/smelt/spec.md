---
description: Draft or update a feature spec under docs/specs/
model: opus
---

# Draft or Update a Feature Spec

You are tasked with producing a normative spec for a smelt feature at `docs/specs/<feature>.md`. The spec is the canonical answer to "how does this feature work?". Plans cite it; `smelt:validate` checks code and docs against it.

## Input

$ARGUMENTS

This may be:
- A feature slug for a **new** spec (e.g., `incremental_models`)
- A path to an **existing** spec to update
- A free-form area to spec (you'll pick the slug)

## Process

### Step 1: Decide new vs update

If the spec already exists at `docs/specs/<feature>.md`, you're updating it — read it fully first. Otherwise you're drafting a new one — read `docs/specs/SPEC_TEMPLATE.md`.

### Step 2: Gather material from existing sources

The spec is normative, but it doesn't appear from nothing. Gather inputs in this order:

1. **The code itself** — primary implementation paths, public surface, error messages, tests. Read these in your own context, don't only delegate.
2. **`docs/DESIGN.md`** — search for relevant sections; this is where most legacy design intent lives.
3. **`docs/research/*` and `docs/plans/*`** — find prior work in this area; cite them under References (history).
4. **`docs-site/docs/*`** — find the user-facing pages that document this feature; they go under References (user docs).
5. **`README.md`** — any current-state claims relevant to this feature.

Spawn parallel `Explore` subagents only if the area spans many files; otherwise read directly.

### Step 3: Present your understanding before writing

Before writing the spec, summarize to the user:

```
**Feature**: {slug}
**Status**: {stable | experimental | deprecated}
**Surface I see today**: {short list — YAML fields, CLI flags, syntax}
**Semantics I see today**: {short list — key rules, invariants}
**Known divergences I noticed**: {anything where code disagrees with DESIGN.md / docs / itself}
**Source files I read**: {paths}

Anything missing or wrong before I draft the spec?
```

Wait for the user to confirm or correct. The spec is normative — drafting it on a wrong understanding is worse than not drafting it.

### Step 4: Write the spec

Use `docs/specs/SPEC_TEMPLATE.md` as the starting structure. Hard rules:

- **Surface section is exhaustive** for what callers depend on (users for feature specs; other components for system specs). If you skip something here, `smelt:validate` won't catch its drift.
- **Semantics section uses normative language** ("must", "must not", "if X then Y"). Avoid hedging like "should usually" — pick a rule.
- **Design section captures the why.** Record the load-bearing decisions and the alternatives rejected, so the spec doesn't read as "rules from nowhere". Keep it dense — link to `docs/research/` for deeper justification.
- **Known Divergences is honest.** If DESIGN.md says one thing and code does another, name it. Don't pretend the spec matches reality if it doesn't.
- **References point to current paths**, not commit-pinned ones. Plans go under "Plans (history)" oldest first.
- **Frontmatter `last_reviewed` = today's date** (use `date -I`).

### Step 5: Update the index, if any

If `docs/specs/README.md` exists as an index, add an entry. If not, don't create one — only create indexing infrastructure when there are 3+ specs.

### Step 6: Report

```
Spec written: docs/specs/{slug}.md ({N} lines)

Key claims:
- {1-3 normative rules from Semantics}
- {1-2 invariants from Constraints}

Known divergences captured:
- {list}

Suggested next steps:
- Review the Surface section against your intent
- If updating an existing feature, run /smelt:validate {slug} to see current drift
- If planning new work, run /smelt:plan {slug} to derive a plan from the spec diff
```

## Important Rules

1. **Spec is normative.** It describes what is true, not what was true. If you're unsure, flag it under Known Divergences instead of guessing.
2. **Capture design and invariants; skip implementation choices.** The spec records the surface, the rules, the design rationale (why this shape, what alternatives were rejected), and the invariants the implementation must preserve. It does *not* prescribe specific data structures, function names, or code layout — those change without spec drift. "What vs how" collapses two distinct things; the right cut is "design vs implementation".
3. **Read source files in your own context first.** A spec written from delegated summaries is unreliable.
4. **Don't refactor while specing.** Note divergences; let the next plan fix them.
5. **Spec stays under ~300 lines for most features.** If it's growing past that, consider splitting (e.g., `incremental_models.md` + `incremental_strategies.md`).
