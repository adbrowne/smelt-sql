---
description: Create a handoff document to preserve context for session continuity
model: opus
---

# Create Handoff

You are tasked with writing a handoff document to pass your work to another agent (or yourself) in a new session. The goal is to compact and summarize your context without losing key details.

## Input

$ARGUMENTS

Optional: a description of what you were working on. If not provided, infer from the current session context.

## Process

### Step 1: Gather State

1. **Check current git state**:
   ```bash
   git status
   git log --oneline -n 10
   git branch --show-current
   git rev-parse --short HEAD
   ```
2. **Review what was accomplished** in this session.
3. **Identify any in-progress work** that isn't complete.

### Step 2: Write Handoff Document

Create the document at `docs/handoffs/YYYY-MM-DD-{description}.md` using today's date.

```markdown
# Handoff: {Brief Description}

**Date**: {YYYY-MM-DD HH:MM}
**Branch**: {current branch}
**Commit**: {short hash}

## Tasks

| Task | Status |
|------|--------|
| {Task 1} | Completed / In Progress / Planned |
| {Task 2} | ... |

**Working from**: {path to plan or research doc, if any}
**Current phase**: {which phase of the plan, if applicable}

## Recent Changes

{List files changed with brief descriptions, using file:line references}

- `crates/smelt-parser/src/parser.rs:142` — Added window function parsing
- `crates/smelt-db/src/type_inference.rs:567` — Extended type inference for OVER clause

## Key Learnings

{Important things discovered during this session that aren't obvious from the code}

- The parser's error recovery relies on sync points at `;` and `)` — new syntax must account for this
- Type inference for aggregates is handled in `infer_function_call`, not a separate path

## Artifacts

{Exhaustive list of files produced or updated}

- `docs/plans/20260331-window-functions.md` — Implementation plan
- `docs/research/2026-03-31-window-functions.md` — Research document

## Next Steps

{Ordered list of what to do next}

1. Continue with Phase 3 of the plan at `docs/plans/20260331-window-functions.md`
2. Run `/implement docs/plans/20260331-window-functions.md` to resume
3. {Any specific concerns or blockers}
```

### Step 3: Present to User

```
Handoff created at `{path}`.

To resume in a new session, start by reading this handoff document:
> Read docs/handoffs/{filename} and resume the work described there.
```

## Important Rules

1. **More information, not less.** This is a minimum template — add detail where needed.
2. **Be precise.** Include file:line references, not vague descriptions.
3. **Avoid large code blocks.** Reference files instead of pasting code.
4. **Include learnings.** The most valuable part of a handoff is what you learned that isn't in the code.
