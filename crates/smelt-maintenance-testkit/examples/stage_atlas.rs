//! Stage the testkit's recipe shapes as on-disk smelt projects so external
//! tooling (e.g. the Maintenance Atlas visualisation) can run `smelt explain`
//! over the full technique family — `ColumnScopedMerge`, the `KeyedFold`
//! ledger grades, and the snapshot-reconcile plain-overwrite executor — none
//! of which the shipped `examples/` workspaces currently reach.
//!
//! Usage: `cargo run -p smelt-maintenance-testkit --example stage_atlas -- <out_dir>`
//! Each recipe lands at `<out_dir>/<name>/{project,db.duckdb}`.

use std::path::Path;

use smelt_maintenance_testkit::recipe::{
    KeyedCombiner, KeyedRecipe, MutableEnrichedRecipe, ValueEnrichedRecipe,
};
use smelt_maintenance_testkit::render;

fn main() -> anyhow::Result<()> {
    let out = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: stage_atlas <out_dir>"))?;
    let out = Path::new(&out);

    stage_value_enriched(&out.join("value_enriched"))?;
    stage_mutable_enriched(&out.join("mutable_enriched"))?;
    for (name, recipe) in [
        (
            "keyed_additive",
            KeyedRecipe::new_window_forward(KeyedCombiner::Additive),
        ),
        (
            "keyed_order_monotone",
            KeyedRecipe::new_window_forward(KeyedCombiner::OrderMonotone),
        ),
        (
            "keyed_snapshot_overwrite",
            KeyedRecipe::new_snapshot_reconcile(KeyedCombiner::PlainOverwrite),
        ),
    ] {
        let dir = out.join(name);
        let project_dir = dir.join("project");
        std::fs::create_dir_all(&project_dir)?;
        render::stage_keyed(&recipe, &project_dir, &dir.join("db.duckdb"))?;
        println!("staged {name} → {}", project_dir.display());
    }
    Ok(())
}

/// Mirrors `stage_value_enriched_recipe` in
/// `crates/smelt-cli/tests/maintenance_conformance/gate.rs` — the
/// closure-pruned LEFT JOIN enrichment whose dimension cell admits
/// `Technique::ColumnScopedMerge`.
fn stage_value_enriched(dir: &Path) -> anyhow::Result<()> {
    let recipe = ValueEnrichedRecipe::new();
    let project_dir = dir.join("project");
    let db_path = dir.join("db.duckdb");
    std::fs::create_dir_all(project_dir.join("models/sources"))?;
    std::fs::write(
        project_dir.join(format!("models/{}.sql", recipe.model_name)),
        recipe.model_file(),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.fact.name)),
        recipe.fact_source_yaml(),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.dimension.name)),
        recipe.dimension_source_yaml(),
    )?;
    let smelt_yml = format!(
        "{base}models:\n  {model}:\n    batched:\n      unique_key: [{id}]\n",
        base = render::render_smelt_yml(&db_path),
        model = recipe.model_name,
        id = recipe.fact.key_column,
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml)?;

    let conn = duckdb::Connection::open(&db_path)?;
    conn.execute_batch(&format!(
        "CREATE SCHEMA IF NOT EXISTS main; \
         CREATE TABLE main.sources_{fact} ({d} DATE, {id} INTEGER, {val} INTEGER); \
         CREATE TABLE main.sources_{dim} ({dim_id} INTEGER, {attr} INTEGER);",
        fact = recipe.fact.name,
        d = recipe.fact.clock_column,
        id = recipe.fact.key_column,
        val = recipe.fact.payload_column,
        dim = recipe.dimension.name,
        dim_id = recipe.dimension.key_column,
        attr = recipe.dimension.payload_column,
    ))?;
    println!("staged value_enriched → {}", project_dir.display());
    Ok(())
}

/// Mirrors `stage_mixed_recipe` in the conformance gate — the INNER JOIN
/// enrichment against a mutable dimension whose open closure makes every
/// cell membership-sensitive (`Technique::DeleteInsert`), the contrast case
/// to `stage_value_enriched`.
fn stage_mutable_enriched(dir: &Path) -> anyhow::Result<()> {
    let recipe = MutableEnrichedRecipe::new();
    let project_dir = dir.join("project");
    let db_path = dir.join("db.duckdb");
    std::fs::create_dir_all(project_dir.join("models/sources"))?;
    std::fs::write(
        project_dir.join(format!("models/{}.sql", recipe.model_name)),
        recipe.model_file(),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.fact.name)),
        recipe.fact_source_yaml(),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.dimension.name)),
        recipe.dimension_source_yaml(),
    )?;
    std::fs::write(
        project_dir.join("smelt.yml"),
        render::render_smelt_yml(&db_path),
    )?;

    let conn = duckdb::Connection::open(&db_path)?;
    conn.execute_batch(&format!(
        "CREATE SCHEMA IF NOT EXISTS main; \
         CREATE TABLE main.sources_{fact} ({d} DATE, {id} INTEGER, {val} INTEGER); \
         CREATE TABLE main.sources_{dim} ({dim_id} INTEGER, {attr} INTEGER);",
        fact = recipe.fact.name,
        d = recipe.fact.clock_column,
        id = recipe.fact.key_column,
        val = recipe.fact.payload_column,
        dim = recipe.dimension.name,
        dim_id = recipe.dimension.key_column,
        attr = recipe.dimension.payload_column,
    ))?;
    println!("staged mutable_enriched → {}", project_dir.display());
    Ok(())
}
