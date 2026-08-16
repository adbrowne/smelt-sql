# Phase 2 summary — dispatch the key-addressed repair cell outside `grain: key`

## Shipped

- `crates/smelt-runtime/src/execute.rs`: shared inputs for key-addressed resolution
  (`db_table_name`, `clean_sql_for_merge`, `maint_source_facts`, `explicitly_mutable`,
  `table_exists_before_run`, `keyed_model_edges`, `key_addressed_edge_cell`) hoisted above the
  `plan_is_keyed` gate and resolved exactly once, shared by both dispatch sites (maintenance-plan
  purity — no second derivation). The keyed branch's execution arm is extracted into
  `dispatch_key_addressed_model_edge`. A new non-keyed dispatch site runs before
  `match plan.incremental.as_ref()`, gated by a widen-never-narrow substitution rule: every
  inbound ref of the model must be the single resolved key-addressed model edge (no declared
  source, no other/non-key-addressed edge) before the ordinary route is skipped.
- `crates/smelt-runtime/tests/key_addressed_model_edge_lowering.rs`: three new tests — the
  dispatch RED→GREEN (`partition_grain_downstream_dispatches_key_addressed_cell`), the
  substitution-gate pin (`partition_grain_downstream_with_an_uncovered_input_keeps_the_ordinary_route`),
  and the unit-level characterization (`key_addressed_cell_resolves_for_a_partition_grain_downstream`).
- `crates/smelt-cli/tests/maintenance_conformance/dags.rs`: `keyed_upstream_partition_downstream_matches_oracle`
  extended with an incrementality assertion — `dag_kpart_b`'s repair-run manifest strategy is
  `per_group_recompute`.
- `docs/specs/incremental_models.md` §Known Divergences: the `grain: partition` inert-cell clause
  is removed; the divergence now names the surviving composed-input residue.

## Decisions

- Fixed a pre-existing derivation bug found while landing the fixture: `analysis::walk`'s
  `group_by_output_keys` matches `GROUP BY` keys against a select item's own expression text, not
  its output alias, so `GROUP BY d, user_id` (grouping by a projected alias `d`) failed grain
  proof entirely rather than dropping only the alias column. Worked around for this phase by
  dropping the independent constant `d` from `GROUP BY` in both the test fixture and
  `keyed_partition_sink_dag` (a literal projection is trivially single-valued per group; valid
  SQL) rather than fixing the walk — that's `smelt-logical`'s gated composition layer, out of a
  dispatch-only phase's scope.
- Substitution gate is deliberately conservative (single resolved edge, zero other refs) per the
  plan — composed multi-component dispatch is phase 3's explicit scope, not squeezed in here.

## For the next planner

- **Real gap, not urgent**: `group_by_output_keys` (alias-vs-expression-text mismatch) is a
  genuine walk limitation — DuckDB/Postgres both accept `GROUP BY <output alias>`. Worth a
  dedicated fix before phase 3 leans harder on grain proofs over grouped shapes with derived
  columns.
- **Sharper edge for phase 3**: `derive_affected_keys` returns every grain column into
  `KeyScope` regardless of source dependency, which is why an alias-based `GROUP BY d, user_id`
  hard-refused (`MaintenanceKeyScopeColumnMissing`) rather than gracefully dropping the
  independent `d` column. Composed multi-component dispatch may hit the same corner when a
  downstream has several key-scoped inputs with different grain columns.
- Everything else (key-valued dirt-sets, live `--since-upstream`, watermark persistence,
  `smelt explain` headline, conformance/validate close-out) is unchanged — still phases 3–8.

## Gates

- `cargo test -p smelt-runtime --test key_addressed_model_edge_lowering` — 9/9 pass
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` — 4/4 + 23/23 pass
- `cargo test -p smelt-cli --test maintenance_conformance` — 76/76 pass
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md` — no matches
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full
  `cargo test` workspace, `example_diagnostics`)
