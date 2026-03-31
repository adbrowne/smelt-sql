# ACE-FCA Guide for smelt

A practical guide to using Advanced Context Engineering with Frequent Intentional Compaction
for development on the smelt codebase.

Based on [ACE-FCA](https://github.com/humanlayer/advanced-context-engineering-for-coding-agents/blob/main/ace-fca.md)
by [Dex Horthy](https://x.com/dexhorthy) of [HumanLayer](https://www.humanlayer.dev/).

## Why This Matters

AI coding agents are stateless functions. The context window is your **only lever** for output
quality. When context fills with noise — file searches, test output, stale conversation — the
agent gets worse. Dex Horthy calls the region above ~40% utilization the **"dumb zone"**: the
model still works, but with diminishing returns.

The fix isn't waiting for smarter models. It's **designing your workflow around context management**.

### The Leverage Hierarchy

A bad line of research leads to thousands of bad lines of code. A bad line of a plan leads to
hundreds of bad lines of code. A bad line of code is just a bad line of code.

Focus your review effort where it has the most impact:

```
Research  →  Plan  →  Code
  1000x       100x      1x    (relative leverage of human review)
```

## The Workflow

The full pipeline produces structured markdown artifacts at each stage. Each stage runs in a
fresh (or compacted) context, consuming the artifact from the previous stage.

```
/research  →  /plan  →  /implement  →  /validate  →  /handoff
    ↑            ↑
    |         /iterate-plan
    |
  (human review gate)
```

### Stage 1: Research (`/research`)

**Purpose**: Understand the codebase as it exists today. No suggestions, no critiques — just
document what's there.

```
/research How does the parser handle error recovery in smelt-parser?
```

This spawns parallel subagents to explore the codebase and produces a structured document at
`docs/research/YYYY-MM-DD-topic.md` containing:
- Key files with line references
- Architecture and data flow
- Current behavior
- Related patterns and test coverage
- Open questions

**Human review point**: Read the research output. Is it accurate? Does it cover the right
scope? If it went in the wrong direction, throw it away and re-steer with better framing.
This is far cheaper than building on bad research.

### Stage 2: Plan (`/plan`)

**Purpose**: Design an explicit, verifiable implementation plan.

```
/plan docs/research/2026-03-31-error-recovery.md
```

The agent reads your research, presents its understanding, and asks for confirmation before
writing the plan. The output goes to `docs/plans/YYYYMMDD-description.md` and includes:
- Overview and desired end state
- Explicit scope boundaries ("What We're NOT Doing")
- Phased implementation with file-level specifics
- Verification steps for each phase (cargo fmt, clippy, test)

**Human review point**: This is the highest-leverage review. Read ~200 lines of plan instead
of reviewing thousands of lines of generated code. Check:
- Does the approach make sense?
- Are the scope boundaries right?
- Are there risks the plan doesn't address?

### Stage 2b: Iterate (`/iterate-plan`)

**Purpose**: Refine a plan based on feedback without starting over.

```
/iterate-plan docs/plans/20260331-error-recovery.md - split Phase 2 into parser and AST phases
```

Makes surgical edits to an existing plan. Only researches new code if the changes require it.

### Stage 3: Implement (`/implement`)

**Purpose**: Execute the plan phase by phase with verification gates.

```
/implement docs/plans/20260331-error-recovery.md
```

The agent:
1. Reads the plan and identifies the next incomplete phase
2. Implements that phase
3. Runs verification (`cargo fmt`, `cargo clippy`, `cargo test`)
4. Updates checkboxes in the plan file
5. **Stops and waits for your confirmation** before the next phase

If reality diverges from the plan, the agent stops and presents the issue with options.

### Stage 4: Validate (`/validate`)

**Purpose**: Verify the implementation matches the plan's specification.

```
/validate docs/plans/20260331-error-recovery.md
```

Produces a validation report comparing the plan's desired end state against the actual code.
Runs all automated checks and lists any manual testing needed.

### Stage 5: Handoff (`/handoff`)

**Purpose**: Compact your session context for continuity.

```
/handoff
```

Creates a structured handoff document at `docs/handoffs/YYYY-MM-DD-description.md` with:
- Task status (completed, in progress, planned)
- Recent changes with file:line references
- Key learnings that aren't obvious from the code
- Ordered next steps

To resume in a new session, just read the handoff document.

## Frequent Intentional Compaction

The core technique: **don't let context fill up organically**. Proactively compact at the
40-60% utilization mark by producing structured artifacts.

### When to compact

- After completing a research or planning phase (the artifact IS the compaction)
- When you notice the agent repeating itself or losing track
- Before switching to a different area of the codebase
- At natural task boundaries

### How to compact in Claude Code

Use `/compact` with a focus hint:

```
/compact focus on the implementation plan at docs/plans/20260331-feature.md,
the list of modified files, and current test status
```

### What to preserve during compaction

Add these standing instructions to guide compaction:
- Full list of modified files
- Current test/build status
- Plan progress (which phases are done)
- Any decisions made and their rationale

## Tips for Effective Use

### Start with research, even if you think you know the answer

The research phase often reveals surprising details. In the original ACE-FCA case study,
the first research attempt concluded the bug was invalid — but re-steering the research
with better framing found the real issue and led to a merged fix.

### Review research and plans, not code

Reading 200 lines of a plan is more valuable than reading 2000 lines of generated code.
If the plan is good, the code will be good. If the plan is wrong, no amount of code
review will save it.

### Be willing to throw away and restart

Bad context is worse than no context. If research went in the wrong direction, discard
it and re-steer. If a plan doesn't feel right, start fresh with better constraints.
The cost of restarting a phase is low compared to building on a bad foundation.

### Use parallel runs for high-stakes decisions

For important plans, you can run two planning sessions in parallel — one with research,
one without — and compare the results. The research-informed plan is usually better, and
the comparison validates this.

### One domain expert is required

The ACE-FCA approach amplifies expertise but doesn't replace it. You need at least one
person who understands the codebase and domain to steer the research and review the plans.

## Artifact Locations

| Artifact | Directory | Convention |
|----------|-----------|------------|
| Research docs | `docs/research/` | `YYYY-MM-DD-topic.md` |
| Implementation plans | `docs/plans/` | `YYYYMMDD-description.md` |
| Handoff docs | `docs/handoffs/` | `YYYY-MM-DD-description.md` |

All artifacts are committed to the repo, creating institutional memory.

## Example: Adding Window Function Support

Here's how you'd use the full workflow:

```bash
# 1. Research how functions are currently parsed and type-checked
/research How are SQL functions parsed in smelt-parser and type-checked in smelt-db?

# 2. Review the research doc, then plan the implementation
/plan docs/research/2026-03-31-sql-functions.md

# 3. Review the plan, iterate if needed
/iterate-plan docs/plans/20260331-window-functions.md - add a phase for LSP completions

# 4. Implement phase by phase
/implement docs/plans/20260331-window-functions.md

# 5. Validate the full implementation
/validate docs/plans/20260331-window-functions.md

# 6. If you need to pause, create a handoff
/handoff window function implementation
```

## Credits & Sources

This workflow is adapted from the following sources:

- **[ACE-FCA: Getting AI to Work in Complex Codebases](https://github.com/humanlayer/advanced-context-engineering-for-coding-agents/blob/main/ace-fca.md)** by Dex Horthy — the original essay that defines Frequent Intentional Compaction and the three-phase workflow
- **[HumanLayer slash command templates](https://github.com/humanlayer/humanlayer/tree/main/.claude/commands)** — the 27 production slash commands that HumanLayer uses internally, which our 6 commands are adapted from
- **[Skill Issue: Harness Engineering for Coding Agents](https://www.humanlayer.dev/blog/skill-issue-harness-engineering-for-coding-agents)** — HumanLayer's expanded workflow with Linear integration showing the full spec → research → plan → implement → review pipeline
- **[Y Combinator talk by Dex Horthy](https://hlyr.dev/ace)** — video presentation on spec-first development and the "dumb zone" concept
- **[Effective Context Engineering (Anthropic)](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)** — Anthropic's own guidance on context management for AI agents
- **[Claude Code Best Practices](https://code.claude.com/docs/en/best-practices)** — official guidance on CLAUDE.md, subagents, and compaction
