# Phase 5 summary — Generative conformance pool: nullable payload, once-write NULL direction covered

**Shipped:**
- `GenRow::val` is now `Option<i64>` (`crates/smelt-maintenance-testkit/src/schedule_gen.rs`), with
  `GenRow::new(d, id, val)`, `GenRow::null(d, id)`, and `GenRow::val_sql()` (the single NULL-or-literal
  SQL rendering seam). Threaded through every construction/read site in `smelt-maintenance-testkit`
  (`recipe.rs`, `feed.rs`, `probes.rs`, `s_tracker.rs`, `families/*.rs`) and `smelt-cli`'s
  `tests/maintenance_conformance/{gate,pinned,probes,contract_points}.rs`.
- `STracker::s_view_select_sql`'s leading UNION branch now emits `CAST(NULL AS INTEGER) AS val` for a
  NULL-payload first row, so the union's column type is never the untyped NULL type.
- `read_source_snapshot` (DuckDB) and `read_source_snapshot_via_backend` (Arrow) now read a NULL
  payload as `None` rather than coercing/panicking.
- New generator `arb_once_write_null_schedule` (`recipe.rs`): a shared re-touched key draws exactly
  one non-NULL value with a NULL prefix in earlier windows (world-fact-preserving by construction),
  plus one fresh non-NULL key per window.
- Four new tests (all green): `gen_row_null_payload_renders_sql_null`,
  `s_view_select_sql_types_a_leading_null_payload`,
  `arb_once_write_null_schedule_preserves_the_world_fact` (testkit units), and
  `once_write_null_pool_upholds_end_state_equivalence` (`smelt-cli --test maintenance_conformance`,
  crossing the generator with all three once-write spellings).
- The pre-existing hand-written `once_write_null_payload_then_value_upholds_equivalence` is retitled
  in its own doc comment as a pinned minimal witness; its stale "non-nullable" rationale is rewritten.
- `docs/specs/incremental_shapes.md` §Known Divergences: deleted the "generative conformance pool
  cannot stage NULL payloads (Open Question)" bullet — the gap it named is closed.

**Decisions:**
- `arb_payload_value()` (the general append-only/keyed pool's own draw) stays non-NULL — only the
  dedicated once-write generator draws NULLs, per the outcome's own phase-5 scoping decision.
- Fresh per-window keys in `arb_once_write_null_schedule` always draw a definite value; only the
  shared re-touched key exercises the NULL-then-value direction — keeps the generator's world-fact
  invariant simple to state and check.

**For the next planner:**
- **Blocked verification item, pre-existing and unrelated to this phase:** the plan's required
  `cargo check -p smelt-cli --tests --features smelt-cli/spark` and the BigQuery twin both fail to
  compile — but the failure is entirely in `smelt-maintenance-testkit/src/families/gate_composed.rs`'s
  call to `smelt_runtime::maintenance_driver::run_windowed_keyed_maintenance` (arg-count/closure-type
  mismatch: the function now takes 12 args including a `&RetryPolicy<'_>`, the call site still
  supplies 11 and passes a `RetryPolicy` where a `FnMut(&MaintenanceStep)` closure is expected).
  Confirmed pre-existing by stashing this phase's changes and re-running the same check against the
  base commit — identical 5 errors, none mentioning `GenRow`/`Option`/`val`. Default-feature builds
  (`cargo check -p smelt-cli --tests`, no spark/bigquery) are clean; every other listed verification
  command passed. This gate needs a small, separate fix in `gate_composed.rs` (or
  `maintenance_driver.rs`'s signature) before it can go green — worth its own short follow-up phase or
  bug fix, since it currently blocks ANY change from passing that specific compile check.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, full workspace
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-maintenance-testkit` — 56 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 75 passed.
- `cargo check -p smelt-cli --tests` (no spark/bigquery) — clean.
- `cargo check -p smelt-cli --tests --features smelt-cli/spark` / `--features smelt-cli/bigquery` —
  FAIL, pre-existing unrelated bug (see above), not caused by this phase.
- `rg -n 'GenRow::val' docs/specs/` — empty (spec delta applied).
