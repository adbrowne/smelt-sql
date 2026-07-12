//! Renders a [`crate::recipe::ModelRecipe`] into SQL/YAML text and a staged
//! project (`docs/plans/20260712-generative-maintenance-conformance.md` Phase
//! 1). "Renders once, serves three" (design §4): [`render_model_body`] is the
//! single function that produces the model's `SELECT`, consumed both by
//! [`render_model_file`] (wrapped in frontmatter, written to `models/*.sql`)
//! and by [`render_oracle_sql`] (source refs swapped for physical table
//! names) — the model SQL and the oracle SQL are, by construction, the exact
//! same text apart from that one substitution, which is the equivalence
//! invariant's own statement (`maintenance_plan.md` §"The equivalence
//! invariant": same SQL body, full inputs).
//!
//! No execution-path code lives here — Phase 1's scope stops at "stages
//! cleanly" (a diagnostics self-check only); driving `execute_project` over a
//! staged recipe is Phase 3 scope (`verdict.rs`).

#![allow(dead_code)]

use std::path::Path;

use crate::recipe::{BodyConstruct, ModelRecipe};

/// The model's `SELECT` body — no frontmatter, no `WHERE start/end` (`smelt`
/// derives the incremental filter; `model_shapes.rs`'s documented
/// convention). Shared verbatim by [`render_model_file`] and
/// [`render_oracle_sql`] (design §4 "renders once, serves three").
pub fn render_model_body(recipe: &ModelRecipe) -> String {
    let src = format!("smelt.sources.{}", recipe.source.name);
    let d = &recipe.source.clock_column;
    let id = &recipe.source.key_column;
    let val = &recipe.source.payload_column;
    match recipe.construct {
        BodyConstruct::PassThrough => {
            format!("SELECT {d}, {id}, {val} FROM {src}")
        }
        BodyConstruct::Filter { threshold } => {
            format!("SELECT {d}, {id}, {val} FROM {src} WHERE {val} > {threshold}")
        }
        BodyConstruct::AdditiveAgg => {
            format!("SELECT {d}, SUM({val}) AS total FROM {src} GROUP BY {d}")
        }
        BodyConstruct::IdempotentAgg => {
            format!("SELECT {d}, MAX({val}) AS max_val FROM {src} GROUP BY {d}")
        }
        BodyConstruct::DecomposedAgg => {
            format!("SELECT {d}, AVG({val}) AS avg_val FROM {src} GROUP BY {d}")
        }
        BodyConstruct::HolisticAgg => {
            format!(
                "SELECT {d}, MEDIAN({val}) AS med_val, COUNT(DISTINCT {id}) AS distinct_ids \
                 FROM {src} GROUP BY {d}"
            )
        }
    }
}

/// The full model file contents: frontmatter (`timeseries:` + `refresh:
/// incremental` + `grain: partition` + `batched.unique_key`) followed by
/// [`render_model_body`]. Follows `model_shapes.rs`'s documented convention:
/// no `WHERE start/end`, `smelt.sources.*` refs.
pub fn render_model_file(recipe: &ModelRecipe) -> String {
    let unique_key = recipe.grain.unique_key.join(", ");
    format!(
        "---\ntimeseries:\n  event_time_column: {etc}\n  partition_column: {pc}\n  granularity: {gran}\nrefresh: incremental\ngrain: partition\nbatched:\n  unique_key: [{unique_key}]\n---\n{body}\n",
        etc = recipe.grain.event_time_column,
        pc = recipe.grain.partition_column,
        gran = recipe.grain.granularity,
        body = render_model_body(recipe),
    )
}

/// The oracle query (`maintenance_plan.md` §"The equivalence invariant";
/// design §6 "Oracle query"): the model body with `smelt.sources.<x>`
/// replaced by its physical table name (`main.sources_<x>`), evaluated
/// directly on a `duckdb::Connection` — independent of smelt's own
/// compile/execute pipeline.
pub fn render_oracle_sql(recipe: &ModelRecipe) -> String {
    render_model_body(recipe).replace(
        &format!("smelt.sources.{}", recipe.source.name),
        &format!("main.sources_{}", recipe.source.name),
    )
}

/// The source YAML sidecar (`sources.md` §"Source YAML shape"), declaring the
/// clocked, `append_only` `events(d, id, val)` source every Phase 1 recipe
/// stages.
pub fn render_source_yaml(recipe: &ModelRecipe) -> String {
    format!(
        "description: generative-conformance source.\nmutation_profile: append_only\ntimeseries:\n  event_time_column: {etc}\n  partition_column: {pc}\n  granularity: {gran}\ncolumns:\n  - name: {d}\n    type: DATE\n  - name: {id}\n    type: INTEGER\n  - name: {val}\n    type: INTEGER\n",
        etc = recipe.grain.event_time_column,
        pc = recipe.grain.partition_column,
        gran = recipe.grain.granularity,
        d = recipe.source.clock_column,
        id = recipe.source.key_column,
        val = recipe.source.payload_column,
    )
}

