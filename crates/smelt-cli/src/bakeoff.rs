//! `smelt bakeoff` measurement engine (`docs/plans/20260719-prod-w7-bakeoff.md`
//! Phase 4; `incremental_models.md` §"CLI" bakeoff bullet).
//!
//! For a maintained model, measures every admissible technique of every
//! selected cell (default: every cell with 2+ admissible techniques — see
//! `smelt_logical::maintenance::choice`'s resolvable-set doc comment) over
//! `--runs` sequential replayed windows of the project's own data. Each
//! technique is forced through the Phase-3 `ExecuteRequest::technique_overrides`
//! seam into a disposable scratch schema (decision B1: the target is cloned
//! in-memory under a synthetic name, only `schema` changed — no runtime
//! schema seam, everything still goes through `execute_project`, preserving
//! the run-pipeline-parity invariant). After the replay, every pair of
//! measured variants is cross-checked with `EXCEPT ALL` in both directions —
//! a mismatch fails the whole run loudly rather than reporting a cost for a
//! technique whose output diverged.
//!
//! `mod commands::bakeoff` (the binary's thin arg-parsing shim, per the
//! `explain.rs` precedent, decision B3) is the only intended caller of
//! [`run_bakeoff`]; tests drive it directly in-process.
//!
//! v0.5 scope: DuckDB targets only (`Explicitly deferred` in the plan —
//! Spark bakeoff inherits whatever W4 concludes). A non-DuckDB target fails
//! loud rather than silently no-op'ing.

use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use tokio_util::sync::CancellationToken;

use smelt_core::config::{CellTechnique, Config, MaintenanceCellConfig, MaintenanceConfig};
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_logical::maintenance::{ColumnGroup, PlanCell, Technique, Trigger};
use smelt_runtime::types::{CellTechniqueOverride, ExecuteRequest};
use smelt_runtime::{execute_project, NoOpReporter};

use crate::backend_factory::CliBackendFactory;
use crate::init_db;

/// One `--cells <col>@<source>` selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellSelector {
    pub column: String,
    pub source: String,
}

impl CellSelector {
    /// Parse a single `<col>@<source>` token. `--cells` accepts a
    /// comma-separated list at the arg-parsing layer; this parses one
    /// already-split entry.
    pub fn parse(raw: &str) -> Result<Self> {
        let (column, source) = raw
            .split_once('@')
            .with_context(|| format!("invalid --cells entry '{raw}': expected <col>@<source>"))?;
        if column.trim().is_empty() || source.trim().is_empty() {
            bail!("invalid --cells entry '{raw}': expected <col>@<source>");
        }
        Ok(CellSelector {
            column: column.trim().to_string(),
            source: source.trim().to_string(),
        })
    }
}

/// Options for one `smelt bakeoff` invocation (`incremental_models.md`
/// §"CLI" bakeoff flags).
#[derive(Debug, Clone)]
pub struct BakeoffOptions {
    /// Empty = every cell with 2+ admissible techniques (the default).
    pub cells: Vec<CellSelector>,
    pub runs: u32,
    pub target: String,
    pub keep: bool,
    /// Emit the winning technique per measured cell as ready-to-paste YAML
    /// (`incremental_models.md` §"CLI": "never rewrites the model's `.sql`
    /// file"). Populates [`BakeoffReport::pin`]; never touches disk.
    pub pin: bool,
}

impl Default for BakeoffOptions {
    fn default() -> Self {
        BakeoffOptions {
            cells: Vec::new(),
            runs: 3,
            target: "dev".to_string(),
            keep: false,
            pin: false,
        }
    }
}

/// One technique's measured cost for one cell.
#[derive(Debug, Clone)]
pub struct TechniqueMeasurement {
    pub technique: CellTechnique,
    /// Wall-clock milliseconds for each of the `--runs` replayed windows, in
    /// order.
    pub run_wall_clock_ms: Vec<u128>,
    /// Row count of the maintained table after the last replayed window.
    pub row_count: i64,
    pub scratch_schema: String,
}

impl TechniqueMeasurement {
    pub fn total_wall_clock_ms(&self) -> u128 {
        self.run_wall_clock_ms.iter().sum()
    }
}

