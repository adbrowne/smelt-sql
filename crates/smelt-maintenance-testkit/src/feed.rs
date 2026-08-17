//! `SimulatedChangeFeed` — the harness-side step family for `change_feed`-
//! declared sources
//! (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 8;
//! design doc `docs/research/20260711-generative-maintenance-conformance.md`
//! §5 "Simulated change feed").
//!
//! Nothing in production consumes a change feed today: deltas are discovered
//! by interval-diffing a clocked source's landings, and a declared
//! `mutation_profile: change_feed` collapses to the stricter
//! `MutableSnapshot` posture for admission purposes only
//! (`incremental_models.md` §Known Divergences — the `change_feed`-scoping
//! entry). This module readies generative coverage for the day a
//! feed-consuming fold lands (offset-based delta detection, retraction-
//! carrying deltas into invertible combiners) without waiting for it: every
//! mutation step against a feed-declared source is applied to the base table
//! *and* appended as an `(op, key, payload, seq)` row to a staged feed table
//! `main.feed_<src>`, atomically per step. v1 asserts only today's honest
//! contract — a `change_feed`-declared source admits full-input
//! re-derivation (`Technique::DeleteInsert`) only, never a fold, and
//! equivalence over a mutation schedule holds via the recompute path.
//!
//! Deliberately NOT routed through [`crate::recipe::SourcePosture`] — that
//! enum's own doc comment already scopes it to "the two testkit-relevant
//! variants (the `ChangeFeed` variant is out of this crate's generated
//! pool)": `SourcePosture` is the *model-shape* pool `arb_recipe` draws
//! typed [`crate::recipe::BodyConstruct`]s from, not this module's harness-
//! only mutation/feed-bookkeeping step family. [`change_feed_source_yaml`]
//! renders the `change_feed` wire spelling directly, bypassing
//! [`crate::recipe::SourceRecipe::source_yaml`] (whose match only ever
//! emits `append_only`/`mutable_snapshot`), so no exhaustive match elsewhere
//! in this crate (notably `render.rs::stage_keyed`) needs to widen.
//!
//! No production code path is touched or exercised by this module beyond
//! the ordinary `execute_project` full-refresh arm every other pool in this
//! crate already drives.

#![allow(dead_code)]

use chrono::NaiveDate;
use duckdb::Connection;
use proptest::prelude::*;

use crate::link_c_harness::LinkCProject;
use crate::recipe::{KeyShape, KeyedCombiner, KeyedRecipe, SourceRecipe};
use crate::render;
use crate::schedule_gen::GenRow;

/// One row of the feed-driven source's `events(d, id, val)` shape — the same
/// row type [`crate::schedule_gen`] uses elsewhere in this crate; kept as an
/// alias rather than a duplicate struct so base-table reads
/// ([`read_source_snapshot`]) and feed-replay reads ([`replay_feed`]) are
/// directly comparable without a conversion step.
pub type FeedRow = GenRow;

/// A source's posture for THIS module's gating check (design §5:
/// "Tombstone/retraction step kinds are part of this family, gated to
/// feed-declared sources"). Narrower than, and deliberately not unified
/// with, [`crate::recipe::SourcePosture`] — see the module doc comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedSourcePosture {
    /// The source declares `mutation_profile: change_feed` — retraction/
    /// tombstone steps are meaningful and may be generated.
    ChangeFeed,
    /// Any other posture — retraction/tombstone steps are refused
    /// (a `change_feed` source is the only shape a delete is ever expressed
    /// as an explicit retraction rather than an ordinary in-place mutation).
    NotFeed,
}

/// The op recorded in the staged feed table alongside each mutation
/// (`(op, key, payload, seq)`, design §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedOp {
    Insert,
    Update,
    Delete,
    /// Tombstone/retraction — [`build_feed_step`] refuses to construct this
    /// op for a [`FeedSourcePosture::NotFeed`] source (design §5).
    Retract,
}

impl FeedOp {
    /// The `op` column's wire text, written into the staged feed table and
    /// read back by [`replay_feed`].
    fn wire_name(self) -> &'static str {
        match self {
            FeedOp::Insert => "insert",
            FeedOp::Update => "update",
            FeedOp::Delete => "delete",
            FeedOp::Retract => "retract",
        }
    }
}

