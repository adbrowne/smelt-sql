---
description: Scaffold a new outcome for the outcome loop (docs/outcome_loop.md)
model: opus
---

# Scaffold an Outcome

Create a new outcome directory under `docs/outcomes/` for the outcome loop.
Read `docs/outcome_loop.md` first — it defines the process and formats.

## Input

$ARGUMENTS

A short name and/or a description of the goal (may reference a research doc
or spec section).

## Process

1. Clarify the goal with the user if it is ambiguous — an outcome with fuzzy
   success criteria produces drifting phases.
2. Create `docs/outcomes/<YYYYMMDD-short-name>/outcome.md` following the
   structure of existing outcomes (e.g. `docs/outcomes/20260809-rung2-state-shapes/outcome.md`):
   - **The outcome** — 3-6 sentences: the end state, not the work.
   - **Success criteria** — numbered, individually checkable, including the
     standing gates staying green.
   - **Out of scope** — explicit exclusions (this is what licenses the
     planner to defer things; anything not listed here that serves the
     criteria must be scheduled, never dropped).
   - **Phases** — a table of ONE-LINE intents with Status `pending`. Aim for
     4-10 rows. No per-phase detail — the plan step writes that just-in-time.
     If the work changes user-visible feature behaviour, phase 1 is the spec
     delta (spec-first rule).
   - Empty **Decision log** and **Blocked** sections.
3. Ask the user whether to activate it now. If yes, write the directory path
   as the single line of `.claude/active-outcome`.
4. Commit the new files (`outcome(<name>): scaffold`). Remind the user how to
   launch: detached tmux running `.claude/scripts/outcome-loop-forever.sh`
   from the target checkout — never a backgrounded Bash call from a session.
