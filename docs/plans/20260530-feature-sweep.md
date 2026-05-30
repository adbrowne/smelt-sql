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
| S0 | Setup: artifacts + loop wiring | done | scaffold | 16e7a49a | 2026-05-30 |
| A1 | architecture | done | 0 fixed, 2 needs-review | (below) | 2026-05-30 |
| A2 | incremental_models | done | 0 fixed, 1 needs-review (mature; +e2e idempotency/equivalence coverage; smelt_shop_min 3 bugs confirmed fixed) | (below) | 2026-05-30 |
| A3 | cli | done | 1 fixed (BUG-005: sub-dir seeds unresolvable in CLI run/explain path — asymmetric discovery; red-green + e2e) | (below) | 2026-05-30 |
| A4 | functions | done | 0 fixed, 1 needs-review (mature; gates+1000-case proptests green) | (below) | 2026-05-30 |
| A5 | meta_language | done | 0 fixed, 1 needs-review (BUG-006: in-model meta (spread/HOF/columns_of/config.var/with_tag) is LSP-clean but unbuildable via CLI run pipeline — BUG-005 class, systemic; 65/65 diag codes present; per_cohort_union builds e2e) | (below) | 2026-05-30 |
| B1 | expansion | done | 0 fixed, 2 needs-review (BUG-007: function-body CTE colliding with a caller CTE silently drops the arg + emits wrong data — mandated codegen-time collision diagnostic absent, **soundness**; BUG-008: stale 3-arg `make_generator_frame` signature in spec). Frame-stack/provenance/`<generator>`-frame surface all covered + green | (below) | 2026-05-30 |
| B2 | function_schema_inference | done | 0 fixed, 1 needs-review (BUG-009: TableExpr `source.*` body called with a `smelt.<path>` arg splices to over-qualified `schema.table.*` → DuckDB syntax error at `smelt run`, while `smelt type` reports the schema clean — Invariant-2 schema/codegen disagreement; schema layer + all gates + 1000-case proptests green) | (below) | 2026-05-30 |
| B3 | cumulative_aggregate | done | 1 fixed (BUG-010: no-window full-refresh path bypassed `classify_cumulative` → forbidden cumulative SQL e.g. `STRING_AGG` silently materialised as a plain table, exit 0; violated Constraint #10. Fixed red-green in both run-pipeline entry points via shared `classify_cumulative_sql`), 1 needs-review (BUG-011: 7 classifier-check diagnostics never surface in the LSP/`smelt-db` layer — run-path strict, LSP permissive; same class as BUG-006). Cross-partition equivalence + classifier unit gates green | (below) | 2026-05-30 |
| B4 | gradual_typing | done | 0 fixed, 2 needs-review (BUG-012: malformed annotation doesn't demote to Tier 1 — `compute_tier` keys on raw `type_ref_text`, not the parse/`InvalidFunctionTypeRef` result; BUG-013: nested `smelt.functions.*` calls inside a function body are emitted verbatim by the run pipeline — printer body-reparse doesn't recognise path calls in the paren-wrapped fragment; function composition unbuildable, BUG-006 class). Type-checking layer (tiers, body checks, frame traces, LSP stability, format contract) sound; gates + 1000-case proptests green | (below) | 2026-05-30 |
| B5 | meta_config_loading | done | 0 fixed, 2 needs-review (BUG-014: per-target overlay is unwired in the production run/generator pipeline — `loader_resolved_value_with_overlay` has zero prod callers, smelt-db has no build-target concept, so `smelt build --target prod` silently uses the base config and never validates the overlay; merge logic exists+unit-tested one hop away. BUG-015: loader content-validation diagnostics don't surface through the CLI run/build pipeline — a schema-violating config silently drops generated models, exit 0; `collect_loader_values` swallows loader diagnostics + run pipeline skips `file_diagnostics`, BUG-006/011 class). Loader layer (parsers, 13 diag codes, schema validation, Salsa invalidation) mature; all gates + loader unit tests green | (below) | 2026-05-30 |
| B6 | planner_integration | done | 0 fixed, 1 needs-review (BUG-016: model frontmatter parser `ModelMetadata`/`deny_unknown_fields` rejects the planner keys `deterministic`/`idempotent`/`append_only`/`backends`/`joins`/`provenance` that architecture.md's Unified-frontmatter rule mandates — a model carrying one silently drops its whole frontmatter, reverting `materialization: table`→`view` with exit 0 and no diagnostic; function files emit spurious `unknown field` warnings. Two divergent parsers exist. Architectural). `--show-plan` surface (determinism, read-only, required-file), the 4 provenance/joins diagnostic codes + their tests, and all planner/example gates green | (below) | 2026-05-30 |
| C1 | types | done | 0 fixed, 1 needs-review (BUG-017: cross-family binary arithmetic not rejected — `42 + '3'` infers `SMALLINT` instead of `Unknown`+`TypeMismatch`, violating types §1/§14; sibling strict surfaces `[1,'hello']`/`UNION` correct, so the gap is the `BinaryExpr` arithmetic path; touches the strict-by-default doctrine, fix may conflict with the DuckDB-coercion proptest oracle). VALUES/alias-arity/empty-values surface + diagnostic codes + timeless-oracle clean; all gates + 1000-case proptests green | (below) | 2026-05-30 |
| C2 | scoping | done | 0 fixed, 2 needs-review (resolution/type layer mature — all 8 scoping codes wired + tested, parameters-first/intersection/ctx-validation/CTE rules sound; gates + 1000-case proptests green. BUG-018: block `PASSING <name> AS (<body>)` fragment args are dropped by the run-pipeline expander — `substitute_params_with_named` only consumes positional+named, so the fragment is emitted as its bare name/default → invalid SQL at `smelt build`; repo's own `rollup_with_passing.sql` demo is broken; BUG-013 class. BUG-019: scoping diagnostics (`CteCycle`/`UnknownContext`/`ParameterShadowsColumn`) fire in `file_diagnostics`/LSP but the CLI run/build/type pipeline never gates on them — a cyclic-CTE function builds into a DuckDB catalog error instead of `CteCycle`; BUG-006 class) | (below) | 2026-05-30 |
| C3 | models | done | 1 fixed (BUG-020: single-model frontmatter `name:` overrode the file stem in both discovery copies — disagreeing with the smelt-db/LSP file-stem path and the spec → LSP↔CLI identity asymmetry; fixed red-green + e2e), 1 needs-review (BUG-021: duplicate model names unrejected — within-file sections silently collapse; cross-file bare-name dupes survive but are disambiguated by canonical-path addressing, so the spec example is stale; within-file fix intersects BUG-006 gating), 1 docs-gap deferred (BUG-022: `test` mode absent from materializations guide). Constraint-violations table + tag-merge + materialization-precedence all enforced; gates + example_workspaces + runtime parity green | (below) | 2026-05-31 |
| C4 | timeseries | done | 0 fixed, 4 needs-review (validate_timeseries layer sound — incremental-required/ephemeral/test/week_start-week rules wired + tested, file_diagnostics mapping + gates green. BUG-023: a serde-level `timeseries:` error (`granularity: fortnight`/missing key) silently drops the *whole* frontmatter → `materialization: table` built as **VIEW**, exit 0, no diag/warning [DuckDB-confirmed]; BUG-016 root mechanism. BUG-024: `MalformedTimeseries`/`TimeseriesRequiredForIncremental` fire in LSP but CLI run/build gates only `UnknownSmeltFn` → malformed-block model builds exit 0 as BASE TABLE, BUG-006 class. BUG-025: `TimeseriesConfig` missing `deny_unknown_fields` → unknown keys silently accepted, entangled with BUG-023. BUG-026: `week_start` accepts all 7 weekdays vs spec monday/sunday, deferred-feature) | (below) | 2026-05-31 |
| C5 | seeds | done | 1 fixed (BUG-027: seed sidecar parse errors — forbidden `name:`, malformed YAML, unknown column type — swallowed by `parse_sidecar(...).ok()` in `read_sidecar_from_path`; seed loaded silently with full inference, exit 0, no diagnostic. Fixed red-green; error now propagates from `discover_seeds`, covering `smelt seed` + `smelt build`), 3 needs-review (BUG-028: `smelt seed --select <source>` silently exits 0 vs spec hard-error "not a seed"; BUG-029: Semantics-5 compile/runtime inference-divergence diagnostic unimplemented + conflicts with the §Surface "v1 sharp edge" note; BUG-030: unknown `materialization:` value silently coerced to `table`), 1 deferred docs-gap (BUG-031: shape-valid-but-calendar-invalid date infers DATE then hard-fails at load). Type inference (11-col DuckDB-verified), CSV strictness, ephemeral VALUES-CTE expansion (e2e), sidecar validation, LSP affordances all sound; all gates + seed suites green | (below) | 2026-05-31 |
| C6 | sources | pending | | | |
| C7 | datagen | done | 0 findings (mature; all config-validation rules enforced) | (drift report) | 2026-05-30 |
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