/// One step of a [`SimulatedChangeFeed`]-family schedule: apply `op` to the
/// base table (`main.sources_<src>`) and append an `(op, key, payload, seq)`
/// row to the staged feed table (`main.feed_<src>`), atomically, per design
/// §5. `row.val` is the payload for `Insert`/`Update`; for `Delete`/
/// `Retract` only `row.id` (the key) is read — `row.d`/`row.val` are still
/// carried so the feed table records the tombstone's own metadata (design
/// §5's `(op, key, payload, seq)` shape).
#[derive(Debug, Clone)]
pub struct FeedStep {
    pub op: FeedOp,
    pub row: FeedRow,
}

/// Construct a [`FeedStep`], refusing (`Err`, never a panic — this is a
/// generator-discipline check, not an infallible builder) to build a
/// [`FeedOp::Retract`] step for a [`FeedSourcePosture::NotFeed`] source
/// (plan Phase 8 TDD list: "retraction/tombstone step kinds refuse to
/// generate for non-feed postures").
pub fn build_feed_step(
    posture: FeedSourcePosture,
    op: FeedOp,
    row: FeedRow,
) -> Result<FeedStep, String> {
    if matches!(op, FeedOp::Retract) && !matches!(posture, FeedSourcePosture::ChangeFeed) {
        return Err(format!(
            "retraction/tombstone steps require FeedSourcePosture::ChangeFeed, got {posture:?}"
        ));
    }
    Ok(FeedStep { op, row })
}

/// The `change_feed` source YAML sidecar (`sources.md`'s
/// `mutation_profile: change_feed` wire spelling,
/// `smelt_core::sources::MutationProfile::ChangeFeed`'s `wire_name`):
/// clocked `events(d, id, val)`-shaped, mirroring
/// [`crate::render::render_source_yaml`]'s append-only shape apart from the
/// one literal this module exists to exercise. See the module doc comment
/// for why this bypasses [`crate::recipe::SourceRecipe::source_yaml`].
/// Branches on [`is_clocked`] since this module stages `change_feed` over
/// both the clocked `events(d, id, val)` shape ([`feed_keyed_recipe`]) and
/// the unclocked dimension shape ([`crate::recipe::SourceRecipe::mutable_dimension`],
/// reused by [`stage_feed_enriched`]).
pub fn change_feed_source_yaml(source: &SourceRecipe) -> String {
    if is_clocked(source) {
        format!(
            "description: generative-conformance change-feed source.\nmutation_profile: change_feed\ntimeseries:\n  event_time_column: {d}\n  partition_column: {d}\n  granularity: day\ncolumns:\n  - name: {d}\n    type: DATE\n  - name: {id}\n    type: INTEGER\n  - name: {val}\n    type: INTEGER\n",
            d = source.clock_column,
            id = source.key_column,
            val = source.payload_column,
        )
    } else {
        format!(
            "description: generative-conformance change-feed source.\nmutation_profile: change_feed\ncolumns:\n  - name: {id}\n    type: INTEGER\n  - name: {val}\n    type: INTEGER\n",
            id = source.key_column,
            val = source.payload_column,
        )
    }
}

/// Whether `source` carries a clocked partition column
/// ([`crate::recipe::SourceRecipe::clock_column`] non-empty) — the append-
/// only `events(d, id, val)` shape does, [`crate::recipe::SourceRecipe::mutable_dimension`]'s
/// unclocked `(id, attr)` shape does not. [`create_feed_tables`]/
/// [`apply_feed_step`]/[`change_feed_source_yaml`] all branch on this so the
/// same step family serves both shapes.
fn is_clocked(source: &SourceRecipe) -> bool {
    !source.clock_column.is_empty()
}

/// The `grain: key` recipe every Phase 8 gate test drives: a clocked
/// `events(d, id, val)` driving source (the same shape
/// [`crate::recipe::KeyedRecipe::new_window_forward`] uses), declared as
/// `change_feed` at staging time by [`stage_feed_keyed`] rather than through
/// `.source.posture` (which [`stage_feed_keyed`] never reads — see the
/// module doc comment).
pub fn feed_keyed_recipe(combiner: KeyedCombiner) -> KeyedRecipe {
    KeyedRecipe {
        model_name: format!("recipe_feed_keyed_{}", combiner.kind_name()),
        source: SourceRecipe::events(KeyShape::Single),
        combiner,
    }
}

