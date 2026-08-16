# Phase 2 — Probe emitters for FD, `bounded_domain`, append-only posture, `assert_monotonic`

**Outcome:** `docs/outcomes/20260809-probe-backed-facts/outcome.md`
**Advances success criteria:** 2 (each of the four declarations gets a probe emitter in the
single-owner maintenance layer). Partially 4 — the emitter carries the diagnostic's payload
(violated fact + sample), the driver that raises it is phase 4.

## Objective

Add the four missing probe emitters to `crates/smelt-logical/src/maintenance/emit.rs`, the
single owner of maintenance statement text, so the registry rows for `functional_dependencies:`,
`bounded_domain:`, source append-only posture, and `timeseries.assert_monotonic` move from
`not-yet` to `built (unwired)`. Pure emitters only: no runtime dispatch, no cadence, no
`DiagnosticCode` variants (those probes fire as transactional run refusals, phase 4). Every
emitter is proved to actually execute on a real DuckDB and to discriminate conforming from
violating data.

## Design constraint — one probe result shape

Every probe returns exactly one row with the columns `violation_count` (integer) and
`sample_keys` (string, ≤5 comma-joined offending identifiers) — the contract
`maintenance_driver`'s existing recurrence gate already reads. Extract the dialect-keyed
wrapper (`VARCHAR`/`STRING_AGG` vs `STRING`/`CONCAT_WS(COLLECT_LIST(...))`) out of
`emit_recurrence_bound_probe` into a private helper the four new emitters share.
`emit_recurrence_bound_probe`'s emitted SQL must stay byte-identical (a golden test pins it) —
`statement_parity` and `technique_lowering` compare against it.

## Spec delta (spec-first; the implement step makes these edits)

`docs/specs/model_properties.md` §"Probe obligation":
- Probe-registry rows 1–4 (`assert_monotonic`, `functional_dependencies:`, `bounded_domain:`,
  source `mutation_profile.kind: append_only`): **Status** `not-yet` → `built (unwired)`, and
  the **Probe** cell names the emitter (`emit_monotonicity_probe`,
  `emit_functional_dependency_probe`, `emit_bounded_domain_probe`,
  `emit_append_only_posture_probe`) the way rows 5–6 already name theirs.
- §Known Divergences: the "five not-yet rows" bullet becomes — four rows now have emitters but
  no live run dispatches them (tracking: this outcome's phases 3–4); `unique_key` /
  `delta_identity` remains the one `not-yet` row.
- One sentence in §"Probe obligation" stating the shared result shape above (a probe's answer is
  a single `violation_count`/`sample_keys` row), so the driver contract is specified, not
  implied by one emitter's code.

## Tests (red-green)

New unit tests in `crates/smelt-logical/tests/emit_statements.rs`:
1. `functional_dependency_probe_counts_keys_with_multiple_determines` — re-aggregates the
   declared `key` over the scope select, counting keys with >1 distinct `determines`.
2. `bounded_domain_probe_counts_distinct_over_cap` — emits a distinct-count of the declared
   column compared against `max_cardinality`; zero when within cap.
3. `monotonicity_probe_flags_out_of_order_event_time_per_partition` — `LAG` over the declared
   partition key ordered by event time; a row below its predecessor is a violation.
4. `append_only_posture_probe_flags_shrunk_partition_and_changed_fingerprint` — current
   per-partition `COUNT(*)` + skeleton-column fingerprint compared against a caller-supplied
   recorded baseline (rendered as a `VALUES` list); either a decreased count or a changed
   fingerprint counts.
5. `every_probe_emitter_returns_violation_count_and_sample_keys` — table-driven over all six
   probe emitters × both `MaintenanceDialect`s; asserts the shared column contract and that the
   Spark rendering uses no `STRING_AGG`/unsized `VARCHAR`.
6. `recurrence_bound_probe_sql_is_unchanged_by_the_shared_wrapper` — golden string; the
   refactor is behaviour-preserving.
7. `probe_emitters_reject_empty_key_arguments` — empty key / empty digest-column vectors are a
   fail-loud panic in the emitter, not a degenerate always-passing query.

New executability test file `crates/smelt-logical/tests/probe_execution.rs` (in-memory DuckDB
via the existing dev-dependency), one pair per new emitter:
8. `<fd|bounded_domain|monotonicity|append_only>_probe_returns_zero_on_conforming_data`
9. `<same four>_probe_returns_nonzero_with_samples_on_violating_data`

Gate extension in `crates/smelt-logical/tests/probe_obligation.rs`:
10. `built_and_unwired_rows_name_a_real_emitter` — widen the existing `built`-only
    emitter-existence assertion to also cover `built (unwired)` rows, so the four new rows are
    held to a real `pub fn emit_*` symbol.

## Tasks

1. Make the spec edits above (registry Status/Probe cells, Known Divergences, result-shape
   sentence).
2. Widen `probe_obligation.rs`'s emitter-existence gate to `built (unwired)` — red.
3. Extract the dialect-keyed `violation_count`/`sample_keys` wrapper helper from
   `emit_recurrence_bound_probe`; add the golden test proving its SQL is unchanged.
4. Add `emit_functional_dependency_probe(scope_select, key, determines, dialect)`.
5. Add `emit_bounded_domain_probe(scope_select, column, max_cardinality, dialect)`.
6. Add `emit_monotonicity_probe(scope_select, partition_key, event_time_column, dialect)`.
7. Add `emit_append_only_posture_probe(source_table, partition_column, digest_columns,
   baseline, dialect)`, reusing `column_fingerprint_expr`/`row_fingerprint_expr` rather than
   re-authoring hashing SQL.
8. Doc-comment each emitter with its registry row, its diagnostic name, and the
   `sources.md`/`model_properties.md` section it implements (house style in this module).
9. Write the two test files; make everything green.

## Verification

- `bash .claude/scripts/verify-phase.sh` (needs `DUCKDB_LIB_DIR=~/.local/lib/duckdb` and
  `LD_LIBRARY_PATH=~/.local/lib/duckdb:$LD_LIBRARY_PATH` in this worktree).
- `cargo test -p smelt-logical --test emit_statements --test probe_execution --test probe_obligation`
- `cargo test -p smelt-runtime --test statement_parity --test technique_lowering` — the
  refactor must not perturb any executed-vs-emitted comparison.

## Commit message

`feat(probes): FD, bounded-domain, monotonicity and append-only posture probe emitters`
