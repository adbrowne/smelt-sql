# Phase 4 summary — validate all four anchor specs and fix drift

## Reports written

- `docs/validations/2026-09-04-definition_deltas-closure.md`
- `docs/validations/2026-09-04-incremental_models-closure.md`
- `docs/validations/2026-09-04-incremental_shapes-closure.md` (supersedes the earlier scoped
  `docs/validations/2026-09-04-incremental_shapes.md`, left untouched)
- `docs/validations/2026-09-04-model_properties-closure.md`

Automated-check leg (`bash .claude/scripts/verify-phase.sh`) run once, before dispatch, all four
gates green: `PASS cargo fmt --check`, `PASS cargo clippy (zero warnings, both feature sets)`,
`PASS cargo test (workspace)`, `PASS example_diagnostics`. All four reports cite this single run.

## Per-spec drift counts and dispositions

| Spec | Drift found | Disposition |
|---|---|---|
| `definition_deltas` | 0 | — |
| `incremental_models` | 0 | — |
| `incremental_shapes` | 1 (References → Code cited a nonexistent `windowing.rs` path for `PartitionAxis`/`resolve_scan_window`) | fixed this phase |
| `model_properties` | 1 (Surface + §Semantics "Event-time monotonicity trace" omitted the shipped `Offset::Integer` variant; `last_reviewed` stale as a result) | fixed this phase |

**No new phase row added, no `## Blocked` entry added.** Both findings were doc/wording drift
(spec text lagging already-shipped, already-tested code) under the phase-4 standing rule, so both
were fixed inline rather than deferred:

- `incremental_shapes.md` §References: corrected `PartitionAxis` → `analysis/partition_axis.rs`,
  `resolve_scan_window` → `analysis/source_bounds.rs`, `PartitionPoint` → kept at
  `smelt-runtime/src/windowing.rs` (already correct there).
- `model_properties.md`: added `Offset::Integer` to the Surface table row and the §Semantics
  "Event-time monotonicity trace" paragraph, with the `incremental_models.md` cross-reference the
  code's own doc comments already pointed to; bumped `last_reviewed` 2026-08-16 → 2026-09-04.

One item was initially mis-dispositioned by the validating sub-agent as behaviour drift needing a
phase row (the `model_properties` `Offset::Integer` gap) and was recategorized during this phase:
the code already implements and tests the variant, so the spec was simply out of date — squarely
"doc/wording drift" per the standing rule, not a functional gap needing new work. Fixed inline
instead of deferred.

Every `❌` line in a report closes with a disposition marker (`— fixed this phase`) or the
`✅`/`⚠️` items cite a `baseline-inventory.md` ID (`— flagged-open: <ID>`) where relevant, per
`check-validations.sh`.

## Timeless-oracle sweep

`rg -n "Phase [A-Z0-9]+" docs/specs/{definition_deltas,incremental_models,incremental_shapes,model_properties}.md`
— zero matches across all four specs.

## Gates

- `bash .claude/scripts/verify-phase.sh` — all green (run once at task 2; only docs changed after,
  so not re-run — no Rust source touched this phase, confirmed via `git diff --stat`).
- `bash docs/outcomes/20260815-incremental-spec-closure-confirm/check-validations.sh` — red before
  (4 missing reports), green after.
- `bash docs/outcomes/20260815-incremental-spec-closure-confirm/check-inventory.sh` — green
  (80 bullets, unchanged).
- `bash docs/outcomes/20260815-incremental-spec-closure-confirm/check-classification.sh` — green
  (80/80 valid, repo-verified dispositions, unchanged).

## For phase 5 (closure report)

Cite these four report paths as the criterion-4 evidence. No new phase row or `## Blocked` entry
exists from this phase — criterion 4 is fully satisfied with zero unresolved drift beyond the
already-flagged-open baseline bullets (criteria 1-3's territory, already closed in phases 1-3).
