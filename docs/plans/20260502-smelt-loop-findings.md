# smelt-loop findings — 2026-05-02 (small tier, local mode)

Accumulated TOOL_BUG and DOCS_GAP findings from the `/smelt-loop --tier small --mode local --iterations 5` run on 2026-05-02. Skill changes were applied directly during the loop and are not listed here.

This is a **review-only plan** — no code or doc changes have been made. Use this as the single document to triage which findings warrant follow-up specs/plans.

Spec: n/a (cross-cutting CLI / docs / type-inference findings; some entries may motivate updates to per-feature specs).

## How to read this

Each entry has:
- **Source**: which iteration's `retro.md` / `review_notes.md` it came from
- **Reproduction**: shell commands to reproduce, where applicable
- **Proposed direction**: not a fix, just the shape the fix should take. Decision deferred to the user.

## TOOL_BUG candidates

### TB-1: `smelt build --verbose` produces no extra output
**Source**: iteration 1.

`smelt build --help` advertises `--verbose` as "Show compiled SQL for each model", but on a clean rebuild the output is identical to the non-verbose run.

```
$ rm -f orders-pipeline.duckdb
$ smelt build --verbose
smelt: loaded 2 seed(s) (15 rows) in 0.03s
smelt: built 3 model(s) in 0.03s
```

**Proposed direction**: either wire the verbose path to actually emit compiled SQL per model, or correct the `--help` text. Likely a single-crate change in `smelt-cli`. Worth checking which code path the standard build runs through and whether `--verbose` is wired only for a separate runner mode.

### TB-2: Inferred type for a passthrough DATE column is TEXT
**Source**: iteration 1.

A CSV column that DuckDB types as `DATE` after `smelt seed` is reported as `TEXT` by `smelt table <staging_model>` and lands as `VARCHAR` in the materialized table when the staging model does a passthrough `SELECT col`. This is a silent narrowing.

```
$ rm -f orders-pipeline.duckdb
$ smelt seed
$ duckdb orders-pipeline.duckdb -c 'DESCRIBE raw_orders'        # order_date -> DATE
$ # stg_orders.sql:  SELECT o.order_date AS order_date FROM smelt.models.raw_orders o
$ smelt build
$ smelt table stg_orders                                         # order_date -> TEXT  ← divergence
$ duckdb orders-pipeline.duckdb -c 'DESCRIBE stg_orders'         # order_date -> VARCHAR
```

**Proposed direction**: reconcile smelt's seed-type inference with DuckDB's CSV-reader output, or surface the divergence as a warning. Worth testing whether DECIMAL/DOUBLE columns hit the same issue. Likely lives in seed schema extraction inside `smelt-db` and/or the type-inference pass over passthrough projections.

### TB-3: No project-wide "compile but don't execute" flag
**Source**: iteration 1. (Borderline — feature request more than bug.)

`smelt build --dry-run` is rejected. `smelt build --show-plan` exists but requires a positional model file, so there's no way to compile-and-validate the whole project graph at once without executing it.

**Proposed direction**: decide whether `--show-plan` should accept "no model file means whole project" or whether a separate `--dry-run` flag is warranted. Document the chosen surface in `reference/cli`.

### TB-4: `smelt --version` is not a recognised flag
**Source**: iteration 3.

Most CLIs surface `--version` as a top-level flag for free; smelt rejects it via clap as unrecognised. Build agent had to "discover this experimentally."

```
$ smelt --version
error: unexpected argument '--version' found
```

**Proposed direction**: add a top-level `--version` flag (or surface the canonical version subcommand in `smelt --help`). Trivial clap-level change.

## DOCS_GAP candidates

### DG-1: Seed physical type vs. smelt inferred type
**Source**: iteration 1.
**Page**: `reference/cli` (and arguably `guide/seeds`).

Add a note that `smelt table <model>` is the source of truth for downstream type-checking and the materialized column types — these can disagree with `DESCRIBE raw_<seed>`. Until TB-2 is resolved, document the rule (e.g., "all CSV columns infer as TEXT regardless of DuckDB's CSV reader output") so users aren't surprised. Once TB-2 ships, replace this note with a pointer to the new behaviour.

### DG-2: `smelt build` flag truth-table
**Source**: iteration 1.
**Page**: `reference/cli`, with a one-liner in `getting-started/quickstart`.

Add a flags cheatsheet for `smelt build`:
- `--dry-run` is **not** a flag.
- `--show-plan` requires a positional model file.
- `--verbose` only emits extra output when models actually run (revisit once TB-1 is fixed).
- `--select` must be repeated, not space-separated.

### DG-3: `--verbose` actual behaviour
**Source**: iteration 1.
**Page**: `reference/cli`.

Add a sentence describing exactly what `--verbose` prints, and under what conditions ("no extra output when all models are up-to-date"). Replace once TB-1 ships.

