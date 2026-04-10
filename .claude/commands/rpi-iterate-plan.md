---
description: Only invoke when /rpi-iterate-plan is explicitly requested by the user. Iterate on an existing implementation plan based on feedback
model: opus
---

# Iterate Implementation Plan

You are tasked with updating an existing implementation plan based on user feedback. Be skeptical, thorough, and ensure changes are grounded in actual codebase reality.

## Input

$ARGUMENTS

This should include:
- A plan file path (e.g., `docs/plans/20260331-feature.md`)
- Requested changes or feedback

## Process

### Step 1: Parse Input

**If NO plan file provided**: Ask for it. Suggest `ls -lt docs/plans/ | head` to find recent plans.

**If plan file provided but NO feedback**: Read the plan and ask what changes are needed.

**If BOTH provided**: Proceed immediately.

### Step 2: Read and Understand

1. Read the existing plan file COMPLETELY.
2. Understand the current structure, phases, and scope.
3. Identify if the requested changes require new codebase research.

### Step 3: Research If Needed

Only spawn research subagents if the changes require understanding new code. Don't research for simple structural changes like "split Phase 2 into two phases" or "update success criteria."

### Step 4: Present Understanding

Before making changes, confirm:

```
Based on your feedback, I understand you want to:
- {Change 1}
- {Change 2}

I plan to update the plan by:
1. {Specific modification}
2. {Another modification}

Does this align with your intent?
```

Get confirmation before editing.

### Step 5: Make Surgical Edits

- Use the Edit tool for precise changes.
- Maintain existing structure unless explicitly changing it.
- Keep all `file:line` references accurate.
- Update success criteria if scope changed.
- Preserve good content that doesn't need changing.

### Step 6: Present Changes

```
I've updated the plan at `{path}`.

Changes made:
- {Change 1}
- {Change 2}

Would you like any further adjustments?
```

## Important Rules

1. **Be skeptical.** Question vague feedback. Ask for clarification.
2. **Be surgical.** Precise edits, not wholesale rewrites.
3. **No open questions in the plan.** If a change raises questions, ask NOW.
4. **Verify references.** If adding new file paths, confirm they exist.
