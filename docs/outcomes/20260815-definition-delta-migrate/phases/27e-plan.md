# Phase 27e plan — the sidecar delta reaches live delta-restriction admission

## Objective

Serves success criterion 17 (conditional-maintenance gaps). Today the fingerprint
sidecar (`_smelt_fingerprint_sidecar`, `diff_fingerprint_sidecar_changed_keys`) and the
enrichment restrict-column resolver (`choice::enrichment_restrict_column`) exist and are
exercised only from `crates/smelt-runtime/tests/technique_lowering.rs`'s
`external_source_point_lookup_recompute` module, which states outright that it drives the
mechanism "directly rather than through `execute_project`". `execute.rs` builds
`DeltaRestrictionFacts` only when `model_edges` is non-empty, so a model whose only
mutation-sensitive input is an external `mutable_snapshot` dimension always takes the
widened scan. This phase wires the sidecar-derived changed-key set into the live
`execute_project` dispatch, and makes the non-DuckDB fallback a declared backend
capability instead of an inline dialect equality check.

## Spec delta (spec-first — make these edits before the code)

1. `docs/specs/multi_backend.md` §Surface capability matrix — new row
   `supports_fingerprint_sidecar`: `true` for DuckDB, `false` for spark_delta,
   spark_parquet and BigQuery. One sentence: a backend without it keeps the widened-scan
   recompute for a mutable external dimension; the gap is declared, never a silent
   narrowing.
2. `docs/specs/incremental_models.md` §Known Divergences, "Conditional-maintenance gaps"
   bullet — delete the clause "delta-restriction admission doesn't yet consume an external
   `mutable_snapshot` source's fingerprint-sidecar delta"; restate the surviving
   "non-DuckDB targets keep the widened-scan recompute" clause as gated on
   `supports_fingerprint_sidecar`.
3. `docs/specs/sources.md` — §"Landed-delta" prose (the "built for DuckDB as a standalone
   capability, not yet wired into this section's own live per-source delta consumption"
   clause) and the matching §Known Divergences bullet: narrow to what stays open (the
   *graph layer*'s own per-source delta still widens to whole-table; only the
   delta-restricted recompute consumes the sidecar live).
4. `docs-site/docs/guide/incremental-models.md` — one sentence naming the DuckDB-only
   scope of the sidecar-restricted recompute, so the user-facing text does not over-promise
   on Spark/BigQuery.

## Tests (red first)

- `smelt-dialect` `capability_conformance::every_flag_matches_matrix` — extended with
  `supports_fingerprint_sidecar` for all four constructors (fails until the flag exists).
- `maintenance_driver` unit `external_facts_resolve_for_a_declared_mutable_dimension` — the
  `examples/timeseries/daily_events_enriched` shape resolves external restriction facts
  (Closed declared-RI closure + single-column `user_id` restrict column).
- `maintenance_driver` unit `external_facts_refuse_a_composite_dimension_key` — composite
  `unique_key` yields `None` with a stated `why`, never a narrowed delta.
- `maintenance_driver` unit `external_facts_refuse_without_the_sidecar_capability` — a
  capabilities set with `supports_fingerprint_sidecar: false` resolves `None` (widened
  scan), asserting the fallback is capability-driven.
- `smelt-runtime/tests/external_source_delta_restriction.rs` (new, real DuckDB, through
  `execute_project`) — `mutation_of_one_dimension_row_restricts_the_recompute`: after
  mutating exactly one `raw.users` row, the executed statement group's DELETE/INSERT
  carries the sidecar-derived restriction on `user_id`, and the rebuilt table equals a
  full-refresh oracle.
- same file — `absent_sidecar_first_run_takes_the_widened_scan`: no sidecar partition yet
  ⇒ every row "changed" ⇒ unrestricted statement group, and the sidecar is populated for
  the next run.
- `smelt-runtime --test statement_parity` — the external-delta restricted family's
  executed statements are byte-identical to the emitter's output (extend the existing
  delta-restricted family entry rather than adding a parallel one).

## Tasks

1. Land the four spec/doc edits above.
2. Add `supports_fingerprint_sidecar` to `BackendCapabilities` (`smelt-dialect/src/dialect.rs`)
   and its four constructors; extend `capability_conformance`.
3. Generalize the delta acquisition inside `execute_delete_insert_with_delta_restriction`
   into one `RestrictionDeltaSource` enum — `ModelEdge { upstream_model, window_start,
   window_end }` (today's `read_observed_delta_changed_keys` route, unchanged) and
   `ExternalSidecar { source_address, source_table, source_key, projection,
   all_source_columns, model_sql }` (`diff_fingerprint_sidecar_changed_keys`). Keep ONE
   executor: the probe dispatch, `resolve_recompute_restriction` call and emitter path stay
   shared; existing call sites pass `ModelEdge` and must not change behaviour.
4. Add `resolve_live_external_delta_restriction_facts` beside
   `resolve_live_delta_restriction_facts`, routing through the same RI-aware plan
   derivation (`derive_model_maintenance_plan_with_edges` with the real
   `SourceReferentialIntegrity`) to read the live `Trigger::UpstreamMutation` cell's
   `skeleton_source_closure`; resolve the restrict column with
   `choice::enrichment_restrict_column` over the source's declared `unique_key`. Return
   `None` — with a `tracing::debug!`-logged `why` — when the closure is not `Closed`, the
   key is composite/undeclared, or the capability is absent.
5. `execute.rs`: where `delta_restriction_facts` is `None` because `model_edges` is empty,
   resolve external facts and dispatch through the generalized executor, under the same
   `DeleteInsert`-strategy and target-exists gating the model-edge path already applies.
6. Refresh the sidecar after a successful restricted write (`refresh_fingerprint_sidecar`),
   inside the existing write path's ordering so a failed write never advances it.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-dialect --test capability_conformance`
- `cargo test -p smelt-runtime --test external_source_delta_restriction --test technique_lowering --test delta_restricted_recompute`
- `cargo test -p smelt-runtime --test statement_parity`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `cargo test -p smelt-cli --test example_diagnostics`

## Commit message

`feat(maintenance): consume an external mutable_snapshot source's fingerprint-sidecar delta in live delta-restriction dispatch`
