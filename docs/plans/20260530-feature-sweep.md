# Plan: Feature Sweep — Bug-Finding & Auto-Improving Loop

**Date**: 2026-05-30
**Meta-plan**: `~/.claude/plans/we-now-have-a-idempotent-torvalds.md` (across-session source of truth: per-phase routine, ledger format, sentinel contract, pause conditions)
**Tracking branch**: `worktree-test_features`
**Docs**: code+docs (fixes update `docs/specs/` only via human review; this loop never edits a spec autonomously)

## Execution prompt (for a fresh session / autonomy iteration)

Read the meta-plan and this file. Run the next `pending` phase in the Progress-tracking table below using the meta-plan's **per-phase routine** (pre-flight → `/smelt:validate` drift report → adversarial fixtures → existing suite → log findings → fix clear code bugs red-green / log judgment calls as `needs-review` → verify green → update this table → commit + push). Append every finding to the ledger `docs/bug-hunt/2026-05-30-findings.md`. Emit exactly one sentinel: `<<PHASE_COMPLETE>>`, `<<ALL_DONE>>`, or `<<PAUSE_FOR_HUMAN>>`.

## Context

A lot of functionality has landed across 26 specs in `docs/specs/`. This plan drives a systematic sweep that exercises each feature individually (Waves A–C, hotspot-first) and in combination (Wave D), records every bug with its root cause in a single ledger, fixes clear code bugs in-loop, and batches judgment calls (spec conflicts, invariant-touching fixes) as `needs-review` for one human pass at the end. See the meta-plan for the full rationale.

## Scope

### In scope
- One probe phase per spec (Waves A–C) and per adjacent-feature seam (Wave D).
- Three probe methods per phase: `/smelt:validate` drift report, new adversarial fixtures, existing suite + property tests.
- Red-green fixes for clear code bugs; `needs-review` logging for judgment calls.

### Explicitly deferred
- `/smelt-loop` end-to-end build tiers.
- Backend coverage beyond DuckDB.
- Resolving `needs-review` entries (done in the post-sweep human review).

## Progress tracking

| Phase | Feature / seam | Status | Findings | Commit | Date |
|-------|----------------|--------|----------|--------|------|
| S0 | Setup: artifacts + loop wiring | pending | | | |
| A1 | architecture | pending | | | |
| A2 | incremental_models | pending | | | |
| A3 | cli | pending | | | |
| A4 | functions | pending | | | |
| A5 | meta_language | pending | | | |
| B1 | expansion | pending | | | |
| B2 | function_schema_inference | pending | | | |
| B3 | cumulative_aggregate | pending | | | |
| B4 | gradual_typing | pending | | | |
| B5 | meta_config_loading | pending | | | |
| B6 | planner_integration | pending | | | |
| C1 | types | pending | | | |
| C2 | scoping | pending | | | |
| C3 | models | pending | | | |
| C4 | timeseries | pending | | | |
| C5 | seeds | pending | | | |
| C6 | sources | pending | | | |
| C7 | datagen | pending | | | |
| C8 | schema_evolution | pending | | | |
| C9 | python_models | pending | | | |
| C10 | testing | pending | | | |
| C11 | model_selection | pending | | | |
| C12 | lsp | pending | | | |
| C13 | data_catalog | pending | | | |
| C14 | smelt_yml | pending | | | |
| D1 | functions × incremental × timeseries | pending | | | |
| D2 | functions × schema_inference × types | pending | | | |
| D3 | meta_language × functions × config_loading | pending | | | |
| D4 | incremental × cumulative_aggregate × timeseries | pending | | | |
| D5 | seeds × sources × types | pending | | | |
| D6 | model_selection × generators × cli | pending | | | |
| D7 | project isolation × lsp | pending | | | |
| D8 | run-pipeline parity (cli ↔ ui) | pending | | | |

**Status values**: `pending` → `done`. A phase is `done` even if it logged `needs-review` findings (those are deliberately deferred, not blocking). Record the count of findings logged in that phase in the Findings column (e.g. `2 fixed, 1 needs-review`).

## Verification

See the meta-plan's Verification section. In short: ledger fully triaged (no `open`), each `fixed` bug has a red-green regression test, full suite + `example_diagnostics` + `example_workspaces` + `smelt-runtime` green, every table row `done`.