/// Stage a [`KeyedRecipe`] (built by [`feed_keyed_recipe`]) into a fresh
/// project dir + DuckDB file, declaring its driving source as
/// `change_feed` ([`change_feed_source_yaml`]) rather than the
/// `append_only`/`mutable_snapshot` spelling
/// [`crate::render::stage_keyed`] would emit — the keyed-pool counterpart of
/// [`crate::render::stage_keyed`], kept here (outside `render.rs`, which is
/// not in this phase's edit scope) since it needs the `change_feed` literal
/// that module's `SourcePosture` match cannot produce without widening.
/// Creates BOTH the base source table and the staged feed table
/// ([`create_feed_tables`]) so a caller can immediately drive
/// [`apply_feed_step`].
pub fn stage_feed_keyed(
    recipe: &KeyedRecipe,
    project_dir: &std::path::Path,
    db_path: &std::path::Path,
) -> anyhow::Result<LinkCProject> {
    std::fs::create_dir_all(project_dir.join("models/sources"))?;
    std::fs::write(
        project_dir.join(format!("models/{}.sql", recipe.model_name)),
        render::render_keyed_model_file(recipe),
    )?;
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", recipe.source.name)),
        change_feed_source_yaml(&recipe.source),
    )?;
    std::fs::write(
        project_dir.join("smelt.yml"),
        render::render_smelt_yml(db_path),
    )?;

    let conn = Connection::open(db_path)?;
    create_feed_tables(&conn, &recipe.source)?;
    drop(conn);

    LinkCProject::load(project_dir.to_path_buf(), db_path.to_path_buf())
}

/// [`stage_feed_keyed`] generalised over [`crate::recipe::ConformanceTarget`]
/// (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 5):
/// `DuckDb` delegates straight to [`stage_feed_keyed`]; `SparkDelta` stages
/// the same model + `change_feed` source YAML + a Spark-targeted
/// `smelt.yml`, then creates the base AND feed physical tables (dropping any
/// stale ones from a prior run first — drop-before-seed idempotency,
/// mirroring `render::stage_for_target`'s convention) through a direct
/// Spark/Delta connection to `crate::recipe::SPARK_CONFORMANCE_SCHEMA`. Only
/// the classify-level admission surface
/// (`change_feed_source_admits_recompute_only`) drives this today — a
/// `grain: key` model with a fold spec over a `change_feed` source carries a
/// build-blocking `MaintenanceNoAdmissibleTechnique` diagnostic regardless of
/// backend, so nothing here is ever executed through `execute_project`.
/// `BigQuery` mirrors the same classify-only staging, creating both tables
/// in the case's own dataset with no drop-before-seed step (the dataset is
/// fresh).
#[cfg(any(feature = "spark", feature = "bigquery"))]
pub fn stage_feed_keyed_for_target(
    recipe: &KeyedRecipe,
    project_dir: &std::path::Path,
    db_path: &std::path::Path,
    target: crate::recipe::ConformanceTarget,
) -> anyhow::Result<LinkCProject> {
    match &target {
        crate::recipe::ConformanceTarget::DuckDb => stage_feed_keyed(recipe, project_dir, db_path),
        crate::recipe::ConformanceTarget::SparkDelta => {
            #[cfg(feature = "spark")]
            {
                std::fs::create_dir_all(project_dir.join("models/sources"))?;
                std::fs::write(
                    project_dir.join(format!("models/{}.sql", recipe.model_name)),
                    render::render_keyed_model_file(recipe),
                )?;
                std::fs::write(
                    project_dir.join(format!("models/sources/{}.yml", recipe.source.name)),
                    change_feed_source_yaml(&recipe.source),
                )?;
                std::fs::write(
                    project_dir.join("smelt.yml"),
                    render::render_smelt_yml_for(target, db_path),
                )?;

                create_feed_tables_via_backend(db_path, &recipe.source)?;

                LinkCProject::load(project_dir.to_path_buf(), db_path.to_path_buf())
            }
            #[cfg(not(feature = "spark"))]
            {
                unimplemented!(
                    "ConformanceTarget::SparkDelta requires the `spark` feature on \
                     smelt-maintenance-testkit"
                )
            }
        }
        crate::recipe::ConformanceTarget::BigQuery { dataset } => {
            #[cfg(feature = "bigquery")]
            {
                std::fs::create_dir_all(project_dir.join("models/sources"))?;
                std::fs::write(
                    project_dir.join(format!("models/{}.sql", recipe.model_name)),
                    render::render_keyed_model_file(recipe),
                )?;
                std::fs::write(
                    project_dir.join(format!("models/sources/{}.yml", recipe.source.name)),
                    change_feed_source_yaml(&recipe.source),
                )?;
                std::fs::write(
                    project_dir.join("smelt.yml"),
                    render::render_smelt_yml_for(target.clone(), db_path),
                )?;

                create_bigquery_feed_tables(dataset, &recipe.source)?;

                LinkCProject::load(project_dir.to_path_buf(), db_path.to_path_buf())
            }
            #[cfg(not(feature = "bigquery"))]
            {
                let _ = dataset;
                unimplemented!(
                    "ConformanceTarget::BigQuery requires the `bigquery` feature on \
                     smelt-maintenance-testkit"
                )
            }
        }
    }
}

