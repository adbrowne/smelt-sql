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
//!
//! Submodules: [`cells`] derives the candidate cell list and the `--pin`
//! suggestion from a maintenance plan; [`duckdb_probe`] wraps the raw DuckDB
//! probes (event-time extent, row counts, `EXCEPT ALL`, scratch-schema
//! source views); [`run`] holds the `run_bakeoff` orchestration itself plus
//! its window-slicing and scratch-schema-naming helpers.

use std::fmt;

use anyhow::{bail, Context, Result};

use smelt_core::config::CellTechnique;

mod cells;
#[cfg(feature = "duckdb")]
mod duckdb_probe;
mod run;

pub use run::run_bakeoff;

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
pub(super) fn technique_slug(t: CellTechnique) -> &'static str {
    match t {
        CellTechnique::Fold => "fold",
        CellTechnique::Recompute => "recompute",
        CellTechnique::RederiveColumns => "rederive_columns",
        CellTechnique::Suppress => "suppress",
        CellTechnique::Unconditional => "unconditional",
    }
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
}
