//! The keyed-succession (SCD2) family's own stage/insert/assert/drive
//! quartet (`docs/outcomes/20260906-scd2-keyed-succession/phases/
//! 07a-plan.md`), modelled on `families::gate_keyed`'s shape and naming.
//!
//! **Not under `families/`.** That module is file-wide gated
//! (`#![cfg(any(feature = "spark", feature = "bigquery"))]`) because its
//! `ConformanceBackend` abstraction exists to share test bodies across the
//! Spark/BigQuery generative-conformance twins, which the succession grain
//! does not have yet (Spark/BigQuery take the recorded availability
//! downgrade for this grain — `docs/outcomes/20260906-scd2-keyed-succession/
//! outcome.md` §Out of scope). A DuckDB-only family that the per-PR
//! reference leg (`crates/smelt-cli/tests/maintenance_conformance/`, built
//! with no optional feature) must call at all cannot live inside a module
//! gated off by default — so this quartet lives here instead, ungated,
//! alongside this crate's other always-compiled modules, in the plainer
//! `project.connect()`/`project.backend()` idiom
//! `crates/smelt-cli/tests/maintenance_conformance/gate/keyed_support.rs` /
//! `keyed_oracle.rs` already use for the DuckDB reference leg's own keyed
//! family, rather than `families::gate_keyed`'s target-generalized
//! `dyn Backend`/`ConformanceTarget` parametrization (which exists to share
//! bodies with backends this grain does not support yet).
//! [`stage_succession_recipe_for`] still takes a
//! [`crate::recipe::ConformanceTarget`] and still refuses non-DuckDB (via
//! [`crate::render::stage_succession_for_target`]) so a later phase widening
//! this family to more backends only has to change the render-layer
//! staging, not this quartet's shape.

use anyhow::Result;
use chrono::{NaiveDate, NaiveDateTime};

use crate::link_c_harness::{base_request, LinkCProject};
use crate::oracle::multiset_equal_via_backend;
use crate::recipe::{ConformanceTarget, SuccessionRecipe};
use crate::render;

/// One event row for a [`SuccessionRecipe`]'s driving source: `key` +
/// `event_time` (the succession clock, `changed_at`) + `arrival` (the
/// source's declared `timeseries.partition_column`, `arrival_date` — a run's
/// window scans THIS column, not `event_time`) + `payload` (the `tier`
/// VARCHAR column) + `is_deleted` (the optional `QUALIFY NOT is_deleted`
/// delete flag, always physically present and `NOT NULL`).
#[derive(Debug, Clone)]
pub struct SuccessionEventRow {
    pub key: i64,
    pub event_time: NaiveDateTime,
    pub arrival: NaiveDate,
    pub payload: String,
    pub is_deleted: bool,
}

impl SuccessionEventRow {
    /// A non-deleted event landing on its own event day (`arrival ==
    /// event_time`'s date) — the common case every splice/lag smoke case
    /// starts from.
    pub fn new(key: i64, event_time: NaiveDateTime, payload: &str) -> Self {
        Self {
            key,
            event_time,
            arrival: event_time.date(),
            payload: payload.to_string(),
            is_deleted: false,
        }
    }

    /// [`Self::new`], additionally declaring a distinct arrival date — the
    /// late-arrival/splice shape: an event whose `event_time` places it
    /// between two already-processed events of the same key, but whose
    /// `arrival` lands in a LATER window than either of them.
    pub fn late(key: i64, event_time: NaiveDateTime, payload: &str, arrival: NaiveDate) -> Self {
        Self {
            key,
            event_time,
            arrival,
            payload: payload.to_string(),
            is_deleted: false,
        }
    }
}

/// Stage a [`SuccessionRecipe`] into a fresh temp project dir targeting
/// `target` (`render::stage_succession_for_target`'s DuckDB-only staging;
/// non-DuckDB refuses loudly rather than silently doing nothing).
pub fn stage_succession_recipe_for(
    recipe: &SuccessionRecipe,
    tmp: &tempfile::TempDir,
    target: ConformanceTarget,
) -> Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(&project_dir)?;
    render::stage_succession_for_target(recipe, &project_dir, &db_path, target)
}

/// Insert one row into a [`SuccessionRecipe`]'s staged driving-source table.
pub fn insert_row_succession_for(
    project: &LinkCProject,
    recipe: &SuccessionRecipe,
    row: &SuccessionEventRow,
) -> Result<()> {
    let is_deleted = if row.is_deleted { "TRUE" } else { "FALSE" };
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "INSERT INTO main.sources_{name} VALUES ({key}, TIMESTAMP '{event_time}', DATE \
             '{arrival}', '{payload}', {is_deleted})",
            name = recipe.source.name,
            key = row.key,
            event_time = row.event_time.format("%Y-%m-%d %H:%M:%S"),
            arrival = row.arrival.format("%Y-%m-%d"),
            payload = row.payload,
        ),
        [],
    )?;
    Ok(())
}

/// The end-state equivalence assertion for a [`SuccessionRecipe`]: the
/// maintained table's full contents equal the model's own SQL evaluated over
/// the CURRENT physical source table (`render::render_succession_oracle_body_over`) —
/// no S-restricted materialization needed (unlike the keyed pool's
/// `STracker`): the succession-patch technique's own equivalence invariant
/// is against a full-refresh oracle over every event landed so far, exactly
/// the leg `crates/smelt-runtime/tests/statement_parity/succession.rs`'s
/// `succession_patch_result_equals_full_refresh` already proves for one
/// fixed recipe — this reuses that same `oracle_relation` seam
/// (`docs/outcomes/20260906-scd2-keyed-succession/phases/07a-plan.md` task
/// 5) rather than introducing a second comparator.
pub async fn assert_succession_equivalence_for(
    project: &LinkCProject,
    recipe: &SuccessionRecipe,
) -> Result<()> {
    let backend = project.backend().await?;
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let source_ref = format!("main.sources_{}", recipe.source.name);
    let oracle_sql = render::render_succession_oracle_body_over(recipe, &source_ref);
    let equal = multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql).await?;
    if !equal {
        anyhow::bail!(
            "succession end-state equivalence violated for model {:?}: maintained \
             ({maintained_sql:?}) != oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// Drive one window's worth of `rows` against `project`/`recipe` through the
/// real `execute_project` pipeline (`LinkCProject::run_quiet`), then assert
/// end-state equivalence — the succession-family counterpart of
/// `families::gate_keyed::drive_keyed_and_assert_for`, restricted to a
/// single window per call so a multi-window smoke schedule can insert a
/// splice/late row between calls.
pub async fn drive_succession_window_and_assert_for(
    project: &LinkCProject,
    recipe: &SuccessionRecipe,
    run_id: &str,
    window_start: NaiveDate,
    window_end: NaiveDate,
    rows: &[SuccessionEventRow],
) -> Result<()> {
    for row in rows {
        insert_row_succession_for(project, recipe, row)?;
    }

    let mut request = base_request("dev");
    request.start = Some(window_start.format("%Y-%m-%d").to_string());
    request.end = Some(window_end.format("%Y-%m-%d").to_string());
    project
        .run_quiet(run_id, request)
        .await
        .map_err(|e| anyhow::anyhow!("succession run {run_id:?} failed: {e}"))?;

    assert_succession_equivalence_for(project, recipe).await
}
