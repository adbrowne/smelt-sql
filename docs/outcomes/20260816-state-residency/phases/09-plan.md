# Phase 9 plan — backend-aware downgrade visibility in `smelt explain`

## Objective

Close criterion 3's "visible in `smelt explain`" half. Phase 5 landed
`MaintenanceStateDowngraded` and its renderer, but every derivation path `smelt explain`
reaches passes `StateAvailability::all()`, so the only projects that actually downgrade (a
Spark-targeted model, no ledger/frontier builder) print no downgrade line and the `--json`
report omits `state_downgrades` entirely. This phase resolves the model's declared target
dialect into real availability at those call sites and carries the field into the JSON report.

## Spec delta

None required — `state.md` §Surface already states that `smelt explain <model>` "prints every
downgraded cell with both the executed technique and the technique that *would* run". This
phase makes the code match a sentence the spec already owns.

One exception, decided during implementation: if `--json` gains a new field, add it to the
`smelt explain --json` field list in `docs/specs/incremental_models.md` §Surface "CLI"
**before** writing the code (spec-first), naming the field and that it mirrors the text
report's downgrade line. Check whether that section enumerates JSON fields; if it does not,
no spec edit is owed.

## Tests

Red-green, in `crates/smelt-cli/tests/explain_maintenance.rs` unless noted:

1. `spark_target_model_explains_state_downgrade` — a keyed-fold-admitting model in a project
   whose only target is `type: spark` produces a report containing a `state downgrade:` line
   naming the ideal technique and the missing structure. Red today (availability is `all()`).
2. `duckdb_target_model_explains_no_state_downgrade` — the same model under a `duckdb` target
   prints no `state downgrade:` line. Guards against the fix over-firing.
3. `explain_json_carries_state_downgrades` — `build_maintenance_plan_json` for the Spark-target
   case emits a non-empty downgrade array (cell group, trigger, resolved technique, ideal
   technique, missing structure, why); the DuckDB case emits an empty one.
4. `explain_graph_path_resolves_real_availability` — the edge-aware resolver
   (`derive_model_maintenance_plan_with_edges` at `maintenance_driver.rs:3465`) reports the
   same downgrade set as the non-edge path for a Spark target, so a model with an inbound
   maintained-model edge does not silently lose its downgrade.
5. `state_availability_for_spark_withholds_ledger_and_frontier` (in
   `crates/smelt-db`'s existing maintenance unit tests) — pins that
   `state_availability_for("spark")` is *not* `all()`, so tests 1/3 fail for the intended
   reason rather than because the resolver is a no-op.

## Tasks

1. Read `crates/smelt-cli/src/commands/explain.rs` and trace exactly which
   `maintenance_driver` resolver produces the `result` that feeds
   `build_maintenance_plan_report` and `build_maintenance_plan_json`; confirm `dialect` (already
   resolved from `config.targets` at `commands/explain.rs:504`) is in scope at that seam.
2. Write tests 1–5 red. Record which fail and why (a test that passes red is a wiring
   assumption that is already true — say so, do not silently keep it).
3. Thread `smelt_db::queries::maintenance::state_availability_for(dialect.name())` into the
   explain-reachable resolvers in place of `StateAvailability::all()` — including
   `maintenance_driver.rs:3465`/`:3484` — following the pattern the two already-real call
   sites (`:1574`, `:2123`) use.
4. Add a `state_downgrades` field to `build_maintenance_plan_json`'s output, mirroring the
   text report's fields; make the spec edit named above first if that section enumerates JSON
   fields.
5. Audit the remaining `all()` sites (`maintenance_driver.rs:570`, `:796`, `:1030`, `:1156`,
   `propagation.rs:812`). Thread real availability where the caller genuinely has a target;
   where it does not, leave `all()` with a one-line comment naming why, and note the residue in
   the phase summary so row 10's sweep can write an accurate Known Divergences bullet.
6. Re-check the example-workspace zero-diagnostics gates — phase 5 already excluded the
   downgrade advisory for `spark`-targeted examples, but this phase makes more paths emit it.

## Verification

- `bash .claude/scripts/verify-phase.sh` (full).
- `cargo test -p smelt-cli --test explain_maintenance --test explain_model --test explain.rs`
  (adjust to the real target names; all explain tests must pass).
- `cargo test -p smelt-cli --test example_diagnostics` and
  `cargo test -p smelt-lsp --test example_workspaces` — the pair phase 5 broke together.
- `cargo test -p smelt-db --test integration` (the availability resolver's own unit coverage).
- Manual: run `smelt explain <model>` in a Spark-targeted fixture and paste the downgrade
  line into the phase summary — the criterion says *visible*, so show it.

## Commit message

`feat(explain): resolve real state availability so downgrades are visible per target`