/// The measured report for one bakeoff cell (one trigger × column group).
#[derive(Debug, Clone)]
pub struct BakeoffCellReport {
    pub trigger_label: String,
    pub on: String,
    pub columns: Vec<String>,
    pub techniques: Vec<TechniqueMeasurement>,
    /// `true` when every measured pair of variants agreed exactly under
    /// `EXCEPT ALL` in both directions — always `true` on a returned report,
    /// since a disagreement fails the whole bakeoff loudly instead
    /// (`incremental_models.md` §"CLI": "failing loud on a mismatch rather
    /// than reporting a cost for a technique whose output diverged").
    pub equivalence_checked: bool,
}

/// The full report for one `smelt bakeoff <model>` invocation.
#[derive(Debug, Clone)]
pub struct BakeoffReport {
    pub model: String,
    pub target: String,
    pub runs: u32,
    pub cells: Vec<BakeoffCellReport>,
    /// Set instead of `cells` being populated when there was nothing to
    /// measure (no cell — or no *selected* cell — has 2+ admissible
    /// techniques).
    pub message: Option<String>,
    /// Scratch schema names retained on disk (`--keep`); empty otherwise.
    pub kept_schemas: Vec<String>,
    /// Set when `--pin` was requested and there was at least one measured
    /// cell to pin. `None` when `--pin` was not requested, or when the run
    /// had nothing to measure (`message` is set instead).
    pub pin: Option<PinSuggestion>,
}

/// The ready-to-paste YAML `--pin` prints alongside the report
/// (`incremental_models.md` §"CLI": "emits the winning `cells[]` entry (or a
/// complete `maintenance:` block when the model has none) as ready-to-paste
/// YAML on stdout — it never rewrites the model's `.sql` file"). Emit-only:
/// nothing here ever touches disk — this is pure formatted data the caller
/// prints.
#[derive(Debug, Clone)]
pub struct PinSuggestion {
    /// "to pin this choice, add to `<model>.sql` frontmatter:" — printed
    /// immediately above `yaml`.
    pub header: String,
    /// The emitted YAML itself: a bare `cells[]` sequence when the model
    /// already declares a `maintenance:` block, or a complete `maintenance:`
    /// block otherwise. Deserializes via the same `Serialize`/`Deserialize`
    /// derives `MaintenanceCellConfig`/`MaintenanceConfig` use for parsing
    /// real frontmatter — a genuine round-trip, not a hand-formatted string.
    pub yaml: String,
    /// Human-readable labels (`columns=[...] on=...`) of cells whose
    /// measured techniques tied on total wall-clock — the pin keeps that
    /// cell's current default choice in that case, called out explicitly
    /// here rather than silently.
    pub tied_cells: Vec<String>,
}

const REPORT_HEADER: &str = "smelt bakeoff report";

impl fmt::Display for BakeoffReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{REPORT_HEADER} for `{}` (target={}, runs={})",
            self.model, self.target, self.runs
        )?;
        if let Some(msg) = &self.message {
            writeln!(f, "{msg}")?;
            return Ok(());
        }
        for cell in &self.cells {
            writeln!(
                f,
                "\ncell: columns={:?} on={} trigger={}",
                cell.columns, cell.on, cell.trigger_label
            )?;
            for m in &cell.techniques {
                writeln!(
                    f,
                    "  - {:<16} total={:>6}ms per-run={:?} rows={} schema={}",
                    technique_slug(m.technique),
                    m.total_wall_clock_ms(),
                    m.run_wall_clock_ms,
                    m.row_count,
                    m.scratch_schema,
                )?;
            }
            writeln!(
                f,
                "  equivalence: {}",
                if cell.equivalence_checked {
                    "OK (EXCEPT ALL empty both directions)"
                } else {
                    "not checked"
                }
            )?;
        }
        if !self.kept_schemas.is_empty() {
            writeln!(
                f,
                "\nkept scratch schemas: {}",
                self.kept_schemas.join(", ")
            )?;
        }
        if let Some(pin) = &self.pin {
            writeln!(f, "\n{}", pin.header)?;
            write!(f, "{}", pin.yaml)?;
            if !pin.tied_cells.is_empty() {
                writeln!(
                    f,
                    "\nnote: total wall-clock tied across all measured techniques for the \
                     following cell(s) — the pin above keeps the current default choice: {}",
                    pin.tied_cells.join("; ")
                )?;
            }
        }
        Ok(())
    }
}

