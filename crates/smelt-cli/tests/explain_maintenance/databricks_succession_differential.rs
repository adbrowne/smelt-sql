//! The offline root-cause differential for row 9c
//! (`docs/outcomes/20260912-databricks-dogfood-spine/phases/09c-plan.md`):
//! does `silver.actor_naming`'s succession cell get downgraded to a
//! different — fold-blind — technique on Databricks than on `dev`, with no
//! workspace and no connection? `smelt explain <model>` derives this exact
//! way internally (`crates/smelt-cli/src/commands/explain.rs`): the plan is
//! dialect-agnostic (`smelt_db::maintenance_plan_report`), then
//! `smelt_logical::maintenance::availability::resolve_availability` is
//! applied once per target dialect before any cell is reported.
//!
//! **Measured finding (task 3 of the plan).** `silver.actor_naming` is the
//! keyed-succession grain's own undeclared-admission shape (no `grain:`
//! written). Its one cell's ideal technique is `Technique::SuccessionPatch`,
//! which requires `StateStructure::TombstoneLedger`
//! (`state_structure::required_state_structure`).
//! `state_structure::realisable_state_structures(SqlDialect::SparkSQL)` is
//! empty — Databricks maps to `SparkSQL` (`backend_type_to_sql_dialect`) —
//! so `resolve_availability` downgrades the cell to `Technique::DeleteInsert`
//! on Databricks while it stays `Technique::SuccessionPatch` on `dev`
//! (DuckDB, which realises `TombstoneLedger`). **The technique differs
//! between the two targets** — the opposite of the plan's cheapest-first
//! hypothesis. Before this phase, `resolve_live_succession_cell`
//! (`crates/smelt-runtime/src/maintenance_driver/succession/mod.rs`) treated
//! a downgraded cell as "not live" and fell through to the generic
//! `DeleteInsert` driver, which has no `(key, clock)` fold at all — exactly
//! reproducing row 8's "no fold at all" symptom (Databricks' row count
//! equals the raw source count). The fix (this phase) keeps a downgraded
//! succession cell live for the succession driver and forces it onto the
//! full-rebuild route every run, which folds via
//! `emit_succession_full_rebuild`'s `ROW_NUMBER() ... = 1` exactly as a full
//! refresh does — see `crates/smelt-runtime/src/maintenance_driver/
//! succession/tests.rs`'s `state_downgraded_cell_still_dispatches_marked_for_full_rebuild`
//! for the resolver-level proof. This test is the plan-level standing gate:
//! it pins the technique difference between the two dialects so a future
//! change to either dialect's realisable-structures table is forced to
//! re-examine this model.

use smelt_backend::SqlDialect;
use smelt_logical::maintenance::availability::resolve_availability;
use smelt_logical::maintenance::Technique;
use smelt_runtime::maintenance_availability::availability_for_run;

use crate::support::plan_result_for;

/// `silver.actor_naming`'s succession cell resolves to `Technique::
/// SuccessionPatch` on `dev` (DuckDB) and downgrades to `Technique::
/// DeleteInsert` on the Databricks dialect (`SqlDialect::SparkSQL`) — the
/// root cause of row 8/9b's Databricks-only duplication. Encodes the
/// decisive offline differential the plan's hypothesis called for.
#[test]
fn silver_actor_naming_succession_cell_downgrades_on_databricks_but_not_dev() {
    // `smelt.yml`'s `databricks`/`databricks_oracle` targets reference
    // `${SMELT_DBX_HOSTNAME}`/`${SMELT_DBX_TOKEN}` — `Config::load` refuses
    // an unresolved reference fail-loud even though this test never
    // connects (it only reads `type: databricks` to pick a dialect), so
    // stub both with a value that is never dereferenced.
    // SAFETY: test-only process env mutation, read back only by this same
    // process's `Config::load` calls, never passed to any live connection.
    unsafe {
        std::env::set_var("SMELT_DBX_HOSTNAME", "unused.invalid");
        std::env::set_var("SMELT_DBX_TOKEN", "unused");
    }

    let mut dev_result =
        plan_result_for("silver.actor_naming").expect("silver.actor_naming has a maintenance plan");
    let mut dbx_result =
        plan_result_for("silver.actor_naming").expect("silver.actor_naming has a maintenance plan");

    let dev_availability = availability_for_run(SqlDialect::DuckDB, &config_for_example());
    let dbx_availability = availability_for_run(SqlDialect::SparkSQL, &config_for_example());
    resolve_availability(&mut dev_result.plan.cells, &dev_availability);
    resolve_availability(&mut dbx_result.plan.cells, &dbx_availability);

    assert_eq!(
        dev_result.plan.cells.len(),
        1,
        "expected exactly one cell for the succession grain's undeclared-admission shape"
    );
    let dev_cell = &dev_result.plan.cells[0];
    let dbx_cell = &dbx_result.plan.cells[0];

    assert_eq!(
        dev_cell.technique,
        Technique::SuccessionPatch,
        "on `dev` (DuckDB, which realises TombstoneLedger) the ideal technique must stand"
    );
    assert!(
        dev_cell.state_downgrade.is_none(),
        "the dev-target cell must carry no state downgrade"
    );

    assert_eq!(
        dbx_cell.technique,
        Technique::DeleteInsert,
        "on Databricks (SparkSQL, which realises no state structures at all) the cell must \
         downgrade — this is the root cause row 8/9b measured"
    );
    let downgrade = dbx_cell
        .state_downgrade
        .as_ref()
        .expect("the Databricks-target cell must record why it downgraded");
    assert_eq!(
        downgrade.original,
        Technique::SuccessionPatch,
        "the downgrade must name SuccessionPatch as the technique it replaced"
    );
}

fn config_for_example() -> smelt_core::config::Config {
    let path = crate::support::example_dir("github_activity").join("smelt.yml");
    serde_yaml::from_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}")),
    )
    .unwrap_or_else(|e| panic!("parse {path:?}: {e}"))
}
