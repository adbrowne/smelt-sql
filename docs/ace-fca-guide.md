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
/smelt:spec  →  /smelt:plan  →  /smelt:implement  →  /smelt:validate
                                                              ↑
                                                         (drift report
                                                          → next /smelt:spec
                                                            or /smelt:plan)
```

The four commands are spec-anchored: every plan cites a spec, every implementation is reviewed
against the spec, and validation reports drift between code, spec, and user docs.

### Stage 1: Spec (`/smelt:spec`)

**Purpose**: Capture or update the canonical answer to "how does this feature work?". The spec
is normative — implementation and user docs must match it.

```
/smelt:spec incremental_models
```

Reads code, `DESIGN.md`, prior plans, and `docs-site/` to produce a spec at
`docs/specs/<feature>.md` with sections:
- Surface (user-visible API: syntax, YAML fields, CLI flags, error messages)
- Semantics (formal rules, edge cases, failure modes)
- Constraints & Invariants (what must always hold; what's explicitly not supported)
- Known Divergences / Open Questions (where code differs from intent)
- References (code paths, tests, user docs, plan history)

**Human review point**: The spec is the source of truth. A wrong spec produces wrong plans
which produce wrong code. Read the Surface and Semantics sections carefully.

### Stage 2: Plan (`/smelt:plan`)

**Purpose**: Derive a phased implementation plan from a spec diff.

```
/smelt:plan incremental_models
```

The agent reads the spec as primary context, computes the diff from the last spec commit, and
produces a plan at `docs/plans/YYYYMMDD-<slug>.md`. Plans cite the spec rather than restate
it — they're typically much shorter than pre-spec plans of comparable scope.

Every plan includes:
- An execution prompt for a fresh Claude session
- Per-phase TDD tests listed verbatim (red before green)
- Per-phase critical files (the implementer is allowed to touch nothing else)
- Per-phase docs touched (default: spec Surface + corresponding `docs-site/` page)
- A Progress tracking table

**Human review point**: This is still the highest-leverage review. Check:
- Does each phase match a spec section?
- Are the listed TDD tests sufficient to prove the spec rule?
- Is anything sneaking past the phase boundary?

### Stage 3: Implement (`/smelt:implement`)

**Purpose**: Execute the plan phase by phase with implementer + reviewer subagents.

```
/smelt:implement docs/plans/20260427-incremental_models.md
```

For each phase the main session runs the loop:
1. **Implementer subagent** — writes the listed TDD tests as failing tests, makes them pass,
   leaves the tree green (`cargo fmt --check`, `cargo clippy` zero warnings, `cargo test`,
   `cargo test -p smelt-cli --test example_diagnostics`).
2. **Reviewer subagent** — gets the phase's review checklist plus the diff. Reports only
   material findings (correctness vs. spec, invariant violations, missing test coverage,
   scope creep). Style nits are out of scope.
3. **Iterate** until the reviewer comes back clean.
4. **Commit + push** atomically with the phase's commit message verbatim.

Pause conditions: same finding twice across implementer passes, TDD tests can't pass without
violating a spec rule, or a pre-existing failure surfaces.

### Stage 4: Validate (`/smelt:validate`)

**Purpose**: Produce a drift report comparing spec, code, and user docs.

```
/smelt:validate incremental_models
```

Checks:
- Surface drift (does every YAML field / CLI flag / error in the spec exist in code and docs?)
- Semantics drift (is every normative rule test-covered? does the code uphold it?)
- Invariant drift (are spec invariants still satisfied?)
- Freshness (`last_reviewed` vs. recent code changes)

Recommends the next step: re-spec if the spec is stale, plan if code drifted, or a small
docs-only fix if only `docs-site/` is out of sync.

### Optional helpers

- `/research` — pre-spec exploration when extracting a spec for an unspecified area
- `/iterate-plan` — surgical edits to an existing plan
- `/handoff` — session compaction artifact at `docs/handoffs/YYYY-MM-DD-description.md`

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
| Feature specs | `docs/specs/` | `<feature>.md` (normative; spec-first) |
| Research docs | `docs/research/` | `YYYY-MM-DD-topic.md` |
| Implementation plans | `docs/plans/` | `YYYYMMDD-description.md` |
| Handoff docs | `docs/handoffs/` | `YYYY-MM-DD-description.md` |

All artifacts are committed to the repo, creating institutional memory.

## Example: Adding Window Function Support

Here's how you'd use the full workflow:

```bash
# 1. Capture the spec (or update an existing one). For a brand-new feature
#    you may want a /research pass first to find the relevant code.
/smelt:spec window_functions

# 2. Review the spec, edit the Surface and Semantics sections to capture intent,
#    then derive a plan from the spec diff
/smelt:plan window_functions

# 3. Optionally iterate on the plan
/iterate-plan docs/plans/20260331-window-functions.md - split Phase 2 into parser and AST phases

# 4. Implement phase by phase (implementer + reviewer subagents per phase)
/smelt:implement docs/plans/20260331-window-functions.md

# 5. Validate the full implementation against the spec
/smelt:validate window_functions

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