/// [`create_feed_tables_via_backend`] generalised to BigQuery: creates both
/// the base AND feed tables in `dataset` — no drop-before-seed step (unlike
/// the Spark arm), since every `dataset` this crate constructs is fresh per
/// case (`crate::recipe::bq_conformance_dataset`).
#[cfg(feature = "bigquery")]
fn create_bigquery_feed_tables(dataset: &str, source: &SourceRecipe) -> anyhow::Result<()> {
    let dataset = dataset.to_string();
    let (base_defs, feed_defs) = if is_clocked(source) {
        (
            format!(
                "{d} DATE, {id} INT64, {val} INT64",
                d = source.clock_column,
                id = source.key_column,
                val = source.payload_column,
            ),
            format!(
                "op STRING, {d} DATE, {id} INT64, {val} INT64, seq INT64",
                d = source.clock_column,
                id = source.key_column,
                val = source.payload_column,
            ),
        )
    } else {
        (
            format!(
                "{id} INT64, {val} INT64",
                id = source.key_column,
                val = source.payload_column,
            ),
            format!(
                "op STRING, {id} INT64, {val} INT64, seq INT64",
                id = source.key_column,
                val = source.payload_column,
            ),
        )
    };
    let source_name = source.name.clone();
    crate::link_c_harness::block_on_isolated(async move {
        let backend = crate::link_c_harness::open_bigquery_backend(&dataset).await?;
        let base_table = crate::link_c_harness::bigquery_qualified_name(
            &dataset,
            &format!("sources_{source_name}"),
        )?;
        let feed_table = crate::link_c_harness::bigquery_qualified_name(
            &dataset,
            &format!("feed_{source_name}"),
        )?;
        smelt_backend::Backend::execute_sql(
            backend.as_ref(),
            &format!("CREATE TABLE {base_table} ({base_defs})"),
        )
        .await
        .map_err(|e| anyhow::anyhow!("create BigQuery feed base table {base_table}: {e}"))?;
        smelt_backend::Backend::execute_sql(
            backend.as_ref(),
            &format!("CREATE TABLE {feed_table} ({feed_defs})"),
        )
        .await
        .map_err(|e| anyhow::anyhow!("create BigQuery feed log table {feed_table}: {e}"))?;
        Ok::<(), anyhow::Error>(())
    })
}