/// Lowercase, spec-facing name for a `CellTechnique` — used both in the
/// report and in the scratch-schema name (`incremental_models.md` §"CLI":
/// schema `smelt_bakeoff_<model>_<technique>`).
fn technique_slug(t: CellTechnique) -> &'static str {
    match t {
        CellTechnique::Fold => "fold",
        CellTechnique::Recompute => "recompute",
        CellTechnique::RederiveColumns => "rederive_columns",
        CellTechnique::Suppress => "suppress",
        CellTechnique::Unconditional => "unconditional",
    }
}

/// The trigger's `on:` address — mirrors
/// `smelt_db::queries::maintenance::trigger_on_address` (private to that
/// crate); reimplemented here as the same trivial, pure classification over
/// `Trigger`, not a re-derivation of any admission/grouping logic
/// (`CLAUDE.md` §"Maintenance-plan purity" governs the *plan*, not this
/// display-label lookup).
fn trigger_on_address(trigger: &Trigger) -> Option<String> {
    match trigger {
        Trigger::NewData { source } | Trigger::UpstreamMutation { source } => Some(source.clone()),
        Trigger::Backfill => Some("backfill".to_string()),
        Trigger::ColumnAdded { .. } => None,
    }
}

/// One candidate cell derived from the plan: its address (`on` + columns)
/// and the two-member resolvable technique set
/// (`smelt_logical::maintenance::choice`'s module doc: "the cell's own
/// admitted technique, RegionRecompute").
struct BakeoffCell {
    trigger_label: String,
    on: String,
    columns: Vec<String>,
    admitted: CellTechnique,
}

/// Map a `PlanCell`'s admitted `Technique` to the `CellTechnique` family a
/// `technique_overrides` entry pins — `None` when the cell only ever admits
/// `DeleteInsert` (recompute), i.e. it has exactly one resolvable technique
/// and is not a bakeoff candidate.
fn admitted_family(technique: &Technique) -> Option<CellTechnique> {
    match technique {
        Technique::KeyedFold | Technique::InPlaceUpdate => Some(CellTechnique::Fold),
        Technique::ColumnScopedMerge => Some(CellTechnique::RederiveColumns),
        // No cell derives this technique yet (`smelt_logical::maintenance::
        // repair::derive_repair_cell` is standalone, not yet wired into
        // `derive_maintenance_plan`), and it has no `CellTechnique` family
        // pin of its own — mirrors `DeleteInsert`'s own "not a bakeoff
        // candidate" verdict.
        Technique::DeleteInsert | Technique::PerGroupRecompute => None,
    }
}

/// Derive the candidate cell list from the plan + column groups, applying
/// `--cells` narrowing when non-empty.
fn candidate_cells(
    cells: &[PlanCell],
    column_groups: &[ColumnGroup],
    selectors: &[CellSelector],
) -> Result<Vec<BakeoffCell>> {
    let mut out = Vec::new();
    for cell in cells {
        let Some(on) = trigger_on_address(&cell.trigger) else {
            continue;
        };
        let Some(admitted) = admitted_family(&cell.technique) else {
            continue;
        };
        let columns: Vec<String> = column_groups
            .iter()
            .find(|g| g.name() == cell.group)
            .map(|g| g.columns.clone())
            .unwrap_or_default();

        if !selectors.is_empty() {
            let matches = selectors
                .iter()
                .any(|s| s.source == on && columns.iter().any(|c| c == &s.column));
            if !matches {
                continue;
            }
        }

        out.push(BakeoffCell {
            trigger_label: format!("{:?}", cell.trigger),
            on,
            columns,
            admitted,
        });
    }

    // A named `--cells` selector that matched nothing at all (as opposed to
    // matching a single-technique cell, which never reaches here because
    // `admitted_family` already excluded it) is a plain "did you mean"
    // usage error — named loud rather than silently measuring nothing.
    for s in selectors {
        let named_cell_exists = cells.iter().any(|cell| {
            trigger_on_address(&cell.trigger).as_deref() == Some(s.source.as_str())
                && column_groups
                    .iter()
                    .find(|g| g.name() == cell.group)
                    .is_some_and(|g| g.columns.iter().any(|c| c == &s.column))
        });
        let selected = out
            .iter()
            .any(|c| c.on == s.source && c.columns.iter().any(|col| col == &s.column));
        if !selected && !named_cell_exists {
            bail!(
                "--cells '{}@{}' does not name any cell of this model's maintenance plan",
                s.column,
                s.source
            );
        }
    }

    Ok(out)
}

