//! The succession grain's `smelt explain` rendering (`docs/specs/cli.md`
//! §"Succession grain") — split out of `explain.rs` proper once its own
//! addition crossed this crate's large-file baseline
//! (`docs/outcomes/20260906-scd2-keyed-succession/phases/08-plan.md`).

use serde::Serialize;
use smelt_core::sources::SourceInfo;
use smelt_logical::maintenance::succession::SuccessionRecipe;
use smelt_runtime::maintenance_driver::{resolve_succession_run_axis, SuccessionPartitioning};

/// The succession grain's `smelt explain` rendering (`docs/specs/cli.md`
/// §"Succession grain"): built once, from the plan's own `SuccessionRecipe`,
/// its resolved run axis, and the model's own db name — `render_text_lines`
/// and `to_json` both read these SAME fields, so the text and `--json`
/// surfaces cannot drift.
#[derive(Debug, Clone)]
pub struct SuccessionExplainView {
    pub key_columns: Vec<String>,
    pub clock_column: String,
    /// `None` when the recipe's driving source cannot be resolved (mirrors
    /// [`resolve_succession_run_axis`]'s own refusal-by-omission) — the
    /// `run axis:`/`partitioning` lines are omitted rather than fabricated.
    pub run_axis: Option<smelt_runtime::maintenance_driver::SuccessionAxis>,
    pub lead_columns: Vec<String>,
    pub lag_columns: Vec<String>,
    pub delete_flag: Option<String>,
    pub pre_window_filter: Option<String>,
    pub tombstone_table: String,
    pub tombstone_columns: Vec<String>,
}

/// Build the succession explain view from the plan's own recipe (`docs/
/// outcomes/20260906-scd2-keyed-succession/phases/05a-plan.md`) — no
/// re-derivation of anything the recipe or the source declarations don't
/// already carry (`CLAUDE.md` §"Maintenance-plan purity"). `model_db_name`
/// is the model's own bare db name (`ModelFile::db_name_owned()`), matching
/// `smelt_logical::maintenance::emit::tombstone_table_name`'s
/// own `<table>__tombstones` convention (never schema-qualified in the
/// report).
pub fn build_succession_explain_view(
    recipe: &SuccessionRecipe,
    source_infos: &[SourceInfo],
    model_db_name: &str,
) -> SuccessionExplainView {
    let run_axis = resolve_succession_run_axis(recipe, source_infos);
    let tombstone_table = smelt_logical::maintenance::emit::tombstone_table_name(model_db_name);
    let mut tombstone_columns = recipe.key_cols.clone();
    tombstone_columns.push(recipe.clock_col.clone());
    SuccessionExplainView {
        key_columns: recipe.key_cols.clone(),
        clock_column: recipe.clock_col.clone(),
        run_axis,
        lead_columns: recipe
            .lead_derived
            .iter()
            .map(|(alias, _)| alias.clone())
            .collect(),
        lag_columns: recipe
            .lag_derived
            .iter()
            .map(|(alias, _)| alias.clone())
            .collect(),
        delete_flag: recipe.delete_flag_expr.clone(),
        pre_window_filter: recipe.pre_filter.clone(),
        tombstone_table,
        tombstone_columns,
    }
}

impl SuccessionExplainView {
    /// Render the up-to-eight succession lines, in `docs/specs/cli.md`
    /// §"Succession grain" order, omitting the two valueless-capable lines
    /// (`run axis:`, `pre-window filter:`) rather than printing a blank.
    pub fn render_text_lines(&self) -> Vec<String> {
        let mut lines = vec![
            "grain: succession".to_string(),
            format!(
                "identity: ({}, {})",
                self.key_columns.join(", "),
                self.clock_column
            ),
            "technique: succession-patch".to_string(),
        ];
        if let Some(axis) = &self.run_axis {
            let kind = match axis.partitioning {
                SuccessionPartitioning::Arrival => "arrival-partitioned",
                SuccessionPartitioning::EventTime => "event-time-partitioned",
            };
            lines.push(format!("run axis: {} ({kind})", axis.column));
        }
        lines.push(format!("clock: {}", self.clock_column));
        lines.push("posture: re-run tolerant; order-independent but serial".to_string());
        if let Some(filter) = &self.pre_window_filter {
            lines.push(format!("pre-window filter: {filter}"));
        }
        lines.push(format!(
            "internal state: tombstone ledger {} ({}, {}) — not part of the model's public \
             schema",
            self.tombstone_table,
            self.key_columns.join(", "),
            self.clock_column
        ));
        lines
    }