/// [`create_feed_tables`] generalised over `&dyn Backend` (plan Phase 5):
/// drop-then-create both the base AND feed tables in
/// `crate::recipe::SPARK_CONFORMANCE_SCHEMA` — the Spark counterpart of
/// [`create_feed_tables`], run on an isolated thread
/// (`link_c_harness::block_on_isolated`) mirroring `render.rs`'s own
/// staging-DDL convention.
#[cfg(feature = "spark")]
fn create_feed_tables_via_backend(
    db_path: &std::path::Path,
    source: &SourceRecipe,
) -> anyhow::Result<()> {
    let db_path = db_path.to_path_buf();
    let schema = crate::recipe::SPARK_CONFORMANCE_SCHEMA;
    let base_table = format!("{schema}.sources_{}", source.name);
    let feed_table = format!("{schema}.feed_{}", source.name);
    let (base_defs, feed_defs) = if is_clocked(source) {
        (
            format!(
                "{d} DATE, {id} INT, {val} INT",
                d = source.clock_column,
                id = source.key_column,
                val = source.payload_column,
            ),
            format!(
                "op STRING, {d} DATE, {id} INT, {val} INT, seq INT",
                d = source.clock_column,
                id = source.key_column,
                val = source.payload_column,
            ),
        )
    } else {
        (
            format!(
                "{id} INT, {val} INT",
                id = source.key_column,
                val = source.payload_column,
            ),
            format!(
                "op STRING, {id} INT, {val} INT, seq INT",
                id = source.key_column,
                val = source.payload_column,
            ),
        )
    };
    crate::link_c_harness::block_on_isolated(async move {
        let backend = crate::link_c_harness::open_spark_conformance_backend(&db_path).await?;
        for table in [&base_table, &feed_table] {
            smelt_backend::Backend::execute_sql(
                backend.as_ref(),
                &format!("DROP TABLE IF EXISTS {table}"),
            )
            .await
            .map_err(|e| anyhow::anyhow!("drop stale Spark feed table {table}: {e}"))?;
        }
        smelt_backend::Backend::execute_sql(
            backend.as_ref(),
            &format!("CREATE TABLE {base_table} ({base_defs}) USING DELTA"),
        )
        .await
        .map_err(|e| anyhow::anyhow!("create Spark feed base table {base_table}: {e}"))?;
        smelt_backend::Backend::execute_sql(
            backend.as_ref(),
            &format!("CREATE TABLE {feed_table} ({feed_defs}) USING DELTA"),
        )
        .await
        .map_err(|e| anyhow::anyhow!("create Spark feed log table {feed_table}: {e}"))?;
        Ok::<(), anyhow::Error>(())
    })
}

/// Stage a [`crate::recipe::MutableEnrichedRecipe`] (the fact +
/// unclocked-dimension mixed shape, `docs/plans/20260712-generative-maintenance-conformance.md`
/// Phase 4) into a fresh project dir + DuckDB file, declaring the DIMENSION
/// as `change_feed` ([`change_feed_source_yaml`]) rather than the
/// `mutable_snapshot` spelling
/// [`crate::recipe::MutableEnrichedRecipe::dimension_source_yaml`] would
/// emit. Unlike a `grain: key` model with a fold spec
/// ([`stage_feed_keyed`]'s pool), this shape builds WITHOUT any refusal:
/// the trigger-list builder only ever constructs an `UpstreamMutation`
/// trigger for a source whose OWN declared posture literally reads
/// `mutable_snapshot` (`incremental_models.md` §Known Divergences'
/// `change_feed`-scoping entry) — a `change_feed`-declared dimension gets
/// no mutation cell constructed for it AT ALL, so there is nothing to
/// refuse; the fact's own creation cell is untouched. This is the shape
/// `feed_declared_source_upholds_equivalence_via_recompute` drives through
/// the real `execute_project` pipeline, since the keyed+fold shape cannot
/// build at all (a fold-refusal diagnostic is Error-severity and blocks
/// `smelt build` outright, regardless of the universal Backfill cell still
/// succeeding underneath it).
///
/// Pre-seeds the dimension with one row per id in `1..=dim_seed_max_id`
/// (`attr = id * 100`), mirroring `gate.rs`'s own `stage_mixed_recipe`
/// convention, so every fact row a caller inserts already has a matching
/// dimension row to join against.
pub fn stage_feed_enriched(
    recipe: &crate::recipe::MutableEnrichedRecipe,
    project_dir: &std::path::Path,
    db_path: &std::path::Path,
    dim_seed_max_id: i64,
) -> anyhow::Result<LinkCProject> {
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
        change_feed_source_yaml(&recipe.dimension),
    )?;
    std::fs::write(
        project_dir.join("smelt.yml"),
        render::render_smelt_yml(db_path),
    )?;

    let conn = Connection::open(db_path)?;
    conn.execute_batch(&format!(
        "CREATE SCHEMA IF NOT EXISTS main; \
         CREATE TABLE main.sources_{fact} ({d} DATE, {id} INTEGER, {val} INTEGER);",
        fact = recipe.fact.name,
        d = recipe.fact.clock_column,
        id = recipe.fact.key_column,
        val = recipe.fact.payload_column,
    ))?;
    create_feed_tables(&conn, &recipe.dimension)?;
    for id in 1..=dim_seed_max_id {
        conn.execute(
            &format!(
                "INSERT INTO main.sources_{} VALUES ({}, {})",
                recipe.dimension.name,
                id,
                id * 100
            ),
            [],
        )?;
    }
    drop(conn);

    LinkCProject::load(project_dir.to_path_buf(), db_path.to_path_buf())
}