/// A minimal `smelt.yml`: one `dev` DuckDB target pointing at `db_path`, one
/// `models` scan root, `table` materialization (matches the
/// `link_c_harness`/`model_shapes` staging convention throughout this
/// crate).
pub fn render_smelt_yml(db_path: &Path) -> String {
    format!(
        "name: generative_conformance\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    )
}

/// The staged project's file contents, keyed by path relative to the project
/// root — the artifact [`stage`] writes to disk.
#[derive(Debug, Clone)]
pub struct StagedFiles {
    pub model_relpath: String,
    pub model_contents: String,
    pub source_relpath: String,
    pub source_contents: String,
    pub smelt_yml_contents: String,
}

/// Render `recipe` into every file a stageable project needs, without
/// touching the filesystem — [`stage`] is the disk-writing wrapper around
/// this.
pub fn render_project(recipe: &ModelRecipe, db_path: &Path) -> StagedFiles {
    StagedFiles {
        model_relpath: format!("models/{}.sql", recipe.model_name),
        model_contents: render_model_file(recipe),
        source_relpath: format!("models/sources/{}.yml", recipe.source.name),
        source_contents: render_source_yaml(recipe),
        smelt_yml_contents: render_smelt_yml(db_path),
    }
}

/// Write `recipe`'s rendered project to `project_dir` and create the empty
/// staged source table in a fresh DuckDB file at `db_path`. Returns the
/// `LinkCProject` fixture ready for `Config::load`-based consumption — no
/// `execute_project` call happens here (Phase 1 scope stops at staging; a
/// run driver is Phase 3 scope).
pub fn stage(
    recipe: &ModelRecipe,
    project_dir: &Path,
    db_path: &Path,
) -> anyhow::Result<crate::link_c_harness::LinkCProject> {
    let staged = render_project(recipe, db_path);

    std::fs::create_dir_all(project_dir.join("models/sources"))?;
    std::fs::write(
        project_dir.join(&staged.model_relpath),
        &staged.model_contents,
    )?;
    std::fs::write(
        project_dir.join(&staged.source_relpath),
        &staged.source_contents,
    )?;
    std::fs::write(project_dir.join("smelt.yml"), &staged.smelt_yml_contents)?;

    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch(&format!(
        "CREATE SCHEMA IF NOT EXISTS main; \
         CREATE TABLE main.sources_{name} ({d} DATE, {id} INTEGER, {val} INTEGER);",
        name = recipe.source.name,
        d = recipe.source.clock_column,
        id = recipe.source.key_column,
        val = recipe.source.payload_column,
    ))?;
    drop(conn);

    crate::link_c_harness::LinkCProject::load(project_dir.to_path_buf(), db_path.to_path_buf())
}

/// Diagnostics self-check (design §4 "Valid-by-construction"): a throwaway
/// `smelt_db::Database`, populated the same way `link_c_harness`'s
/// `build_db_and_graph` does (discover the staged `.sql` models, `set_source_file`
/// each), returning every `file_diagnostics` entry across the staged model
/// file(s). Per-entity source YAMLs are resolved lazily straight off disk by
/// `ProjectInput`-keyed Salsa queries (`architecture.md` §"Resolution"), so
/// they need no explicit `set_source_file` call here.
pub fn staged_diagnostics(project_dir: &Path) -> anyhow::Result<Vec<smelt_db::Diagnostic>> {
    let config = smelt_core::config::Config::load(project_dir)?;
    let discovery =
        smelt_core::ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let sql_models = discovery.discover_models()?;

    let mut db = smelt_db::Database::default();
    let project = db.set_project_input(project_dir.to_path_buf(), String::new());
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf()))
        .collect();
    db.set_workspace(source_files.clone(), vec![project]);

    let workspace = db.workspace();
    Ok(source_files
        .into_iter()
        .flat_map(|file| smelt_db::file_diagnostics(&db, workspace, file))
        .collect())
}

/// Every `DiagnosticCode` variant in the `Maintenance*` family
/// (`maintenance_plan.md` §Diagnostics) — permitted on a staged recipe's
/// diagnostics, since Phase 1 does not yet constrain which techniques a
/// recipe's cell admits. Any other diagnostic on a valid-by-construction
/// recipe is a generator bug.
fn is_maintenance_family(code: Option<smelt_db::DiagnosticCode>) -> bool {
    matches!(
        code,
        Some(
            smelt_db::DiagnosticCode::MaintenanceNoAdmissibleTechnique
                | smelt_db::DiagnosticCode::MaintenanceScanUnbounded
                | smelt_db::DiagnosticCode::MaintenanceGranularityMismatch
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::{arb_recipe, RecipePool};
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::TestRunner;

    /// `rendered_recipe_stages_cleanly` (plan Phase 1 TDD list): every
    /// generated recipe renders to a staged project whose `file_diagnostics`
    /// contain no parse/type/config errors (maintenance-family diagnostics
    /// permitted); a dirty render is a generator bug, failed loudly.
    #[test]
    fn rendered_recipe_stages_cleanly() {
        let mut runner = TestRunner::deterministic();
        let strat = arb_recipe(RecipePool::partition_append_only());

        for i in 0..40 {
            let recipe = strat.new_tree(&mut runner).unwrap().current();
            let tmp = tempfile::TempDir::new().expect("tempdir");
            let project_dir = tmp.path().join("project");
            let db_path = tmp.path().join("db.duckdb");
            std::fs::create_dir_all(&project_dir).expect("create project dir");

            stage(&recipe, &project_dir, &db_path)
                .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} failed to stage: {e}"));

            let diags = staged_diagnostics(&project_dir).unwrap_or_else(|e| {
                panic!("case {i}: recipe {recipe:?} diagnostics query failed: {e}")
            });
            let bad: Vec<_> = diags
                .iter()
                .filter(|d| {
                    d.severity == smelt_db::DiagnosticSeverity::Error
                        && !is_maintenance_family(d.code)
                })
                .collect();
            assert!(
                bad.is_empty(),
                "case {i}: recipe {recipe:?} rendered to model:\n{}\nwith non-maintenance \
                 diagnostics (generator bug): {bad:#?}",
                render_model_file(&recipe),
            );
        }
    }
}
