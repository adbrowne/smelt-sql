//! Renders a [`crate::recipe::ModelRecipe`] into SQL/YAML text and a staged
//! project (`docs/plans/20260712-generative-maintenance-conformance.md` Phase
//! 1). "Renders once, serves three" (design §4): [`render_model_body`] is the
//! single function that produces the model's `SELECT`, consumed both by
//! [`render_model_file`] (wrapped in frontmatter, written to `models/*.sql`)
//! and by [`render_oracle_sql`] (source refs swapped for physical table
//! names) — the model SQL and the oracle SQL are, by construction, the exact
//! same text apart from that one substitution, which is the equivalence
//! invariant's own statement (`incremental_models.md` §"The equivalence
//! invariant": same SQL body, full inputs).
//!
//! No execution-path code lives here — Phase 1's scope stops at "stages
//! cleanly" (a diagnostics self-check only); driving `execute_project` over a
//! staged recipe is Phase 3 scope (`verdict.rs`).

#![allow(dead_code)]

use std::path::Path;

use crate::recipe::{
    BodyConstruct, ComposedKeyedRecipe, ComposedRoute, ConformanceTarget, KeyedRecipe, ModelEdit,
    ModelRecipe, SourcePosture,
};

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

/// The model's `SELECT` body with `edit` applied (Phase 9;
/// `crate::recipe::ModelEdit`) — the `RewriteModel` schedule step's
/// rendering, kept alongside [`render_model_body`] so both share the same
/// "renders once, serves three" discipline for the edited body (model file
/// AND its oracle, [`render_oracle_sql_with_edit`]). Only the `AdditiveAgg`/
/// `IdempotentAgg`/`DecomposedAgg`/`HolisticAgg`/`PassThrough`/`Filter`
/// combinations `ModelEdit::applicable_evolutions` (see `recipe.rs`)
/// actually names are handled; any other combination is a generator/caller
/// bug, panicking loudly rather than silently rendering something bogus.
pub fn render_model_body_with_edit(recipe: &ModelRecipe, edit: ModelEdit) -> String {
    let src = format!("smelt.sources.{}", recipe.source.name);
    let d = &recipe.source.clock_column;
    let id = &recipe.source.key_column;
    let val = &recipe.source.payload_column;
    match (recipe.construct, edit) {
        (BodyConstruct::PassThrough, ModelEdit::AddPayloadColumn) => {
            format!("SELECT {d}, {id}, {val}, {val} * 2 AS val_doubled FROM {src}")
        }
        (BodyConstruct::Filter { threshold }, ModelEdit::AddPayloadColumn) => {
            format!(
                "SELECT {d}, {id}, {val}, {val} * 2 AS val_doubled FROM {src} \
                 WHERE {val} > {threshold}"
            )
        }
        (BodyConstruct::AdditiveAgg, ModelEdit::AddPayloadColumn) => {
            format!(
                "SELECT {d}, SUM({val}) AS total, COUNT(*) AS row_count FROM {src} GROUP BY {d}"
            )
        }
        (BodyConstruct::AdditiveAgg, ModelEdit::AddGroupingColumn) => {
            format!("SELECT {d}, {id}, SUM({val}) AS total FROM {src} GROUP BY {d}, {id}")
        }
        (BodyConstruct::IdempotentAgg, ModelEdit::AddPayloadColumn) => {
            format!(
                "SELECT {d}, MAX({val}) AS max_val, COUNT(*) AS row_count FROM {src} \
                 GROUP BY {d}"
            )
        }
        (BodyConstruct::IdempotentAgg, ModelEdit::AddGroupingColumn) => {
            format!("SELECT {d}, {id}, MAX({val}) AS max_val FROM {src} GROUP BY {d}, {id}")
        }
        (BodyConstruct::DecomposedAgg, ModelEdit::AddPayloadColumn) => {
            format!(
                "SELECT {d}, AVG({val}) AS avg_val, COUNT(*) AS row_count FROM {src} \
                 GROUP BY {d}"
            )
        }
        (BodyConstruct::DecomposedAgg, ModelEdit::AddGroupingColumn) => {
            format!("SELECT {d}, {id}, AVG({val}) AS avg_val FROM {src} GROUP BY {d}, {id}")
        }
        (BodyConstruct::HolisticAgg, ModelEdit::AddPayloadColumn) => {
            format!(
                "SELECT {d}, MEDIAN({val}) AS med_val, COUNT(DISTINCT {id}) AS distinct_ids, \
                 COUNT(*) AS row_count FROM {src} GROUP BY {d}"
            )
        }
        (BodyConstruct::HolisticAgg, ModelEdit::AddGroupingColumn) => {
            format!(
                "SELECT {d}, {id}, MEDIAN({val}) AS med_val, COUNT(DISTINCT {id}) AS \
                 distinct_ids FROM {src} GROUP BY {d}, {id}"
            )
        }
        (BodyConstruct::PassThrough, ModelEdit::AddGroupingColumn)
        | (BodyConstruct::Filter { .. }, ModelEdit::AddGroupingColumn) => {
            panic!(
                "ModelEdit::AddGroupingColumn is not applicable to {:?} — row-shaped \
                 constructs already project every source column and have no GROUP BY \
                 skeleton to widen (recipe.rs's applicable_evolutions must never offer this \
                 combination)",
                recipe.construct
            )
        }
    }
}