/// Create the base source table (`main.sources_<name>`) plus the staged
/// feed table (`main.feed_<name>`, design §5's `(op, key, payload, seq)`
/// shape) — clocked `(d, id, val)` or unclocked `(id, val)` per
/// [`is_clocked`].
pub fn create_feed_tables(conn: &Connection, source: &SourceRecipe) -> anyhow::Result<()> {
    if is_clocked(source) {
        conn.execute_batch(&format!(
            "CREATE SCHEMA IF NOT EXISTS main; \
             CREATE TABLE main.sources_{name} ({d} DATE, {id} INTEGER, {val} INTEGER); \
             CREATE TABLE main.feed_{name} (op VARCHAR, {d} DATE, {id} INTEGER, {val} INTEGER, seq INTEGER);",
            name = source.name,
            d = source.clock_column,
            id = source.key_column,
            val = source.payload_column,
        ))?;
    } else {
        conn.execute_batch(&format!(
            "CREATE SCHEMA IF NOT EXISTS main; \
             CREATE TABLE main.sources_{name} ({id} INTEGER, {val} INTEGER); \
             CREATE TABLE main.feed_{name} (op VARCHAR, {id} INTEGER, {val} INTEGER, seq INTEGER);",
            name = source.name,
            id = source.key_column,
            val = source.payload_column,
        ))?;
    }
    Ok(())
}

/// Apply one [`FeedStep`] to `source`'s staged base table AND append its
/// `(op, key, payload, seq)` row to the staged feed table — the "applies to
/// the base table *and* appended as a feed row" contract design §5 states.
/// `seq` is the caller-assigned monotonic sequence number (a schedule's own
/// step index is the natural choice — see call sites).
pub fn apply_feed_step(
    conn: &Connection,
    source: &SourceRecipe,
    step: &FeedStep,
    seq: i64,
) -> anyhow::Result<()> {
    let table = format!("main.sources_{}", source.name);
    let clocked = is_clocked(source);
    let FeedRow { d, id, val } = step.row;
    match step.op {
        FeedOp::Insert => {
            if clocked {
                conn.execute(
                    &format!(
                        "INSERT INTO {table} VALUES (DATE '{}', {id}, {val})",
                        d.format("%Y-%m-%d"),
                    ),
                    [],
                )?;
            } else {
                conn.execute(&format!("INSERT INTO {table} VALUES ({id}, {val})"), [])?;
            }
        }
        FeedOp::Update => {
            conn.execute(
                &format!(
                    "UPDATE {table} SET {val_col} = {val} WHERE {id_col} = {id}",
                    val_col = source.payload_column,
                    id_col = source.key_column,
                ),
                [],
            )?;
        }
        FeedOp::Delete | FeedOp::Retract => {
            conn.execute(
                &format!(
                    "DELETE FROM {table} WHERE {id_col} = {id}",
                    id_col = source.key_column,
                ),
                [],
            )?;
        }
    }

    if clocked {
        conn.execute(
            &format!(
                "INSERT INTO main.feed_{name} VALUES ('{op}', DATE '{d_lit}', {id}, {val}, {seq})",
                name = source.name,
                op = step.op.wire_name(),
                d_lit = d.format("%Y-%m-%d"),
            ),
            [],
        )?;
    } else {
        conn.execute(
            &format!(
                "INSERT INTO main.feed_{name} VALUES ('{op}', {id}, {val}, {seq})",
                name = source.name,
                op = step.op.wire_name(),
            ),
            [],
        )?;
    }
    Ok(())
}

