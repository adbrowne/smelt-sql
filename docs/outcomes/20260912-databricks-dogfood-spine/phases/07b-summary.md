# Phase 7b summary — the sidecar-less key-addressed cell downgrades at plan derivation

## Shipped

- `docs/specs/state.md` §"The degradation contract" step 2: the required structure is now a
  function of the **cell**, not the technique alone (`FingerprintSidecar` for a
  `PerGroupRecompute` cell whose `key_scope.discovery` is `UpstreamKeyed` or
  `DownstreamGrainOverUpstream`), and the recompute-family fallback now depends on the key
  scope's own discovery route.
- `required_state_structure` (`crates/smelt-logical/src/maintenance/availability/
  state_structure.rs`) takes `&PlanCell` instead of `Technique`, exhaustive over both
  `Technique` and, for the `PerGroupRecompute` arm, `KeyDiscovery`.
- `recompute_equivalent` (`crates/smelt-logical/src/maintenance/availability/mod.rs`): a cell
  already at `PerGroupRecompute` downgrades to `DeleteInsert` (never a no-op back to itself),
  and a `key_scope`-carrying cell now downgrades per its discovery route — `UpstreamKeyed`/
  `DownstreamGrainOverUpstream` to `PerGroupRecompute` (the key-addressed driver dispatches
  these), `EnrichmentKeyed` to `DeleteInsert` (the driver never dispatches this variant, so
  `PerGroupRecompute` is not a realisable fallback for it).
- `maintenance_driver/key_addressed/mod.rs`'s `!supports_fingerprint_sidecar` bail is now
  documented as a defensive guard for inconsistent inputs, not the primary route.
- 6 new/updated tests in `smelt-logical`'s `maintenance_availability` suite (29 total, was 28)
  and 1 new test in `smelt-runtime`'s `key_addressed_model_edge_lowering` (14 total, was 13),
  per the plan's test list.
- Live: three consecutive Databricks windows (W4 `2026-08-10`, W5 `2026-08-11`, W6 `2026-08-12`),
  each **16 success / 0 failed / 0 skipped** — `gold.events_enriched` and `marts.star_growth`
  both complete for the first time in this outcome.

## Decisions

- **Downgrade at plan derivation, not sidecar realisation on Delta** — per the outcome's own
  decision log (2026-09-12, phase 7b plan), confirmed correct by the live re-run.
- **The real live bug was one level deeper than the plan's root cause.** The plan assumed
  `gold.events_enriched`'s failing cell was ideally-derived as `PerGroupRecompute` directly.
  In fact its only key-addressed candidate is the `gold.repo_dim` edge's `ColumnScopedMerge`
  cell (`KeyDiscovery::EnrichmentKeyed`, a value-enrichment join) — ideal derivation never
  gives this a `PerGroupRecompute` technique. The actual failure path: Spark has no
  `MergeLedger`, so availability resolution downgrades this `ColumnScopedMerge` cell via
  `recompute_equivalent`'s **generic** `key_scope.is_some() → PerGroupRecompute` rule — which,
  before this phase, did not distinguish discovery routes and handed the key-addressed driver a
  `PerGroupRecompute` cell it can never execute for `EnrichmentKeyed` (per that variant's own
  doc comment). Fixed by making `recompute_equivalent` route on `KeyDiscovery` explicitly:
  `EnrichmentKeyed` cells fall straight to `DeleteInsert`. This fix subsumes the plan's
  originally-scoped one (both are the same `recompute_equivalent`/`required_state_structure`
  pair) and required no additional file changes beyond what the plan's tasks already touched.
- Verified this deeper bug is **pre-existing, not introduced by this phase**: the downgrade
  rule `ColumnScopedMerge` (missing `MergeLedger`) → `PerGroupRecompute` existed unchanged
  before this phase; only the *result* of feeding that `PerGroupRecompute` cell to the
  key-addressed driver was new. Consistent with W1–W3 (row 7) recording this as the *only*
  incremental-path gap, recurring identically every window.

## For the next planner

- **`gold.events_enriched`'s coverage has a gap**: `[2026-08-05, 2026-08-07)` then
  `[2026-08-10, 2026-08-13)` — `2026-08-07` through `2026-08-10` was never computed for this one
  model (every window in that range failed before this phase landed). Every other model is
  contiguous `[2026-08-05, 2026-08-13)`. This does not block criterion 6 (16/16 completes
  going forward) but **will** matter for row 8 (dual-target parity) and row 9 (oracle
  equivalence) if either assumes contiguous coverage — a backfill run
  (`--event-time-start 2026-08-07 --event-time-end 2026-08-10`) may be needed before trusting
  either check over the full history. Left unresolved here since it is outside 7b's own scope
  (one cell, one model, three *new* windows).
- `completed_at`/`duration_ms` are now populated in the three new run reports, unlike the gap
  row 7's summary recorded. Not investigated or fixed here — likely coincidental (e.g. a prior
  run's in-flight failure leaving the reporter's finalize step unreached vs. these three
  runs completing cleanly) rather than a real fix; row 10's punch-list should note the original
  gap is not reproducible on a clean run and may not need a fix at all.
- `free-edition-facts.md`'s quota table gained a second wall-time row for the 16/16 windows
  (79.9s / 64.5s / 62.4s for W4/W5/W6) — W4 is not dramatically slower than W5/W6 despite
  recomputing `gold.events_enriched`'s whole 3-day-pinned region, likely because the region is
  small relative to per-model/session overhead on this fixture's scale.
- No definition-delta / migration-approval prompt fired during any of the three windows.

## Gates

- `cargo test -p smelt-logical --test maintenance_availability` — 29/29 pass.
- `cargo test -p smelt-runtime --test key_addressed_model_edge_lowering` — 14/14 pass.
- `cargo test -p smelt-cli --test maintenance_conformance` — 104/104 pass.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full workspace `cargo test`, `example_diagnostics`). One large-file baseline update was needed
  and applied (`crates/smelt-runtime/tests/key_addressed_model_edge_lowering.rs` 1147→1200 lines,
  the new test 6) — the growth is the plan's own new test, not incidental.
- Live: W4/W5/W6 run reports under `examples/github_activity/.smelt/targets/databricks/reports/`
  (`20260912-141835-94ba08.json`, `20260912-142038-4fb3f0.json`, `20260912-142216-fec49c.json`),
  each 16/16; `intervals.json` confirms every model ends at `2026-08-13`.