/// The declared `batched.unique_key` after applying `edit` (Phase 9): unchanged
/// for [`ModelEdit::AddPayloadColumn`] (same skeleton); the source's row-key
/// column appended for [`ModelEdit::AddGroupingColumn`] (the widened `GROUP
/// BY`'s own new column).
pub fn rewritten_unique_key(recipe: &ModelRecipe, edit: ModelEdit) -> Vec<String> {
    match edit {
        ModelEdit::AddPayloadColumn => recipe.grain.unique_key.clone(),
        ModelEdit::AddGroupingColumn => {
            let mut key = recipe.grain.unique_key.clone();
            key.push(recipe.source.key_column.clone());
            key
        }
    }
}

/// The full rewritten model file contents (Phase 9): same frontmatter shape
/// as [`render_model_file`], with [`render_model_body_with_edit`]'s body.
/// The retired `batched:` sub-block this used to carry
/// [`rewritten_unique_key`]'s declared key under is gone — it never fed
/// row-identity derivation for a `Grain::Partition` output anyway
/// (`derive::ModelInputs::declared_unique_key` is empty for every
/// `Grain::Partition`), so dropping it changes no derived maintenance plan.
/// [`rewritten_unique_key`] is kept for callers that still want the
/// evolved key shape as data (e.g. schema-evolution comparisons) without it
/// being re-embedded in frontmatter.
pub fn render_model_file_with_edit(recipe: &ModelRecipe, edit: ModelEdit) -> String {
    format!(
        "---\ntimeseries:\n  event_time_column: {etc}\n  partition_column: {pc}\n  granularity: {gran}\nrefresh: incremental\ngrain: partition\n---\n{body}\n",
        etc = recipe.grain.event_time_column,
        pc = recipe.grain.partition_column,
        gran = recipe.grain.granularity,
        body = render_model_body_with_edit(recipe, edit),
    )
}

/// The rewritten oracle query (Phase 9) — [`render_model_body_with_edit`]
/// with `smelt.sources.<x>` replaced by its physical table name, mirroring
/// [`render_oracle_sql`]'s substitution for the un-rewritten body.
pub fn render_oracle_sql_with_edit(recipe: &ModelRecipe, edit: ModelEdit) -> String {
    render_model_body_with_edit(recipe, edit).replace(
        &format!("smelt.sources.{}", recipe.source.name),
        &format!("main.sources_{}", recipe.source.name),
    )
}

/// The full model file contents: frontmatter (`timeseries:` + `refresh:
/// incremental` + `grain: partition`) followed by [`render_model_body`].
/// Follows `model_shapes.rs`'s documented convention: no `WHERE start/end`,
/// `smelt.sources.*` refs. The retired `batched.unique_key` sub-block this
/// used to carry `recipe.grain.unique_key` under is gone — it never fed
/// row-identity derivation for a `Grain::Partition` output anyway
/// (`derive::ModelInputs::declared_unique_key` is empty for every
/// `Grain::Partition`), so dropping it changes no derived maintenance plan.
pub fn render_model_file(recipe: &ModelRecipe) -> String {
    format!(
        "---\ntimeseries:\n  event_time_column: {etc}\n  partition_column: {pc}\n  granularity: {gran}\nrefresh: incremental\ngrain: partition\n---\n{body}\n",
        etc = recipe.grain.event_time_column,
        pc = recipe.grain.partition_column,
        gran = recipe.grain.granularity,
        body = render_model_body(recipe),
    )
}