/// Replay the staged feed table (`main.feed_<name>`, ordered by `seq`) into
/// the row set it implies — folding `insert`/`update` as an upsert-by-key
/// and `delete`/`retract` as a removal-by-key. [`FeedOp::Retract`] and
/// [`FeedOp::Delete`] replay identically (design §5 records the retraction
/// as its own named op for the day a fold consumes it, but v1's replay
/// self-check only needs "row is gone"). Scoped to the CLOCKED shape only
/// (the only shape this module's own `feed_table_records_every_mutation_step`
/// self-check exercises); [`stage_feed_enriched`]'s unclocked dimension
/// never calls this.
pub fn replay_feed(conn: &Connection, source: &SourceRecipe) -> anyhow::Result<Vec<FeedRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT op, CAST({d} AS VARCHAR), {id}, {val} FROM main.feed_{name} ORDER BY seq",
        d = source.clock_column,
        id = source.key_column,
        val = source.payload_column,
        name = source.name,
    ))?;
    let rows: Vec<(String, String, i64, i64)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?
        .collect::<Result<_, _>>()?;

    let mut state: std::collections::HashMap<i64, FeedRow> = std::collections::HashMap::new();
    for (op, d_text, id, val) in rows {
        let d = NaiveDate::parse_from_str(&d_text, "%Y-%m-%d")?;
        match op.as_str() {
            "insert" => {
                state.insert(id, FeedRow { d, id, val });
            }
            // `apply_feed_step`'s Update only ever writes the base table's
            // payload column (`UPDATE ... SET val = ... WHERE id = ...`,
            // never `d`) — mirrored here so replay agrees with the base
            // table on which column actually changed.
            "update" => {
                if let Some(existing) = state.get_mut(&id) {
                    existing.val = val;
                } else {
                    state.insert(id, FeedRow { d, id, val });
                }
            }
            "delete" | "retract" => {
                state.remove(&id);
            }
            other => anyhow::bail!(
                "unrecognized feed op {other:?} in main.feed_{}",
                source.name
            ),
        }
    }
    let mut result: Vec<FeedRow> = state.into_values().collect();
    result.sort_by_key(|r| r.id);
    Ok(result)
}

/// A generated sequence of [`FeedStep`]s over one source, under `posture`.
#[derive(Debug, Clone)]
pub struct FeedStepSchedule(pub Vec<FeedStep>);

/// One op choice [`arb_feed_step_schedule`] draws from — weighted toward
/// `Insert` so a schedule always has rows to `Update`/`Delete`/`Retract`
/// against.
#[derive(Debug, Clone, Copy)]
enum OpChoice {
    Insert,
    Update,
    Delete,
    Retract,
}

fn arb_op_choice(posture: FeedSourcePosture) -> BoxedStrategy<OpChoice> {
    match posture {
        FeedSourcePosture::ChangeFeed => prop_oneof![
            3 => Just(OpChoice::Insert),
            2 => Just(OpChoice::Update),
            1 => Just(OpChoice::Delete),
            1 => Just(OpChoice::Retract),
        ]
        .boxed(),
        FeedSourcePosture::NotFeed => prop_oneof![
            3 => Just(OpChoice::Insert),
            2 => Just(OpChoice::Update),
            1 => Just(OpChoice::Delete),
        ]
        .boxed(),
    }
}

/// Schema-generic feed-mutation schedule generator (design §5 "Simulated
/// change feed"): 3-6 steps drawn from [`arb_op_choice`] plus an integer
/// payload ([`crate::recipe::arb_payload_value`]) per step, folded into a
/// [`FeedStepSchedule`] by [`build_feed_schedule`] — `Update`/`Delete`/
/// `Retract` choices that land before any row has been inserted fall back to
/// `Insert` (there is nothing yet to mutate), so every generated schedule is
/// valid by construction: [`build_feed_step`] never returns `Err` for a step
/// this generator itself produced.
pub fn arb_feed_step_schedule(
    posture: FeedSourcePosture,
) -> impl Strategy<Value = FeedStepSchedule> {
    (3_usize..=6).prop_flat_map(move |n| {
        proptest::collection::vec(
            (arb_op_choice(posture), crate::recipe::arb_payload_value()),
            n,
        )
        .prop_map(move |choices| build_feed_schedule(posture, &choices))
    })
}

