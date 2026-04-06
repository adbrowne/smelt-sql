---
description: Create an implementation plan from research or a task description
model: opus
---

# Create Implementation Plan

You are tasked with creating a detailed implementation plan for a feature or change in the smelt codebase. This plan will be the source of truth for implementation — make it precise enough that an agent (or future you) can execute it without guessing.

## Input

$ARGUMENTS

This may be:
- A path to a research document (e.g., `docs/research/2026-03-31-error-recovery.md`)
- A task description
- A ticket or issue reference

## Process

### Step 1: Gather Context

1. **If a research document was provided**: Read it FULLY. This is your primary context.
2. **If a task description was provided**: Spawn a focused research subagent to understand the relevant code before planning.
3. **Read all files mentioned** in the research or task description.

### Step 2: Present Understanding

Before writing any plan, present your understanding to the user:

```
Based on my analysis, here's what I understand:

**Goal**: {What we're trying to achieve}
**Current State**: {How things work today}
**Key Files**: {Files that will need to change}
**Approach**: {High-level strategy}

Does this match your intent? Any corrections or constraints I should know about?
```

Wait for user confirmation or correction before proceeding.

### Step 3: Design Options (if applicable)

If there are meaningful design choices, present 2-3 options with trade-offs:

```
**Option A**: {Description}
- Pro: ...
- Con: ...

**Option B**: {Description}
- Pro: ...
- Con: ...

Which approach do you prefer?
```

Skip this for straightforward tasks where the approach is obvious.

### Step 4: Write the Plan

Create the plan at `docs/plans/YYYYMMDD-{description}.md` using today's date and the existing naming convention.

Use this structure:

```markdown
# Plan: {Title}

**Date**: {YYYY-MM-DD}
**Research**: {path to research doc, if any}
**Status**: Draft

## Overview

{1-2 paragraph description of what this plan achieves and why}

## Current State

{Brief description of how things work today, with file:line references}

## Desired End State

{What the codebase should look like when this plan is fully implemented}

## What We're NOT Doing

{Explicit scope boundaries — things that are out of scope}

## Implementation Phases

### Phase 1: {Name}

**Files to modify**:
- `path/to/file.rs` — {what changes}

**Changes**:
1. {Specific change with enough detail to implement}
2. {Next change}

**Verification**:
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets` (no warnings)
- [ ] `cargo test` (all pass)
- [ ] {Phase-specific check}

### Phase 2: {Name}

{Same structure as Phase 1}

...

## Testing Strategy

{How to verify the full implementation end-to-end}

## Risks & Mitigations

{Known risks and how to handle them}
```

### Step 5: Present the Plan

After writing, summarize:
- Number of phases and their names
- Key files affected
- The plan file path
- Ask if any adjustments are needed before implementation

## Important Rules

1. **Be precise.** Specify exact files, functions, and what changes. "Update the parser" is too vague. "Add a `parse_window_function` method to `crates/smelt-parser/src/parser.rs` after `parse_function_call`" is correct.
2. **Every phase must be independently verifiable.** Include `cargo fmt`, `cargo clippy`, and `cargo test` in every phase's verification.
3. **Scope clearly.** The "What We're NOT Doing" section prevents scope creep.
4. **Interactive first.** Always present understanding and get confirmation before writing.
5. **Reference research.** If working from a research doc, cite it and link to specific sections.
6. **Follow existing conventions.** Check `docs/plans/` for examples of prior plans in this project.
