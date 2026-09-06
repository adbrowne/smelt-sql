//! `run_bakeoff` orchestration: builds the Salsa DB, derives the candidate
//! cells, replays each technique variant into a scratch schema, and
//! cross-checks every variant pair with `EXCEPT ALL`.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use tokio_util::sync::CancellationToken;

use smelt_core::config::{CellTechnique, Config};
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_logical::maintenance::Trigger;
use smelt_runtime::types::{CellTechniqueOverride, ExecuteRequest};
use smelt_runtime::{execute_project, NoOpReporter};

use crate::backend_factory::CliBackendFactory;
use crate::init_db;

use super::cells::{build_pin_suggestion, candidate_cells};
use super::{
    technique_slug, BakeoffCellReport, BakeoffOptions, BakeoffReport, TechniqueMeasurement,
};

/// Build the Salsa DB + dependency graph for one `execute_project` call.
/// Rebuilt per call (mirrors `smelt-maintenance-testkit`'s `LinkCProject`)
/// so a between-window mutation on disk is picked up.
type SharedDb = Arc<tokio::sync::Mutex<smelt_db::Database>>;
type SharedGraph = Arc<tokio::sync::Mutex<DependencyGraph>>;

#[cfg_attr(not(feature = "duckdb"), allow(dead_code))]
fn build_db_and_graph(
    project_dir: &Path,
    config: &Config,
    real_target: &str,
) -> Result<(SharedDb, SharedGraph)> {
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let sql_models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;
    let mut db = init_db(project_dir, &sql_models);
    db.set_active_target(Some(Arc::from(real_target)));
    let graph = DependencyGraph::build(sql_models, None)
        .with_context(|| "Failed to build dependency graph")?;
    Ok((
        Arc::new(tokio::sync::Mutex::new(db)),
        Arc::new(tokio::sync::Mutex::new(graph)),
    ))
}

/// A collision-safe scratch schema name for one (cell, technique) pair. The
/// common case (a single bakeoff cell — the norm for a bakeoff invocation
/// scoped to one model) matches `incremental_models.md` §"CLI" literally:
/// `smelt_bakeoff_<model>_<technique>`. With 2+ candidate cells in the same
/// run, a per-cell index disambiguates so two cells' same-named technique
/// never collide on one schema.
#[cfg_attr(not(feature = "duckdb"), allow(dead_code))]
fn scratch_schema_name(
    model: &str,
    cell_index: usize,
    total_cells: usize,
    technique: CellTechnique,
) -> String {
    let model_slug = model.replace(['.', '-'], "_");
    if total_cells <= 1 {
        format!("smelt_bakeoff_{model_slug}_{}", technique_slug(technique))
    } else {
        format!(
            "smelt_bakeoff_{model_slug}_c{cell_index}_{}",
            technique_slug(technique)
        )
    }
}

/// Every declared `smelt.sources.<...>` ref's physical table name
/// (`sources_<dot-joined remainder, underscore-joined>` — the same
/// `address_segments.join("_")` convention `ModelFile`'s doc comment
/// states) the model reads directly. Excludes model-to-model refs — an
/// upstream maintained model is rebuilt fresh into the scratch schema by
/// the `+<model>` selection instead of viewed from the real schema.
#[cfg_attr(not(feature = "duckdb"), allow(dead_code))]
fn model_source_tables(model: &crate::ModelFile) -> Vec<String> {
    let mut tables: Vec<String> = model
        .refs
        .iter()
        .filter_map(|r| {
            let segs = r.smelt_ref.to_path();
            if segs.first().map(String::as_str) != Some("sources") {
                return None;
            }
            Some(format!("sources_{}", segs[1..].join("_")))
        })
        .collect();
    tables.sort();
    tables.dedup();
    tables
}

#[cfg_attr(not(feature = "duckdb"), allow(dead_code))]
fn state_dir_for(project_dir: &Path, target: &str) -> std::path::PathBuf {
    project_dir.join(".smelt").join("targets").join(target)
}

