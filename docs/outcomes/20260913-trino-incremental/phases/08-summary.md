# Phase 8 summary — the generative gate's Trino arm and the append-only pool live

**Shipped:**
- `ConformanceTarget::Trino { schema }` in `smelt-maintenance-testkit`'s `recipe.rs`, plus
  `trino_conformance_schema`/`trino_env` (BigQuery's per-case-dataset shape, not Spark's shared
  schema), and Trino arms across `render.rs` (`render_smelt_yml_for`, all three staging entry
  points) and `link_c_harness.rs` (run + backend-open matches, `open_trino_conformance_backend`).
- The feature-gate that used to restrict `smelt-maintenance-testkit`'s families to
  `spark`/`bigquery` builds is gone — Trino needs no optional client crate, only a runtime env
  check — so `dag.rs`/`feed.rs`/`schedule_gen.rs`/`gate_composed.rs`/`gate_keyed.rs`/`gate_mixed.rs`
  now compile unconditionally.
- New binary `crates/smelt-cli/tests/maintenance_conformance_trino/` (`main.rs`, `backend.rs`,
  `gate_trino.rs`, `harness_self_check_trino.rs`), modelled on `maintenance_conformance_bigquery/`:
  `TrinoConformanceBackend` implements `ConformanceBackend` with `BackendType::Trino`, a
  `VARCHAR` `string_type` override (Trino has no `STRING` type), and the default `EXCEPT ALL`
  (Trino supports it natively — measured, no override needed) and default `oracle_relation`
  override path shared with BigQuery's inline-derived-table form (no session temp view on Trino).
- `ConformanceBackend::excluded_constructs()` — a new default-empty trait hook
  (`families/mod.rs`) mirroring the crate's existing per-backend-capability overrides
  (`multiset_diff_sql`, `storage_clause`, `string_type`). `RecipePool::partition_append_only_excluding`
  filters the six-construct pool by it; the two `families::gate` legs that sample the full pool
  now call `RecipePool::partition_append_only_excluding(b.excluded_constructs())` instead of the
  raw pool.
- `TrinoConformanceBackend::excluded_constructs()` returns `&[ConstructKind::HolisticAgg]` —
  see Decisions below.
- `.github/workflows/compat.yml`: new `maintenance-conformance-trino` job (same trigger discipline
  as `trino-integration`/`maintenance-conformance-spark`: schedule / `run-docker-tests` label /
  `needs.changes.outputs.trino`), brings the tier up/down, loud-fails on any `skipping` line.
  `trino_ci_wiring.rs`'s census widened to accept coverage from either Trino job.
- `docs/specs/multi_backend.md` §"Generative equivalence coverage" names the Trino leg's gated
  command; §Known Divergences rewritten to reflect the actual final state (all six legs pass; the
  pool excludes `HolisticAgg` on Trino rather than failing on it).

**Decisions:**
- Live-measured: Trino has no `MEDIAN`, and no exact ordered-set aggregate either (`percentile_cont
  ... WITHIN GROUP` is not valid Trino syntax; only the approximate `approx_percentile` exists).
  I independently verified an exact ARRAY_AGG-based lowering works on live Trino (matches DuckDB's
  interpolated median for even/odd/null-bearing inputs) — the same technique BigQuery's
  `RewriteId::BigQueryMedian` uses. I deliberately did NOT implement it as a new `RewriteId` in
  `smelt-types`/`smelt-dialect`: that crosses into the Function-registry single-ownership area
  (`crates/smelt-db/tests/dialect_audit/ledger.rs` already carries a `MEDIAN`/Trino gap row, issue
  #209), which has its own standing live-tier gates (`dialect_audit`'s Trino schema+value legs)
  this phase's task list never mentioned touching, and the invariant explicitly wants such
  extensions reviewed rather than landed opportunistically inside an unrelated phase.
- Instead, excluded `HolisticAgg` from Trino's generative recipe pool via a new
  `ConformanceBackend::excluded_constructs()` hook — surgical, in-crate, matches the pool's own
  documented precedent (Spark/BigQuery already leave whole *families* out of their Trino/BigQuery
  pool for genuine per-backend reasons, recorded in the same Known Divergences entry). This lets
  criterion 7 ("equivalence proved ... under actual availability") hold honestly: the pool no
  longer samples a call the registry cannot answer for, rather than sampling it and asserting
  failure is fine.
- Did not add `admission_rate_stays_above_floor_on_trino`'s exclusion as a no-op: it was already
  green before this fix (admission is pure classification, never executes SQL), but I applied the
  same `excluded_constructs()` filter there too for consistency — its admission-rate floor should
  reflect the same reachable pool the equivalence leg actually exercises.

**For the next planner:**
- Follow-up: give Trino an exact `MEDIAN` lowering (`RewriteId::TrinoMedian` or similar) in
  `crates/smelt-types/src/signatures/builtins/extended_aggregates.rs`, mirroring
  `BigQueryMedian`'s ARRAY_AGG-based structure (aggregate position: sort+index; whole-partition
  window: reuse `RestructureId::WindowToCte`; running window: `Emission::Unsupported`, no exact
  form exists). This closes issue #209, requires updating `dialect_audit/ledger.rs`'s gap row,
  `docs/reference/dialect-coverage.md` (regenerated), and verifying via the live
  `cargo test -p smelt-db --test dialect_audit` Trino legs — separate scope from this outcome's
  harness work, but directly unblocks re-including `HolisticAgg` in Trino's pool.
- Phase 8b (already queued) should widen the pool to the families criterion 2 admits — plan
  already scopes this correctly.
- The `maintenance-conformance-trino` CI job is gated (schedule/label/path-triggered only), so it
  hasn't run in real CI yet; first live trigger should be watched.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test trino_ci_wiring --test trino_incremental_spec_freshness --quiet`
  — 8+8 passed.
- `cargo test -p smelt-maintenance-testkit --quiet` — 85+4 passed.
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — 104 passed (DuckDB leg
  unaffected).
- `cargo build -p smelt-cli --features smelt-cli/spark --test maintenance_conformance_spark` —
  compiles clean.
- Live: `bash scripts/trino-up.sh && source scripts/trino-env.sh && cargo test -p smelt-cli --test
  maintenance_conformance_trino -- --test-threads=1` — **10/10 passed** (all six `families::gate`
  legs plus the 3 backend unit tests plus the harness self-check), then `bash
  scripts/trino-down.sh`. No `skipping` line with the tier up.
- `.claude/large-file-baseline.txt` updated (`--update`, reviewer sign-off: this phase) for
  `link_c_harness.rs`/`recipe.rs`/`render.rs`'s legitimate growth from the new Trino arm, plus one
  pre-existing orphaned-baseline gap from phase 7 (`statement_parity/structural_and_ledger.rs` had
  no baseline entry at all) picked up incidentally by the same `--update` run.