    /// The `--json` `succession` object (`docs/specs/cli.md` §"Succession
    /// grain"): the fixed postures come straight from
    /// `smelt_logical::maintenance::succession::SUCCESSION_POSTURES`, never
    /// re-derived per model.
    pub fn to_json(&self) -> SuccessionJson {
        let (run_axis, partitioning) = match &self.run_axis {
            Some(axis) => (
                Some(axis.column.clone()),
                Some(
                    match axis.partitioning {
                        SuccessionPartitioning::Arrival => "arrival",
                        SuccessionPartitioning::EventTime => "event_time",
                    }
                    .to_string(),
                ),
            ),
            None => (None, None),
        };
        let postures = smelt_logical::maintenance::succession::SUCCESSION_POSTURES;
        SuccessionJson {
            key_columns: self.key_columns.clone(),
            clock_column: self.clock_column.clone(),
            run_axis,
            partitioning,
            lead_columns: self.lead_columns.clone(),
            lag_columns: self.lag_columns.clone(),
            delete_flag: self.delete_flag.clone(),
            pre_window_filter: self.pre_window_filter.clone(),
            tombstone_ledger: SuccessionTombstoneLedgerJson {
                table: self.tombstone_table.clone(),
                columns: self.tombstone_columns.clone(),
            },
            rerun_tolerant: postures.rerun_tolerant,
            order_independent: postures.order_independent,
            concurrent: postures.concurrent,
        }
    }
}

/// JSON shape of `smelt explain --json`'s per-model `succession` object
/// (`docs/specs/cli.md` §"Succession grain"), absent entirely for a
/// non-succession model — never `null`.
#[derive(Debug, Serialize)]
pub struct SuccessionJson {
    pub key_columns: Vec<String>,
    pub clock_column: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_axis: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partitioning: Option<String>,
    pub lead_columns: Vec<String>,
    pub lag_columns: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete_flag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_window_filter: Option<String>,
    pub tombstone_ledger: SuccessionTombstoneLedgerJson,
    pub rerun_tolerant: bool,
    pub order_independent: bool,
    pub concurrent: bool,
}

/// JSON shape of the succession model's tombstone ledger (`smelt explain
/// --json`): `{"table": "...", "columns": [...]}`.
#[derive(Debug, Serialize)]
pub struct SuccessionTombstoneLedgerJson {
    pub table: String,
    pub columns: Vec<String>,
}

/// Build the `keyed_succession` [`super::DeltaSignatureHeadline`] — takes
/// precedence over the model's own `OutputDelta` derivation entirely
/// (`docs/specs/cli.md` §"Delta-signature headline").
pub fn succession_delta_signature(
    view: &SuccessionExplainView,
    grain: Option<String>,
) -> super::DeltaSignatureHeadline {
    super::DeltaSignatureHeadline {
        shape: "keyed_succession".to_string(),
        addressing: "event".to_string(),
        keys: Some(view.key_columns.clone()),
        axis: view.run_axis.as_ref().map(|a| a.column.clone()),
        degraded_by: None,
        slice_bound: None,
        settle_bound: None,
        grain,
        clock: Some(view.clock_column.clone()),
    }
}

/// Render the `keyed_succession` shape's `emits:` clause (`docs/specs/
/// cli.md` §"Delta-signature headline").
pub fn render_keyed_succession_emits(keys: &str, clock: &str) -> String {
    format!("event history keyed by [{keys}], event-addressed by ({keys}, {clock})")
}
