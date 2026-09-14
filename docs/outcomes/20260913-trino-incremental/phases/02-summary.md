# Phase 2 summary — Spec delta: Trino's maintenance surface stated

**Shipped:**
- `docs/specs/multi_backend.md` §"Whole-row MERGE" now names Trino as a third spelling family
  (distinct from DuckDB/Spark's star form and GoogleSQL's `INSERT ROW`): both arms rendered
  column-by-column over `CompiledModel::output_columns`, with the three refused shorthands and
  their measured parse errors quoted.
- §"Column-scoped merge and conditional-write capabilities" states Trino as the second
  `supports_merge_not_matched_by_source = false` backend, over the non-atomic
  `TargetSchema`-resident staged group (not the session-temporary atomic group every prior `false`
  case had), and lists the four clause forms phase 1 measured accepted — `WHEN MATCHED THEN
  DELETE`, the `AND <pred>` guard, first-match-wins over two ordered arms, and the `USING (SELECT
  … FROM <staged>)` subquery source.
- §"Incremental & schema evolution per backend" gained a per-`Technique` landing-state table
  (reachable / downgraded / refused, with diagnostic) derived from
  `smelt_logical::maintenance::availability`, plus a closing paragraph stating the three existing
  diagnostic codes cover every route and no Trino-specific code exists.
- New gate `crates/smelt-cli/tests/trino_incremental_spec_freshness.rs` (6 tests), checking the
  new prose against the pure availability functions and the diagnostics catalogue rather than
  against itself.

**Decisions:**
- Reused the existing three diagnostic codes (`MaintenanceStateDowngraded`,
  `UnsupportedOnBackend`, `DeclaredContractRequiresState`) rather than minting a Trino one — no
  route in the derived table needed a new shape.
- The per-technique table's verdicts were checked against `required_state_structure`'s and
  `realisable_state_structures`'s actual source (read as text in the test, since building a
  `PlanCell` has no public test constructor and this phase is spec-only) rather than asserted
  from the outcome's prose alone.

**For the next planner:**
- No `PlanCell` test-construction helper exists in `smelt-logical`; a later phase that wants to
  assert `resolve_availability`'s behavior directly (rather than via source-text checks, as this
  phase's freshness gate does) will need one, or will have to build cells the long way as
  `crates/smelt-logical/src/maintenance/choice/tests.rs` already does internally.
- Phase 3 lands `maintenance_dialect` for `SqlDialect::Trino` (today `Err`) — nothing here changed
  that; the spec's table is written for when it does.
- No divergence text needed rewriting; the existing "realises none of the five correctness
  structures" paragraph absorbed the new table cleanly.

**Gates:**
- `cargo test -p smelt-cli --test trino_incremental_spec_freshness` — 6 passed.
- `cargo test -p smelt-cli --test trino_spec_freshness --test trino_emission_spec_freshness` — 12
  passed (unchanged).
- `cargo test -p smelt-core --test trino_docs_freshness` — 6 passed (unchanged).
- `bash .claude/scripts/large-file-check.sh` — OK, no ratchet moved.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full workspace `cargo test`, example_diagnostics).
