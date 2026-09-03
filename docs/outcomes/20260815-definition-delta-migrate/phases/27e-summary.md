# Phase 27e summary — delta-restriction admission consumes an external `mutable_snapshot` source's fingerprint-sidecar delta

## Shipped

- `BackendCapabilities::supports_fingerprint_sidecar` (`crates/smelt-dialect/src/dialect.rs`) —
  `true` for DuckDB, `false` elsewhere; extended `capability_conformance.rs`'s matrix assertion
  and exhaustiveness-destructure gate.
- `RestrictionDeltaSource` enum (`crates/smelt-runtime/src/maintenance_driver.rs`) generalizing
  `execute_delete_insert_with_delta_restriction`'s delta acquisition over the existing model-edge
  route (`ModelEdge`, unchanged behaviour) and the new external-source route (`ExternalSidecar`,
  via `diff_fingerprint_sidecar_changed_keys`). All ~16 call sites across `execute.rs` and 6 test
  files threaded through mechanically.
- `resolve_live_external_delta_restriction_facts` + `ExternalDeltaRestrictionFacts` — resolves the
  live `Trigger::UpstreamMutation` cell's P1 closure and `enrichment_restrict_column` restriction
  for an explicitly-mutable external source, gated on `supports_fingerprint_sidecar`. 3 new unit
  tests (`maintenance_driver.rs`).
- `execute.rs` wiring: when a model reads no maintained-model upstream (`model_edges` empty), the
  batch loop now also resolves external facts and dispatches through the generalized executor
  under the same `DeleteInsert`-strategy/target-exists gating the model-edge route uses; the
  sidecar is refreshed after a successful restricted write.
- New end-to-end test `crates/smelt-runtime/tests/external_source_delta_restriction.rs` — drives
  the mechanism through real `execute_project` (not a direct driver call, unlike the existing
  `technique_lowering.rs` proof), 2 tests, real DuckDB.
- Bug fix (found and fixed as part of this phase, blocking its own verification):
  `emit_count_preservation_probe_from_body`/`select_with_enrichment_join`
  (`crates/smelt-logical/src/maintenance/emit.rs`) only unwrapped the SQL compiler's own cast-wrap
  shape (`_smelt_typed`), never `smelt-runtime`'s `inject_time_filter` output-clamp wrap
  (`_smelt_output_clamp`) — so a live, time-filtered run's declared-`referential_integrity` probe
  silently failed to build, dropping delta restriction on every real call. Now accepts both wrap
  aliases. This also affects the **pre-existing model-edge route**, not just this phase's own —
  it was previously untested against a real compiled `body_sql`.
- Second bug fix: the probe's `enrichment_source` argument must be the join's PHYSICAL table text
  (e.g. `main.sources_raw_users`), not the closure's own bare logical address (`raw.users`) — the
  two only coincide for a model edge (whose physical name has no naming-convention prefix over its
  bare address); an external source's `sources_` naming-convention prefix breaks the match.
  `execute_delete_insert_with_delta_restriction` now derives the probe's enrichment-source name
  from `delta_source` itself (`source_table` for `ExternalSidecar`) rather than trusting the
  closure's embedded field, for both routes.
- Spec edits: `multi_backend.md` (capability matrix row + new §"The fingerprint sidecar
  capability"), `incremental_models.md` (Known Divergences narrowed), `sources.md` (Landed-delta
  prose + Known Divergences bullet narrowed), `docs-site/docs/guide/incremental-models.md` (one
  sentence on DuckDB-only scope).

## Decisions

- Reused `daily_events_enriched`'s exact shape but added `RANDOM() AS jitter` to the test fixture.
  Reason: the pre-existing phase-27c keyless whole-row staged-candidate mechanism
  (`resolve_live_membership_recompute_cell`) unconditionally wins the SAME `UpstreamMutation` cell
  ahead of this phase's dispatch for any `grain: partition` model, UNLESS at least one output
  column has no P3 comparability proof (refusing `StagedKeyless` fail-closed). `daily_events_enriched`
  itself is always claimed by 27c live — its own doc comment's claim that "MP11 wires a live
  column-scoped MERGE" for its `{user_name}` cell turned out to be stale/aspirational; the live
  derivation actually resolves `Technique::DeleteInsert` (membership-sensitive INNER JOIN), and
  27c's keyless mechanism — not this phase's — is what dispatches for it today.
- Fixed the two probe-building bugs above rather than working around them, since they block this
  phase's own verification and (per the first one) silently defeat the pre-existing model-edge
  route too whenever a live run's compiled body carries a time-filter wrap — i.e. always, for any
  batched incremental run. Both are minimal, targeted fixes with no behavior change for a caller
  whose `probe_body`/`source` already matched (existing unit/integration tests for the model-edge
  route all still pass unchanged).
- Threaded real declared `referential_integrity:` facts from `source_infos` into the new
  `execute.rs` call site (previously the sibling model-edge resolver hardcoded an empty
  `SourceReferentialIntegrity`, internally, un-parameterized) — needed for the golden path to be
  reachable at all against a real project's source YAML declarations.

## For the next planner

- **The model-edge route (`resolve_live_delta_restriction_facts`/`DeltaRestrictionFacts`) has
  likely never actually applied its restriction on a real `execute_project` run before this
  phase's probe-wrap fix**, whenever the driving cell resolves `DeclaredReferentialIntegrity` (vs.
  `JoinShape`, which needs no probe and was unaffected). Every existing test for it either (a)
  drives `execute_delete_insert_with_delta_restriction` directly with a hand-built, unwrapped
  `body` (bypassing `inject_time_filter` entirely), or (b) uses a `JoinShape` closure. Worth a
  follow-up: an `execute_project`-driven model-edge test with a `DeclaredReferentialIntegrity`
  closure, to confirm the fix actually closes this gap for that route too (I did not add one —
  out of this phase's own scope, but directly adjacent).
- The `--full-refresh`/first-materialization interaction with the sidecar is untested here beyond
  "creation never refreshes it" (confirmed via `absent_sidecar_first_run_takes_the_widened_scan`).
  A `--full-refresh` re-run over an *existing* target's sidecar-populated history isn't covered.
- `docs/specs/incremental_models.md` §Known Divergences still carries the `write:` pin
  keyed-MERGE/staged-candidate bullet (27d/27g territory) — untouched, out of scope here.
- Two retries occur per real DuckDB write in the new end-to-end test (visible during debugging,
  both statement groups byte-identical) — did not investigate root cause; harmless for the test's
  own assertions (idempotent write) but worth a look if `err.is_transient()` is over-broad for a
  fresh local DuckDB connection.

## Gates

- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, full workspace
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-dialect --test capability_conformance` — PASS (2/2).
- `cargo test -p smelt-runtime --test external_source_delta_restriction --test technique_lowering --test delta_restricted_recompute` — PASS (2 + 32 + 4).
- `cargo test -p smelt-runtime --test statement_parity` — PASS (27/27).
- `cargo test -p smelt-cli --test maintenance_conformance` — PASS (74/74).
- `cargo test -p smelt-cli --test example_diagnostics` — PASS (119/119, 1 ignored).
