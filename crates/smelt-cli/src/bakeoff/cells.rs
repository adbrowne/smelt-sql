//! Candidate-cell derivation and the `--pin` YAML suggestion — the pure
//! parts of a bakeoff run that turn a maintenance plan (and its measured
//! results) into what the CLI reports.

use anyhow::{bail, Context, Result};

use smelt_core::config::{CellTechnique, MaintenanceCellConfig, MaintenanceConfig};
use smelt_logical::maintenance::{ColumnGroup, PlanCell, Technique, Trigger};

use super::{BakeoffCellReport, CellSelector, PinSuggestion, TechniqueMeasurement};

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
pub(super) struct BakeoffCell {
    pub(super) trigger_label: String,
    pub(super) on: String,
    pub(super) columns: Vec<String>,
    pub(super) admitted: CellTechnique,
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
        // The succession grain's own technique has no bakeoff family either
        // — a succession cell derives exactly one technique
        // (`Technique::SuccessionPatch`), never a bakeoff-resolvable
        // alternative.
        Technique::DeleteInsert | Technique::PerGroupRecompute | Technique::SuccessionPatch => None,
    }
}

/// Derive the candidate cell list from the plan + column groups, applying
/// `--cells` narrowing when non-empty.
pub(super) fn candidate_cells(
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

/// The winning technique for one measured cell: lowest total wall-clock
/// across the replayed windows. A tie keeps the cell's current default
/// choice — `techniques[0]`, always the cell's own admitted technique, per
/// `run_bakeoff`'s `variants = [cell.admitted, CellTechnique::Recompute]`
/// ordering — and is reported as such (the second element is `true` on a
/// tie).
pub(super) fn cell_winner(cell: &BakeoffCellReport) -> (CellTechnique, bool) {
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
pub(super) fn build_pin_suggestion(
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