/// Build the Salsa DB + dependency graph for one `execute_project` call.
/// Rebuilt per call (mirrors `smelt-maintenance-testkit`'s `LinkCProject`)
/// so a between-window mutation on disk is picked up.
type SharedDb = Arc<tokio::sync::Mutex<smelt_db::Database>>;
type SharedGraph = Arc<tokio::sync::Mutex<DependencyGraph>>;

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

fn state_dir_for(project_dir: &Path, target: &str) -> std::path::PathBuf {
    project_dir.join(".smelt").join("targets").join(target)
}

#[cfg(feature = "duckdb")]
mod duckdb_probe {
    use anyhow::{Context, Result};
    use std::path::Path;

    pub fn open(database_path: &Path) -> Result<duckdb::Connection> {
        duckdb::Connection::open(database_path)
            .with_context(|| format!("opening duckdb database at {}", database_path.display()))
    }

    /// `[start, end]` inclusive date extent (`YYYY-MM-DD`) of
    /// `event_time_column` in the driving source's own physical table
    /// (`{schema}.{table}`).
    pub fn event_time_extent(
        conn: &duckdb::Connection,
        schema: &str,
        table: &str,
        event_time_column: &str,
    ) -> Result<Option<(String, String)>> {
        let query = format!(
            "SELECT CAST(MIN(CAST({event_time_column} AS DATE)) AS VARCHAR), \
             CAST(MAX(CAST({event_time_column} AS DATE)) AS VARCHAR) \
             FROM {schema}.{table}"
        );
        let row: (Option<String>, Option<String>) = conn
            .query_row(&query, [], |r| Ok((r.get(0)?, r.get(1)?)))
            .with_context(|| format!("computing event-time extent of {schema}.{table}"))?;
        Ok(match row {
            (Some(lo), Some(hi)) => Some((lo, hi)),
            _ => None,
        })
    }

    pub fn row_count(conn: &duckdb::Connection, schema: &str, table: &str) -> Result<i64> {
        conn.query_row(&format!("SELECT count(*) FROM {schema}.{table}"), [], |r| {
            r.get(0)
        })
        .with_context(|| format!("counting rows in {schema}.{table}"))
    }

    /// Rows in `left` not present in `right` (multiset semantics) —
    /// `EXCEPT ALL`, one direction. Zero in both directions is the
    /// equivalence proof; non-zero in either fails the bakeoff loudly.
    pub fn except_all_count(
        conn: &duckdb::Connection,
        left_schema: &str,
        right_schema: &str,
        table: &str,
    ) -> Result<i64> {
        conn.query_row(
            &format!(
                "SELECT count(*) FROM (SELECT * FROM {left_schema}.{table} EXCEPT ALL \
                 SELECT * FROM {right_schema}.{table})"
            ),
            [],
            |r| r.get(0),
        )
        .with_context(|| format!("EXCEPT ALL {left_schema}.{table} vs {right_schema}.{table}"))
    }