/// The oracle query (`incremental_models.md` §"The equivalence invariant";
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
/// crate). Equivalent to `render_smelt_yml_for(ConformanceTarget::DuckDb,
/// db_path)` — kept as its own function since it is the only shape every
/// existing caller in this crate and its dependents renders (DuckDB is the
/// only target ever constructed before
/// `docs/plans/20260720-prod-w9-spark-conformance-twin.md`'s later phases).
pub fn render_smelt_yml(db_path: &Path) -> String {
    render_smelt_yml_for(ConformanceTarget::DuckDb, db_path)
}

/// The `(target_name, target_name: ...)` pair keyed under `targets:` for
/// `target`. The Spark block shape mirrors
/// `crates/smelt-cli/tests/common/mod.rs::targets_yaml`'s own Spark arm
/// (`connect_url`/`catalog`/`schema`/`warehouse`/`format: delta`); a
/// `warehouse` directory sibling to `db_path` is used since no caller
/// constructs the Spark arm yet (`ConformanceTarget::SparkDelta` is not
/// reachable through `LinkCProject::run_with_target` until a later phase
/// wires up the Spark backend factory).
fn render_target_block(target: ConformanceTarget, db_path: &Path) -> (&'static str, String) {
    match target {
        ConformanceTarget::DuckDb => (
            "dev",
            format!(
                "type: duckdb\n    database: {db}\n    schema: main",
                db = db_path.display()
            ),
        ),
        ConformanceTarget::SparkDelta => {
            let warehouse = db_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("spark_warehouse");
            (
                "spark",
                format!(
                    "type: spark\n    connect_url: sc://localhost:15002\n    \
                     catalog: spark_catalog\n    schema: smelt_conformance\n    \
                     warehouse: {warehouse}\n    format: delta",
                    warehouse = warehouse.display(),
                ),
            )
        }
    }
}