/// Split `[start, end]` (inclusive dates, `YYYY-MM-DD`) into `runs`
/// sequential, contiguous, non-overlapping `[window_start, window_end)`
/// windows (`window_end` exclusive, matching `ExecuteRequest::start`/`end`
/// convention) covering the whole extent — decision B4's window slicer.
#[cfg_attr(not(feature = "duckdb"), allow(dead_code))]
fn slice_windows(start: &str, end: &str, runs: u32) -> Result<Vec<(String, String)>> {
    use chrono::NaiveDate;
    let start_date = NaiveDate::parse_from_str(start, "%Y-%m-%d")
        .with_context(|| format!("invalid extent start date '{start}'"))?;
    let end_date_inclusive = NaiveDate::parse_from_str(end, "%Y-%m-%d")
        .with_context(|| format!("invalid extent end date '{end}'"))?;
    // The window end is exclusive, so the last window must reach one day
    // past the last observed date to include it.
    let end_exclusive = end_date_inclusive + chrono::Duration::days(1);
    let total_days = (end_exclusive - start_date).num_days().max(1) as u32;
    let runs = runs.max(1);
    let base = (total_days / runs).max(1);
    let mut windows = Vec::new();
    let mut cursor = start_date;
    for i in 0..runs {
        if cursor >= end_exclusive {
            break;
        }
        let is_last = i + 1 == runs;
        let window_end = if is_last {
            end_exclusive
        } else {
            (cursor + chrono::Duration::days(base as i64)).min(end_exclusive)
        };
        windows.push((
            cursor.format("%Y-%m-%d").to_string(),
            window_end.format("%Y-%m-%d").to_string(),
        ));
        cursor = window_end;
    }
    Ok(windows)
}

