# Phase 02 summary — stale citation sweep

## Shipped

Per-site resolution table:

| # | Site | Anchor before | Resolution |
|---|------|---------------|------------|
| 1 | `materialized_view.md:20` | `incremental_models.md` §"The composition contract" | → §"Shape profiles" |
| 2 | `run_state.md:139` | `incremental_models.md` §"Failure mode" | → `incremental_shapes.md` §"First-run and backfill" |
| 3 | `timeseries.md:119` | `incremental_models.md` §"Granularity values" | dropped (circular; the enum is owned by `timeseries.md` itself) |
| 4 | `models.md:252`, `metadata.rs:1232`, `rules/incremental.rs:457` | `incremental_models.md` §"Non-determinism and the payload rule" | → `incremental_shapes.md` §"Safety checks (per-cell admission for recompute-a-region)" (the section that actually states the `plausible`-column taint rule; the plan's candidate, §"Batch safety classification", turned out to be about bound-map classes, not determinism — verified by reading both before choosing) |
| 5 | `gate.rs:5189`, `:5400` | `incremental_models.md` §"Per-slice equivalence" | → `incremental_shapes.md` §"Key temporal locality (the time-partitioned output)" |
| 6 | `propagate.rs:411` | `incremental_models.md` §"Row movement" | → `incremental_shapes.md` §"Key temporal locality", "Row movement" (form copied from `locality.rs:1138`); the co-cited `§"What the composed shape uniquely enables"` on the same line is **not** on this phase's site list and stays dangling — see Follow-up below |
| 7 | `propagation.rs:1723` | `incremental_models.md` §"The clamp both directions" | → §"Windowed maintenance and the horizon" |
| 8 | `refresh_axis.rs:451` | `incremental_models.md` §"The declared shape axis" | → §"The declared shape" |

`materialized_view.md:20`'s `(×2)` in the TODO turned out to be one real citation plus one
plain-prose mention of "the composition contract" at line 123 (not a `§"…"` anchor) — the TODO's
count was off by one, not a second site.

`docs/TODO.md`'s "Stale citations flagged by the sweep" bullet is deleted.

## Decisions

- Verified every candidate target against the citing sentence's actual claim before editing,
  not just against heading existence — site 4's plan-suggested target didn't carry the claim;
  the real target does.
- Left the co-cited `§"What the composed shape uniquely enables"` (site 6's line, and
  `models.md:134`, and `propagate.rs:343`) untouched: not named in this phase's site list, and a
  real gap (the heading doesn't exist in either spec) rather than a simple re-point — needs its
  own investigation into which section, if any, should carry it.

## For the next planner

- New `docs/TODO.md` bullet added: "Dangling 'What the composed shape uniquely enables'
  citations" — three sites (`models.md:134`, `propagate.rs:343`, `propagate.rs:412`) cite a
  heading absent from both `incremental_models.md` and `incremental_shapes.md`. Not part of this
  outcome's success criteria; flagged for a future sweep.
- Residual sweep noise outside this phase's scope (pre-existing, not touched): three
  `docs/research/20260816-bigquery-backend.md` citations in `smelt-backend-bigquery/src/sql.rs`
  (research docs aren't specs, so the sweep script's spec-only check misfires on them — not an
  actual defect); one `docs/specs/CLAUDE.md` citation in `smelt-backend-duckdb/CLAUDE.md` (a
  process file, not a spec — same false-positive class).
- Phase 3 (absent-state sentences) and phase 5 (validate) are unaffected by this phase's choices.

## Gates

- `bash .claude/scripts/verify-phase.sh` → `VERIFY: ALL GREEN`
- Citation sweep (ad hoc `rg`/shell, not a cargo test) over all eight sites and the two touched
  specs: clean (see table above; full workspace sweep found only pre-existing non-spec noise).
- `rg -n 'Stale citations flagged by the sweep' docs/TODO.md` → exit 1 (removed).
- `rg -n 'Phase [A-Z0-9]' docs/specs/{materialized_view,run_state,timeseries,models}.md` → only
  hit is `models.md`'s own Timeless-oracle-rule banner quoting the pattern as an example; not a
  violation.