/// A minimal `smelt.yml` targeting `target`: one target block
/// ([`render_target_block`]) under whichever name that target uses, one
/// `models` scan root, `table` materialization
/// (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 2's
/// `ConformanceTarget`-parametrized `render_smelt_yml`).
pub fn render_smelt_yml_for(target: ConformanceTarget, db_path: &Path) -> String {
    let (name, block) = render_target_block(target, db_path);
    format!(
        "name: generative_conformance\nversion: 1\npaths:\n  - models\ntargets:\n  {name}:\n    {block}\ndefault_materialization: table\n",
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

    create_source_table_via_backend(
        db_path,
        &recipe.source.name,
        &format!(
            "{d} DATE, {id} INTEGER, {val} INTEGER",
            d = recipe.source.clock_column,
            id = recipe.source.key_column,
            val = recipe.source.payload_column,
        ),
    )?;

    crate::link_c_harness::LinkCProject::load(project_dir.to_path_buf(), db_path.to_path_buf())
}

/// Create `main.sources_<name>` (columns `column_defs`) at `db_path` through
/// [`smelt_backend::Backend::execute_sql`] rather than a raw
/// `duckdb::Connection`
/// (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 2) — the
/// single staging-DDL routine [`stage`]/[`stage_keyed`]/[`stage_composed`]
/// all funnel through, so every real caller of those three (this crate's
/// `schedule_gen.rs`/`probes.rs`/`verdict.rs`, and `gate.rs`) gets the
/// Backend-routed execution channel for free with no signature change.
/// `DuckDbBackend::new` already issues `CREATE SCHEMA IF NOT EXISTS main`
/// itself, so this only needs the `CREATE TABLE`. Run on an isolated thread
/// (`link_c_harness::block_on_isolated`) since callers include both plain
/// `#[test]` functions and `#[tokio::test]` bodies already driving a runtime.
fn create_source_table_via_backend(
    db_path: &Path,
    source_name: &str,
    column_defs: &str,
) -> anyhow::Result<()> {
    let db_path = db_path.to_path_buf();
    let ddl = format!("CREATE TABLE main.sources_{source_name} ({column_defs})");
    crate::link_c_harness::block_on_isolated(async move {
        let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
            .await
            .map_err(|e| anyhow::anyhow!("open backend for staging DDL: {e}"))?;
        smelt_backend::Backend::execute_sql(&backend, &ddl)
            .await
            .map_err(|e| anyhow::anyhow!("create source table {ddl:?}: {e}"))?;
        Ok::<(), anyhow::Error>(())
    })
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

/// The `grain: key` model's `SELECT` body (Phase 5;
/// [`crate::recipe::KeyedRecipe`]): `SELECT <key>, <agg>(<val>) AS <alias>
/// FROM smelt.sources.<name> GROUP BY <key>`. Kept separate from
/// [`render_model_body`]'s exhaustive [`BodyConstruct`] match — `KeyedRecipe`
/// is not a `BodyConstruct` (plan Phase 5 "Implementation shape": "Keyed
/// rendering per `model_shapes.rs`'s keyed conventions").
pub fn render_keyed_model_body(recipe: &KeyedRecipe) -> String {
    let src = format!("smelt.sources.{}", recipe.source.name);
    let key = &recipe.source.key_column;
    let val = &recipe.source.payload_column;
    let (agg, alias) = recipe.combiner.agg_and_alias();
    format!("SELECT {key}, {agg}({val}) AS {alias} FROM {src} GROUP BY {key}")
}

/// The full `grain: key` model file: `refresh: incremental` + `grain: key`
/// frontmatter — deliberately no `timeseries:` block (`incremental_models.md`
/// §Known Divergences "The key grain": "every `timeseries:` block on a keyed model is refused
/// unconditionally") and no `batched.unique_key` (keyed output has no
/// partition column) — followed by [`render_keyed_model_body`].
pub fn render_keyed_model_file(recipe: &KeyedRecipe) -> String {
    format!(
        "---\nrefresh: incremental\ngrain: key\n---\n{body}\n",
        body = render_keyed_model_body(recipe),
    )
}

/// The keyed model's oracle query evaluated over `source_table_ref` instead
/// of `smelt.sources.<name>` — the same body [`render_keyed_model_body`]
/// renders once, serving both the model file and the oracle (design §4).
/// Callers pass either the physical source table (a full-refresh oracle) or
/// an `STracker`-materialized `oracle_<name>` temp table (the S-restricted
/// / end-state oracle, `docs/plans/20260712-generative-maintenance-conformance.md`
/// Phase 5).
pub fn render_keyed_oracle_body_over(recipe: &KeyedRecipe, source_table_ref: &str) -> String {
    render_keyed_model_body(recipe).replace(
        &format!("smelt.sources.{}", recipe.source.name),
        source_table_ref,
    )
}

/// Stage a [`KeyedRecipe`] into a fresh project dir + DuckDB file: writes the
/// model file, the driving source's YAML
/// ([`crate::recipe::SourceRecipe::source_yaml`], clocked or unclocked per
/// its posture), `smelt.yml`, and creates the physical source table matching
/// the source's column shape — the keyed-pool counterpart of [`stage`].
pub fn stage_keyed(
    recipe: &KeyedRecipe,
    project_dir: &Path,
    db_path: &Path,
) -> anyhow::Result<crate::link_c_harness::LinkCProject> {
    std::fs::create_dir_all(project_dir.join("models/sources"))?;
    std::fs::write(
        project_dir.join(format!("models/{}.sql", recipe.model_name)),
        render_keyed_model_file(recipe),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.source.name)),
        recipe.source.source_yaml(),
    )?;
    std::fs::write(project_dir.join("smelt.yml"), render_smelt_yml(db_path))?;

    let column_defs = match recipe.source.posture {
        SourcePosture::AppendOnly => format!(
            "{d} DATE, {id} INTEGER, {val} INTEGER",
            d = recipe.source.clock_column,
            id = recipe.source.key_column,
            val = recipe.source.payload_column,
        ),
        SourcePosture::MutableSnapshot => format!(
            "{id} INTEGER, {val} INTEGER",
            id = recipe.source.key_column,
            val = recipe.source.payload_column,
        ),
    };
    create_source_table_via_backend(db_path, &recipe.source.name, &column_defs)?;

    crate::link_c_harness::LinkCProject::load(project_dir.to_path_buf(), db_path.to_path_buf())
}

/// Every `DiagnosticCode` variant in the `Maintenance*` family
/// (`incremental_models.md` §Diagnostics) — permitted on a staged recipe's
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

// ---------------------------------------------------------------------
// Phase A6: the composed (`grain: key` + `timeseries:`) recipe family
// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase A6).
// ---------------------------------------------------------------------

