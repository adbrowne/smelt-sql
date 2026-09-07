# Phase 8 — settle the DuckDB half of criterion 7

## Objective

Turn phase 6's blocked centrepiece into a green, per-window equivalence check. Measure the
real shape of the `gold_events_enriched` enrichment staleness (does it recur? does it always
converge?), root-cause the convergence mechanism far enough to state an honest bound, register
that bound in the same `DIVERGENCE_REGISTRY` shape the succession entries use, and un-`#[ignore]`
`every_window_matches_the_full_refresh_oracle`. This closes criterion 7's DuckDB half — the
"trusted offline before any cloud spend" precondition every live phase rests on. It does **not**
fix smelt's derivation gap: `## Out of scope` assigns fixes to `20260906-bigquery-correctness`,
so the outcome of the root-cause is a handoff finding plus a measured bound.

## Spec delta

None. No user-visible feature behaviour changes — this phase measures and characterises existing
behaviour and adds test coverage. If the root-cause work shows a spec sentence is *wrong* about
maintenance of a `grain: partition` model over a clockless keyed dimension, record it as a
finding for the criterion-8 handoff rather than editing the spec here.

## Tests

All in `crates/smelt-cli/tests/github_activity_oracle.rs` unless noted.

1. `every_window_deep_sweep` (new, `#[ignore]`d by design, run once during this phase and its
   result recorded in the summary) — the exhaustive variant: build a growing full-refresh
   oracle after **every** one of the 30 windows, and record for each day which relations
   diverge and on which columns. Its purpose is measurement, not per-PR gating; it exists so
   the claim "the divergence is confined to days X–Y and self-heals within N windows" is
   backed by a run rather than by the 10-day sample phase 6 had.
2. `enrichment_staleness_is_confined_to_the_enriched_column` — on a diverging window, the
   `gold_events_enriched` difference is only in `current_repo_name` (and only for repos that
   renamed): every other column, and the row *key set* (`id`), is identical between the
   incremental leg and the oracle. Distinguishes "stale value" from "wrong/missing rows".
3. `enrichment_staleness_converges_within_the_declared_bound` — the divergence present at a
   window is absent after at most N further windows, N being the number the deep sweep measured
   (not a guess). Red first with a deliberately-too-small N.
4. `every_window_matches_the_full_refresh_oracle` — the phase 6 centrepiece, `#[ignore]`
   removed, now passing because the new registry entry covers the enrichment divergence and its
   `check_bound` is the convergence predicate above.
5. `registry_entries_are_all_live` (existing) — must still pass with the third entry: an entry
   whose divergence no longer occurs is a failure, so a future fix in `bigquery-correctness`
   loudly retires it rather than leaving dead prose.

## Tasks

1. Write and run `every_window_deep_sweep`; capture per-day, per-relation, per-column divergence
   output into the phase summary (and, condensed, into `examples/github_activity/README.md`).
2. From that output, answer three questions in the summary: does the divergence recur after
   day 10; is it ever unbounded within the fixture; is any relation other than
   `gold_events_enriched` involved.
3. Root-cause the convergence: for one diverging repo, diff the maintenance plan issued on the
   diverging run against the one issued on the run that heals it (`smelt explain
   gold.events_enriched --json`, plus the run report per window). Name the mechanism —
   redelivery re-touching the row by `merge_key: [id]`, a full-scan-licensed recompute, or
   something else — and say so plainly in the summary; "unknown" is an acceptable answer only
   if the phase records what was tried and why it did not settle it.
4. Generalise `DivergenceEntry`/`check_bound` so a bound can be a convergence predicate rather
   than only a `(key, clock)` fold — keep the two succession entries expressed exactly as they
   are today (their bound must not weaken).
5. Add the `gold_events_enriched` entry with the measured N and a `reason` that names the
   derivation gap and its owning outcome.
6. Remove the `#[ignore]` and the stale doc comment on `every_window_matches_the_full_refresh_oracle`.
7. Re-retire `github_activity_replay.rs::full_refresh_matches_incremental_replay`'s hardcoded
   row-count assertions, now genuinely superseded (phase 6 restored them only while the
   centrepiece was blocked).
8. Append the root-cause finding to the criterion-8 material for
   `docs/outcomes/20260906-bigquery-correctness` — one entry naming the model, the statement,
   and the measured convergence behaviour.
9. Runtime budget: the per-PR centrepiece stays at phase 6's sampling (first 10 windows + the
   final) unless the every-day version fits in **5 minutes**; if it does, promote it. Record the
   measured wall time either way. Do not silently drop windows to fit.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test github_activity_oracle` — 5+ tests green, only the deep sweep ignored
- `cargo test -p smelt-cli --test github_activity_replay`
- `cargo test -p smelt-cli --test example_diagnostics`
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`test(github_activity): bound the enrichment staleness and un-ignore the per-window oracle`