/// Run `smelt bakeoff <model>` — see the module doc comment.
#[cfg(feature = "duckdb")]
pub async fn run_bakeoff(
    project_dir: &Path,
    config: Arc<Config>,
    model_name: &str,
    opts: BakeoffOptions,
) -> Result<BakeoffReport> {
    use super::duckdb_probe;

    let selectors = opts.cells.clone();

    // ── 1. Derive the maintenance plan (Salsa purity: assemble inputs, call
    // the pure derivation via `smelt_db::maintenance_plan_report`). ──
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;
    let model = models
        .iter()
        .find(|m| m.canonical_path() == model_name || m.name == model_name)
        .with_context(|| format!("model '{model_name}' not found"))?
        .clone();

    let mut plan_db = init_db(project_dir, &models);
    plan_db.set_active_target(Some(Arc::from(opts.target.as_str())));
    let ws = smelt_db::Workspace::try_get(&plan_db).context("workspace not initialized")?;
    let file = plan_db
        .source_file(&model.path)
        .with_context(|| format!("model file not registered: {}", model.path.display()))?;
    let result = smelt_db::maintenance_plan_report(&plan_db, ws, file).with_context(|| {
        format!(
            "'{model_name}' is not an incremental model with a declared grain — nothing for \
             bakeoff to measure"
        )
    })?;

    let candidates = candidate_cells(&result.plan.cells, &result.column_groups, &selectors)?;
    if candidates.is_empty() {
        return Ok(BakeoffReport {
            model: model_name.to_string(),
            target: opts.target.clone(),
            runs: opts.runs,
            cells: Vec::new(),
            message: Some(
                "no cell in this model's maintenance plan admits 2+ techniques — nothing to \
                 measure. Every trigger resolves to exactly one technique today, so there is \
                 no alternative to bake off."
                    .to_string(),
            ),
            kept_schemas: Vec::new(),
            pin: None,
        });
    }

    // ── 2. Resolve the target to clone + the driving event-time extent. ──
    let real_target = config
        .targets
        .get(&opts.target)
        .with_context(|| format!("target '{}' not found in smelt.yml", opts.target))?
        .clone();
    if real_target.target_type != "duckdb" {
        bail!(
            "smelt bakeoff currently supports only duckdb targets (target '{}' is '{}')",
            opts.target,
            real_target.target_type
        );
    }
    let database_path = project_dir.join(
        real_target
            .database
            .clone()
            .with_context(|| format!("target '{}' has no database path", opts.target))?,
    );

    let timeseries = model
        .metadata
        .as_deref()
        .and_then(|m| m.timeseries.as_ref())
        .with_context(|| format!("model '{model_name}' has no `timeseries:` block — bakeoff's window replay needs a declared event-time clock"))?
        .clone();

    // The driving source's own physical table: `Trigger::NewData { source }`
    // names the ref-path remainder after the `sources.` prefix
    // (`smelt_db::maintenance_plan_report`'s `bare` stripping), and every
    // source's physical table name is `sources_<that address, dot-joined
    // segments underscore-joined>` — the same `address_segments.join("_")`
    // convention `ModelFile`'s own doc comment states, since a declared
    // source's address always begins with the `sources` segment.
    let driving_source = result
        .plan
        .cells
        .iter()
        .find_map(|c| match &c.trigger {
            Trigger::NewData { source } => Some(source.clone()),
            _ => None,
        })
        .with_context(|| {
            format!("'{model_name}' has no creation-trigger cell to derive a driving source from")
        })?;
    let driving_table = format!("sources_{}", driving_source.replace('.', "_"));

    let conn = duckdb_probe::open(&database_path)?;
    let extent = duckdb_probe::event_time_extent(
        &conn,
        &real_target.schema,
        &driving_table,
        &timeseries.event_time_column,
    )?
    .with_context(|| {
        format!(
            "no data found in {}.{driving_table} — nothing to replay for '{model_name}'",
            real_target.schema
        )
    })?;
    let windows = slice_windows(&extent.0, &extent.1, opts.runs)?;
    drop(conn);

    let physical_table = model.address_segments.join("_");
    let source_tables = model_source_tables(&model);
    let total_cells = candidates.len();
    let mut cell_reports = Vec::new();
    let mut created_schemas: Vec<String> = Vec::new();

    for (cell_index, cell) in candidates.iter().enumerate() {
        let variants = [cell.admitted, CellTechnique::Recompute];
        let mut measurements = Vec::new();

        for technique in variants {
            let schema = scratch_schema_name(model_name, cell_index, total_cells, technique);
            created_schemas.push(schema.clone());

            let mut scratch_config = (*config).clone();
            let mut scratch_target = real_target.clone();
            scratch_target.schema = schema.clone();
            let scratch_target_name = format!("__bakeoff_{schema}");
            scratch_config
                .targets
                .insert(scratch_target_name.clone(), scratch_target);
            let scratch_config = Arc::new(scratch_config);

            {
                let conn = duckdb_probe::open(&database_path)?;
                duckdb_probe::ensure_scratch_source_views(
                    &conn,
                    &schema,
                    &real_target.schema,
                    &source_tables,
                )?;
            }

            let mut run_ms = Vec::new();
            for (i, (start, end)) in windows.iter().enumerate() {
                let (db, graph) = build_db_and_graph(project_dir, &scratch_config, &opts.target)?;
                let request = ExecuteRequest {
                    target: scratch_target_name.clone(),
                    select: vec![format!("+{model_name}")],
                    exclude: vec![],
                    start: Some(start.clone()),
                    end: Some(end.clone()),
                    batch_size_days: None,
                    per_partition: false,
                    full_refresh: false,
                    rebuild: false,
                    dry_run: false,
                    enforce_safety: false,
                    allow_column_removal: false,
                    allow_full_refresh: false,
                    ephemeral_seed_ctes: vec![],
                    run_checks: false,
                    checks: vec![],
                    jobs: Some(1),
                    retry_max: Some(0),
                    retry_backoff_ms: None,
                    resume: false,
                    technique_overrides: vec![CellTechniqueOverride {
                        columns: cell.columns.clone(),
                        on: cell.on.clone(),
                        technique,
                    }],
                };
                let run_id = format!("bakeoff-{schema}-{i}");
                let backend_factory = CliBackendFactory {
                    database_override: None,
                };
                let started = Instant::now();
                execute_project(
                    run_id,
                    request,
                    Arc::clone(&scratch_config),
                    graph,
                    db,
                    project_dir,
                    &backend_factory,
                    &NoOpReporter,
                    CancellationToken::new(),
                )
                .await
                .with_context(|| {
                    format!(
                        "bakeoff replay window {start}..{end} failed for technique \
                         '{}' on cell (on={}, columns={:?})",
                        technique_slug(technique),
                        cell.on,
                        cell.columns
                    )
                })?;
                run_ms.push(started.elapsed().as_millis());
            }

            let conn = duckdb_probe::open(&database_path)?;
            let row_count = duckdb_probe::row_count(&conn, &schema, &physical_table)?;
            measurements.push(TechniqueMeasurement {
                technique,
                run_wall_clock_ms: run_ms,
                row_count,
                scratch_schema: schema,
            });
        }

        // ── Cross-variant equivalence (`EXCEPT ALL`, both directions). ──
        let conn = duckdb_probe::open(&database_path)?;
        let left = &measurements[0].scratch_schema;
        let right = &measurements[1].scratch_schema;
        let forward = duckdb_probe::except_all_count(&conn, left, right, &physical_table)?;
        let backward = duckdb_probe::except_all_count(&conn, right, left, &physical_table)?;
        if forward != 0 || backward != 0 {
            if !opts.keep {
                for schema in &created_schemas {
                    let _ = duckdb_probe::drop_schema(&conn, schema);
                    let _ = std::fs::remove_dir_all(state_dir_for(
                        project_dir,
                        &format!("__bakeoff_{schema}"),
                    ));
                }
            }
            bail!(
                "bakeoff equivalence check failed for cell (on={}, columns={:?}): \
                 '{}' and '{}' disagree ({forward} rows only in '{}', {backward} rows only in \
                 '{}') — refusing to report a cost for a technique whose output diverged",
                cell.on,
                cell.columns,
                technique_slug(measurements[0].technique),
                technique_slug(measurements[1].technique),
                left,
                right,
            );
        }

        cell_reports.push(BakeoffCellReport {
            trigger_label: cell.trigger_label.clone(),
            on: cell.on.clone(),
            columns: cell.columns.clone(),
            techniques: measurements,
            equivalence_checked: true,
        });
    }

    // ── Cleanup (unless --keep). ──
    let kept_schemas = if opts.keep {
        created_schemas.clone()
    } else {
        let conn = duckdb_probe::open(&database_path)?;
        for schema in &created_schemas {
            let _ = duckdb_probe::drop_schema(&conn, schema);
            let _ =
                std::fs::remove_dir_all(state_dir_for(project_dir, &format!("__bakeoff_{schema}")));
        }
        Vec::new()
    };

    let pin = if opts.pin {
        let model_has_maintenance_block = model
            .metadata
            .as_deref()
            .and_then(|m| m.maintenance.as_ref())
            .is_some();
        Some(build_pin_suggestion(
            model_name,
            &cell_reports,
            model_has_maintenance_block,
        )?)
    } else {
        None
    };

    Ok(BakeoffReport {
        model: model_name.to_string(),
        target: opts.target,
        runs: opts.runs,
        cells: cell_reports,
        message: None,
        kept_schemas,
        pin,
    })
}

#[cfg(not(feature = "duckdb"))]
pub async fn run_bakeoff(
    _project_dir: &Path,
    _config: Arc<Config>,
    _model_name: &str,
    _opts: BakeoffOptions,
) -> Result<BakeoffReport> {
    bail!("smelt bakeoff requires the `duckdb` feature (v0.5 scope is duckdb-only)")
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn slice_windows_covers_whole_extent_contiguously() {
        let windows = slice_windows("2025-01-01", "2025-01-09", 3).unwrap();
        assert_eq!(windows.first().unwrap().0, "2025-01-01");
        assert_eq!(windows.last().unwrap().1, "2025-01-10");
        for pair in windows.windows(2) {
            assert_eq!(pair[0].1, pair[1].0, "windows must be contiguous");
        }
    }

    #[test]
    fn scratch_schema_name_disambiguates_multi_cell() {
        let single = scratch_schema_name("m", 0, 1, CellTechnique::Recompute);
        assert_eq!(single, "smelt_bakeoff_m_recompute");
        let multi = scratch_schema_name("m", 1, 2, CellTechnique::Recompute);
        assert_eq!(multi, "smelt_bakeoff_m_c1_recompute");
    }
}