/// [`ComposedKeyedRecipe`]'s model `SELECT` body — see that type's own doc
/// comment for the per-route shape and why `KeyDetermined`'s body is not
/// itself valid, executable DuckDB SQL (it exists only to exercise the
/// real key-temporal-locality gate's admission over real staged
/// frontmatter/YAML; the gate's own executable-mechanics coverage drives a
/// separately hand-written per-step query, `gate.rs`'s own
/// `compile_step`-shaped helpers).
pub fn render_composed_model_body(recipe: &ComposedKeyedRecipe) -> String {
    let src = format!("smelt.sources.{}", recipe.source.name);
    let d = &recipe.source.clock_column;
    let id = &recipe.source.key_column;
    let val = &recipe.source.payload_column;
    match recipe.route {
        ComposedRoute::KeyEmbedded => {
            format!("SELECT {id}, {d}, SUM({val}) AS total FROM {src} GROUP BY {id}, {d}")
        }
        ComposedRoute::KeyDetermined => {
            format!(
                "SELECT {id}, CAST({d} AS DATE) AS pdate, SUM({val}) AS total FROM {src} \
                 GROUP BY {id}"
            )
        }
        ComposedRoute::RecurrenceBounded => {
            format!("SELECT {id}, MAX({d}) AS last_seen FROM {src} GROUP BY {id}")
        }
    }
}

/// The full composed model file: frontmatter (`timeseries:` +
/// `refresh: incremental` + `grain: key`, plus a `functional_dependencies:`
/// entry for [`ComposedRoute::KeyDetermined`]) followed by
/// [`render_composed_model_body`]. Deliberately no `batched.unique_key` —
/// like [`render_keyed_model_file`], a keyed output's `unique_key` is
/// derived from its own `GROUP BY`, never declared.
pub fn render_composed_model_file(recipe: &ComposedKeyedRecipe) -> String {
    let partition_column = recipe.partition_column();
    let fd_block = match recipe.functional_dependency() {
        Some((key, determines)) => format!(
            "functional_dependencies:\n  - key: [{}]\n    determines: {}\n",
            key.join(", "),
            determines,
        ),
        None => String::new(),
    };
    format!(
        "---\ntimeseries:\n  event_time_column: {pc}\n  partition_column: {pc}\n  granularity: day\nrefresh: incremental\ngrain: key\n{fd_block}---\n{body}\n",
        pc = partition_column,
        body = render_composed_model_body(recipe),
    )
}

/// The oracle query for [`ComposedRoute::KeyEmbedded`] (the only route this
/// text-substitution oracle is valid for — see [`render_composed_model_body`]'s
/// doc comment for why routes 2/3 need their own hand-written oracle
/// queries, `gate.rs`'s own `composed_route2_oracle_sql`): the model body
/// with `smelt.sources.<x>` replaced by its physical table name.
pub fn render_composed_oracle_sql(recipe: &ComposedKeyedRecipe) -> String {
    render_composed_model_body(recipe).replace(
        &format!("smelt.sources.{}", recipe.source.name),
        &format!("main.sources_{}", recipe.source.name),
    )
}

/// Stage a [`ComposedKeyedRecipe`] into a fresh project dir + DuckDB file:
/// writes the model file, the driving source's YAML
/// ([`crate::recipe::SourceRecipe::source_yaml`], carrying `key_recurrence`
/// for [`ComposedRoute::RecurrenceBounded`]), `smelt.yml`, and creates the
/// physical source table — the composed-pool counterpart of [`stage`]/
/// [`stage_keyed`]. The returned [`crate::link_c_harness::LinkCProject`]'s
/// `db_path` is reused by `gate.rs` to open a direct `DuckDbBackend` for
/// routes 2/3's own execution path (see [`ComposedKeyedRecipe`]'s doc
/// comment).
pub fn stage_composed(
    recipe: &ComposedKeyedRecipe,
    project_dir: &Path,
    db_path: &Path,
) -> anyhow::Result<crate::link_c_harness::LinkCProject> {
    std::fs::create_dir_all(project_dir.join("models/sources"))?;
    std::fs::write(
        project_dir.join(format!("models/{}.sql", recipe.model_name)),
        render_composed_model_file(recipe),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.source.name)),
        recipe.source.source_yaml(),
    )?;
    std::fs::write(project_dir.join("smelt.yml"), render_smelt_yml(db_path))?;

    create_source_table_via_backend(
        db_path,
        &recipe.source.name,
        &format!(
            "{d} DATE, {id} INTEGER, {val} INTEGER",
            d = recipe.source.clock_column,
            id = recipe.source.key_column,
            val = recipe.source.payload_column,
        ),
    )?;

    crate::link_c_harness::LinkCProject::load(project_dir.to_path_buf(), db_path.to_path_buf())
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