    pub fn drop_schema(conn: &duckdb::Connection, schema: &str) -> Result<()> {
        conn.execute_batch(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
            .with_context(|| format!("dropping scratch schema {schema}"))
    }

    /// Make every declared source the model reads visible in the scratch
    /// schema without copying data: a same-database view over the real
    /// schema's physical source table. `execute_project` still only ever
    /// *writes* the maintained model's own output into the scratch schema —
    /// sources are read-only from the run's perspective, so a view is
    /// equivalent to a copy for measurement purposes and orders of
    /// magnitude cheaper.
    pub fn ensure_scratch_source_views(
        conn: &duckdb::Connection,
        scratch_schema: &str,
        real_schema: &str,
        source_tables: &[String],
    ) -> Result<()> {
        conn.execute_batch(&format!("CREATE SCHEMA IF NOT EXISTS {scratch_schema}"))
            .with_context(|| format!("creating scratch schema {scratch_schema}"))?;
        for table in source_tables {
            conn.execute_batch(&format!(
                "CREATE OR REPLACE VIEW {scratch_schema}.{table} AS SELECT * FROM {real_schema}.{table}"
            ))
            .with_context(|| format!("creating scratch source view {scratch_schema}.{table}"))?;
        }
        Ok(())
    }
}

/// Split `[start, end]` (inclusive dates, `YYYY-MM-DD`) into `runs`
/// sequential, contiguous, non-overlapping `[window_start, window_end)`
/// windows (`window_end` exclusive, matching `ExecuteRequest::start`/`end`
/// convention) covering the whole extent — decision B4's window slicer.
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
    // "duckdb" is not a placeholder here — `smelt bakeoff` fails loud below
    // (§2) on any non-duckdb target, so this is the real backend.
    let result =
        smelt_db::maintenance_plan_report(&plan_db, ws, file, "duckdb").with_context(|| {
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
                    keyed_restrictions: std::collections::BTreeMap::new(),
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

/// The winning technique for one measured cell: lowest total wall-clock
/// across the replayed windows. A tie keeps the cell's current default
/// choice — `techniques[0]`, always the cell's own admitted technique, per
/// `run_bakeoff`'s `variants = [cell.admitted, CellTechnique::Recompute]`
/// ordering — and is reported as such (the second element is `true` on a
/// tie).
fn cell_winner(cell: &BakeoffCellReport) -> (CellTechnique, bool) {
    let default_technique = cell.techniques[0].technique;
    let min_cost = cell
        .techniques
        .iter()
        .map(TechniqueMeasurement::total_wall_clock_ms)
        .min()
        .unwrap_or(0);
    let winners: Vec<CellTechnique> = cell
        .techniques
        .iter()
        .filter(|m| m.total_wall_clock_ms() == min_cost)
        .map(|m| m.technique)
        .collect();
    match winners.as_slice() {
        [only] => (*only, false),
        _ => (default_technique, true),
    }
}

/// Build the `--pin` suggestion: one `MaintenanceCellConfig` per measured
/// cell, naming its winning technique, serialized via the same
/// `Serialize`/`Deserialize` derives real `maintenance:` frontmatter parses
/// through (`incremental_models.md` §"CLI"). `model_has_maintenance_block`
/// selects the shape: a bare `cells[]` sequence to append to an existing
/// block, or a complete `maintenance:` block when the model declares none.
fn build_pin_suggestion(
    model_name: &str,
    cells: &[BakeoffCellReport],
    model_has_maintenance_block: bool,
) -> Result<PinSuggestion> {
    let mut cell_configs = Vec::with_capacity(cells.len());
    let mut tied_cells = Vec::new();
    for cell in cells {
        let (winner, is_tie) = cell_winner(cell);
        if is_tie {
            tied_cells.push(format!("columns={:?} on={}", cell.columns, cell.on));
        }
        cell_configs.push(MaintenanceCellConfig {
            columns: cell.columns.clone(),
            on: cell.on.clone(),
            prefer: None,
            technique: Some(winner),
            write: None,
        });
    }

    let yaml = if model_has_maintenance_block {
        serde_yaml::to_string(&cell_configs).context("serializing bakeoff pin cells[] fragment")?
    } else {
        let config = MaintenanceConfig {
            defaults: None,
            cells: cell_configs,
            scan_bounds: None,
        };
        let body =
            serde_yaml::to_string(&config).context("serializing bakeoff pin maintenance: block")?;
        let indented = body
            .lines()
            .map(|line| {
                if line.is_empty() {
                    String::new()
                } else {
                    format!("  {line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("maintenance:\n{indented}\n")
    };

    Ok(PinSuggestion {
        header: format!("to pin this choice, add to {model_name}.sql frontmatter:"),
        yaml,
        tied_cells,
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
    fn cell_selector_parses_col_at_source() {
        let s = CellSelector::parse("user_name@users").unwrap();
        assert_eq!(s.column, "user_name");
        assert_eq!(s.source, "users");
    }

    #[test]
    fn cell_selector_rejects_missing_at() {
        assert!(CellSelector::parse("user_name").is_err());
    }

    #[test]
    fn technique_slug_matches_spec_naming() {
        assert_eq!(technique_slug(CellTechnique::Fold), "fold");
        assert_eq!(technique_slug(CellTechnique::Recompute), "recompute");
        assert_eq!(
            technique_slug(CellTechnique::RederiveColumns),
            "rederive_columns"
        );
    }

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