### DG-4: Quickstart omits `model_paths` / `seed_paths` keys
**Source**: iteration 2.
**Page**: `getting-started/quickstart`.

The quickstart's `smelt.yml` example relies on the default `model_paths` / `seed_paths` and never names the keys. A beginner hitting a `smelt.yml` parse error has no fast pointer to "valid top-level keys are X". Add a short callout near the example listing the supported keys (`name`, `version`, `model_paths`, `seed_paths`, `targets`) and noting the defaults; cross-link to `reference/smelt-yml`.

### DG-5: No discoverable doc on seed CSV column-type inference
**Source**: iteration 2 (related to TB-2 / DG-1 from iteration 1).
**Page**: `guide/seeds`.

The skill covers when to `CAST` because of seed-type quirks, but `guide/seeds` doesn't explain how smelt infers seed CSV column types or when its inference disagrees with DuckDB's stored type. Add a "Column type inference" section: how inference works, the divergence cases (TB-2), and the recommendation to `CAST` explicitly in staging when the spec dictates a target type. Mention `smelt table <model>` as the inspection tool. Likely overlaps with DG-1 — consolidate into a single section once TB-2 is decided.

### DG-6: `reference/cli` should enumerate every flag
**Source**: iteration 3.
**Page**: `reference/cli`.

Add a full flag listing for `smelt`, `smelt build`, `smelt table`, `smelt docs`. Call out the `--select` repeated-flag requirement (positional values fail), the absence of `--dry-run`, the per-model `--show-plan` (TB-3), and the lack of a top-level `--version` (TB-4). Subsumes DG-2 — merge them when writing the page.

### DG-7: `guide/seeds` filename → ref mapping
**Source**: iteration 3.
**Page**: `guide/seeds`.

Add an explicit "`seeds/raw_orders.csv` is referenced as `smelt.models.raw_orders`" example near the top, with a one-liner that seed name = filename minus `.csv` and that seeds are first-class ref targets (no separate namespace). Pairs with DG-5.

### DG-8: Aggregate type-inference subsection in `reference/language`
**Source**: iteration 3.
**Page**: `reference/language` (or wherever type inference is documented).

Add a "type inference for aggregates" subsection covering `COUNT(*)` → `BIGINT`, `SUM(DOUBLE)` → `DOUBLE`, `SUM(INTEGER)` widening behaviour, and when `COALESCE(agg, literal)` produces a non-nullable column. The skill currently carries this as a workflow gotcha; the canonical reference should own it so the skill can shrink to a pointer once the docs page exists.

### DG-9: Document the default materialization
**Source**: iteration 4.
**Page**: `reference/smelt-yml` and/or `guide/materializations`.

If `materialization:` is omitted from a model's frontmatter, the model is built as a `table`. Currently neither `reference/smelt-yml` nor `guide/materializations` states this explicitly, and the build agent in iter 4 added `materialization: table` to every model "to be safe." Add a one-line statement and cross-link `reference/smelt-yml` → `guide/materializations`.

## Iterations completed

| # | tier | passed/total | retro signal | skill diff |
|---|------|--------------|--------------|------------|
| 1 | small | 10/10 | yes (3 TB, 3 DG, 4 SG) | applied (167 lines) |
| 2 | small | 10/10 | weak (0 TB, 2 DG, 0 SG) | none |
| 3 | small | 10/10 | weak (1 TB, 3 DG, 0 SG) | none |
| 4 | small | 10/10 | weak (0 TB, 1 DG new, 2 SG) | applied |
| 5 | small | 10/10 | none (0 TB, 0 DG new, 0 SG actionable) | none |

## Loop convergence

Five iterations on the small fixture. Iterations 4 and 5 effectively converged — clean 10/10 builds with marginal retro signal that resolved to placement nits or duplicates. Future loop runs should switch to a larger / different-shape fixture (medium tier) to surface new failure modes; the small fixture has been exhausted.

## Triage shortlist

Highest leverage findings, in rough order:

1. **TB-2** (passthrough DATE → TEXT type narrowing) — surfaces in the very first iteration on the very first staging model. High user-pain, possibly easy bug. Pair with **DG-1 / DG-5** (consolidate into a single seed-type-inference doc section).
2. **TB-1** (`--verbose` produces no output) — `smelt build --help` over-promises. Either wire it up or correct the help text.
3. **TB-4** (`--version` flag missing) — trivial clap fix, high discoverability win.
4. **DG-2 / DG-6** (CLI flag truth-table) — folds in the surface-area mismatches behind TB-1/3/4 once those are decided. Single docs PR.
5. **TB-3** (no project-wide compile-only flag) — feature-shaped; decide if `--show-plan` should accept "no positional means whole project" or add a `--dry-run`.
6. **DG-9** (default materialization is `table`) — one-line addition to `reference/smelt-yml`. Easy win.
7. Lower priority / lower frequency: **DG-7** (seed filename → ref mapping), **DG-8** (aggregate type-inference subsection).