fn build_feed_schedule(
    posture: FeedSourcePosture,
    choices: &[(OpChoice, i64)],
) -> FeedStepSchedule {
    let base = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid base date");
    let mut steps = Vec::new();
    let mut alive_ids: Vec<i64> = Vec::new();
    let mut next_id = 1_i64;

    for (i, (choice, val)) in choices.iter().enumerate() {
        let effective = if alive_ids.is_empty() {
            OpChoice::Insert
        } else {
            *choice
        };
        let op = match effective {
            OpChoice::Insert => FeedOp::Insert,
            OpChoice::Update => FeedOp::Update,
            OpChoice::Delete => FeedOp::Delete,
            OpChoice::Retract => FeedOp::Retract,
        };
        let row = match effective {
            OpChoice::Insert => {
                let row = FeedRow {
                    d: base + chrono::Duration::days(i as i64),
                    id: next_id,
                    val: *val,
                };
                alive_ids.push(next_id);
                next_id += 1;
                row
            }
            OpChoice::Update => {
                let id = alive_ids[i % alive_ids.len()];
                FeedRow {
                    d: base + chrono::Duration::days(i as i64),
                    id,
                    val: *val,
                }
            }
            OpChoice::Delete | OpChoice::Retract => {
                let idx = i % alive_ids.len();
                let id = alive_ids.remove(idx);
                FeedRow {
                    d: base + chrono::Duration::days(i as i64),
                    id,
                    val: *val,
                }
            }
        };
        let step = build_feed_step(posture, op, row)
            .expect("build_feed_schedule only ever constructs ops valid for its own posture");
        steps.push(step);
    }

    FeedStepSchedule(steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule_gen::read_source_snapshot;
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::TestRunner;

    /// `retraction_steps_are_gated_to_feed_sources` (plan Phase 8 TDD list):
    /// retraction/tombstone step kinds refuse to generate for non-feed
    /// postures (self-check).
    #[test]
    fn retraction_steps_are_gated_to_feed_sources() {
        let row = FeedRow {
            d: chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date"),
            id: 1,
            val: 5,
        };

        assert!(
            build_feed_step(FeedSourcePosture::ChangeFeed, FeedOp::Retract, row.clone()).is_ok(),
            "a ChangeFeed-postured source must be able to construct a Retract step"
        );
        let err = build_feed_step(FeedSourcePosture::NotFeed, FeedOp::Retract, row)
            .expect_err("a NotFeed-postured source must refuse a Retract step");
        assert!(
            err.contains("ChangeFeed"),
            "refusal message must name the required posture, got: {err:?}"
        );

        // Generator-level self-check: a deterministic sample of schedules
        // generated under NotFeed posture never contains a Retract step —
        // never trust an unverified gate (design §5 "F7"'s discipline,
        // applied here to this module's own generator).
        let mut runner = TestRunner::deterministic();
        let strat = arb_feed_step_schedule(FeedSourcePosture::NotFeed);
        for i in 0..50 {
            let schedule = strat.new_tree(&mut runner).unwrap().current();
            assert!(
                !schedule.0.iter().any(|s| matches!(s.op, FeedOp::Retract)),
                "case {i}: NotFeed-posture schedule generated a Retract step: {schedule:?}"
            );
        }
    }

    /// `feed_table_records_every_mutation_step` (plan Phase 8 TDD list):
    /// each mutation step against a feed-declared source appends exactly
    /// one feed row; base table and feed replay agree (feed self-check).
    #[test]
    fn feed_table_records_every_mutation_step() {
        let mut runner = TestRunner::deterministic();
        let strat = arb_feed_step_schedule(FeedSourcePosture::ChangeFeed);
        let source = SourceRecipe::events(KeyShape::Single);

        for i in 0..30 {
            let schedule = strat.new_tree(&mut runner).unwrap().current();

            let tmp = tempfile::TempDir::new().expect("tempdir");
            let db_path = tmp.path().join("db.duckdb");
            let conn = Connection::open(&db_path).expect("open duckdb");
            create_feed_tables(&conn, &source).expect("create feed tables");

            for (step_i, step) in schedule.0.iter().enumerate() {
                apply_feed_step(&conn, &source, step, step_i as i64).unwrap_or_else(|e| {
                    panic!("case {i} step {step_i}: apply_feed_step failed: {e}")
                });
            }

            let feed_count: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM main.feed_{}", source.name),
                    [],
                    |r| r.get(0),
                )
                .expect("count feed rows");
            assert_eq!(
                feed_count as usize,
                schedule.0.len(),
                "case {i}: expected exactly one feed row per mutation step, schedule={schedule:?}"
            );

            let mut replayed = replay_feed(&conn, &source).expect("replay feed");
            let mut base = read_source_snapshot(&conn, &source);
            replayed.sort_by_key(|r| r.id);
            base.sort_by_key(|r| r.id);
            assert_eq!(
                replayed, base,
                "case {i}: replaying the feed table must reproduce the base table's current \
                 contents exactly, schedule={schedule:?}"
            );
        }
    }
}
