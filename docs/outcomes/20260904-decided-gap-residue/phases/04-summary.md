# Phase 4 summary — sidecar per-consuming-edge namespace fix

## Shipped

- `_smelt_fingerprint_sidecar`'s row key is now `(source_address, projection_identity,
  consumer_address, source_key)` — a fourth namespace/PK column (`crates/smelt-state/src/
  ddl_duckdb.rs`: table DDL, refresh upsert `ON CONFLICT`, stale-check, partition-exists, GC).
- `consumer_address` threaded through the emitter (`crates/smelt-logical/src/maintenance/
  emit.rs::{emit_fingerprint_sidecar_diff, emit_repair_group_sidecar_diff,
  sidecar_diff_over_digest_select}`) and every `smelt-runtime` entry point
  (`diff_fingerprint_sidecar_changed_keys`, `refresh_fingerprint_sidecar`,
  `diff_repair_group_sidecar_changed_keys`, `refresh_repair_group_sidecar`,
  `resolve_key_addressed_affected_keys`, `execute_key_addressed_model_edge_cell`,
  `RepairSidecarRefresh`, `RestrictionDeltaSource::ExternalSidecar`).
- Live call sites in `crates/smelt-runtime/src/execute.rs` now supply the real consuming
  model's address (`smelt.models.<canonical_path>`) rather than a placeholder.
- `Backend::execute_write_and_refresh_fingerprint_sidecar`'s own signature is unchanged — the
  namespace is baked into the pre-built SQL strings it's handed, not a new trait parameter.
- 3 new tests in `crates/smelt-runtime/tests/fingerprint_sidecar.rs`: byte-identical-body
  consumers don't share a comparandum; each edge keeps its own comparandum across interleaved
  runs; a sibling's refresh leaves this edge's stored rows untouched (asserted on raw rows).
- `docs/specs/sources.md` §"The fingerprint sidecar" — "Naming and namespace" rewritten (row
  key, sharing deliberately not taken) and the Known Divergences bullet closed to the residual
  gap (orphaned-partition GC, un-taken shared-digest-with-cursors optimization).
- `docs/TODO.md`'s "Sidecar per-consuming-edge audit" bullet removed.

## Decisions

- Namespace fix over stamp-fold: folding the consumer into `stamp` would fix the unsoundness
  but keep the clobber-thrash (siblings still mutually invalidate or unsoundly satisfy each
  other). A fourth PK column is what "each edge gets its own comparandum" means physically.
- Shared-digest-with-consumption-cursors is named as a future option in the spec but not built
  — the storage saving doesn't justify the cursor machinery this early.
- `Backend` trait needed no signature change: `execute_write_and_refresh_fingerprint_sidecar`
  only ever receives pre-built SQL text; the namespace change is entirely upstream of it.

## For the next planner

- Orphaned-partition GC (a deleted/redefined consumer's stale rows) is still unswept — named in
  the spec's lifecycle bullet, unchanged by this phase, still open.
- Not touched: `smelt-cli/tests/maintenance_conformance` fixtures don't exercise the multi-
  consumer scenario end-to-end through `execute_project`; the new coverage is at the
  `smelt-runtime` direct-dispatch level (matching the plan's own scope — live multi-consumer
  wiring through the real trigger/technique-selection path is separately unbuilt per the
  existing "live dispatch remains" divergence).
- Phase 3's blocked once-write generative-pool clause is untouched by this phase; still needs
  the human decision recorded in `## Blocked`.

## Gates

- `cargo test -p smelt-runtime --test fingerprint_sidecar` — 16 passed (3 new).
- `cargo test -p smelt-runtime --test statement_parity` — 37 passed.
- `cargo test -p smelt-state --lib ddl_duckdb::` — 55 passed.
- `cargo test -p smelt-logical --lib maintenance::emit::` — 43 passed.
- `cargo test -p smelt-runtime --test external_source_delta_restriction --test
  delta_restricted_recompute` — 6 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 79 passed.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
