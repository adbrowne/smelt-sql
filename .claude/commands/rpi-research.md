---
description: Only invoke when /rpi-research is explicitly requested by the user. Research the codebase to understand a topic, feature, or problem area
model: opus
---

# Research Codebase

You are tasked with researching the smelt codebase to understand a specific topic. YOUR ONLY JOB IS TO DOCUMENT AND EXPLAIN THE CODEBASE AS IT EXISTS TODAY. Do not suggest improvements, critique code quality, or propose changes unless the user explicitly asks.

## Topic

$ARGUMENTS

## Process

### Step 1: Acknowledge and Clarify

If the topic is clear, proceed. If ambiguous, ask ONE focused clarifying question before starting.

### Step 2: Read Mentioned Files First

If specific files were mentioned, read them FULLY before spawning any subagents. This ensures you have primary context in your own window.

### Step 3: Parallel Research

Spawn up to 3 focused subagents to investigate in parallel. Each agent should have a specific search focus:

**Suggested agent roles** (adapt based on topic):
- **codebase-locator**: Find all files relevant to the topic. Search across crates (`smelt-parser`, `smelt-db`, `smelt-lsp`, `smelt-planner`, `smelt-cli`, `smelt-backend-*`, `smelt-dialect`, `smelt-core`, `smelt-types`). Report file paths and brief descriptions.
- **codebase-analyzer**: Read the located files and trace data/control flow. Document function signatures, key types, and how components interact. Use `file:line` references.
- **pattern-finder**: Find related patterns, tests, and examples. Check `examples/`, `tests/`, and existing `docs/plans/` for prior work on this topic.

Be SPECIFIC in agent prompts. Tell each agent exactly which directories to search and what to look for.

### Step 4: Synthesize Findings

Wait for ALL subagents to complete. Then read any additional files they identified as critical. Synthesize into a structured research document.

### Step 5: Write Research Document

Create the document at `docs/research/YYYY-MM-DD-{topic-slug}.md` using today's date.

Use this structure:

```markdown
# Research: {Topic Title}

**Date**: {YYYY-MM-DD}
**Topic**: {Description of what was researched}
**Branch**: {current git branch}
**Commit**: {current HEAD short hash}

## Summary

{2-3 sentence overview of findings}

## Key Files

| File | Purpose | Key Lines |
|------|---------|-----------|
| `crates/smelt-parser/src/...` | ... | L42-67 |
| ... | ... | ... |

## Architecture & Data Flow

{How the relevant components connect. Trace the flow from entry point to output.}

## Current Behavior

{What the code does today, with specific references.}

## Related Patterns

{Similar patterns elsewhere in the codebase that are relevant.}

## Test Coverage

{What tests exist for this area, what they cover.}

## Open Questions

{Things that remain unclear or need human input to resolve.}
```

### Step 6: Present Summary

After writing the document, present a brief summary to the user:
- What you found (3-5 bullet points)
- The document path
- Any open questions that need human input

## Important Rules

1. **Describe, don't prescribe.** Document what EXISTS. No suggestions, no critiques.
2. **Use file:line references.** Every claim about the code should be traceable.
3. **Read before spawning.** Always read explicitly mentioned files in YOUR context first.
4. **Wait for all agents.** Don't synthesize until all parallel research completes.
5. **Be honest about gaps.** If something is unclear, say so in Open Questions.
6. **Keep it concise.** Target ~150-200 lines. The document should be scannable.
