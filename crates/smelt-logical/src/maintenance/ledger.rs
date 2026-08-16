//! The per-column guarantee ledger (`docs/specs/incremental_models.md`
//! §Surface "CLI" — "the printed summary of what each output column is
//! guaranteed") and the pre-execution refusal summary: pure formatters over
//! already-derived data, matching `signature.rs`'s single-owner precedent so
//! `smelt explain`'s text and `--json` renderings can never drift.
//!
//! **The ledger row.** One row per output column, carrying the column
//! group that owns it, its effective equivalence contract (or, for a
//! volatile column, its determinism exemption in place of that contract —
//! `incremental_models.md` §"The equivalence invariant" "The determinism
//! scope"), and the derived settle bound. Never fabricates: a column with
//! no established key-temporal-locality slice prints [`SettleLabel::NotDerived`]
//! rather than a zero or sentinel interval.
//!
//! **The refusal summary.** [`render_refusal`] renders one
//! [`super::Refusal`] as `<code>: <reason>` — the single formatter both the
//! text report's pre-execution refusal block and `--json`'s `refusals`
//! array read, so a refusal is never rendered via `{:?}` of the enum.

use crate::analysis::walk::{ColumnDeterminism, Determinism};
use crate::maintenance::locality::SettleBound;
use crate::maintenance::{ColumnGroup, KeyLocality, Refusal};
use smelt_core::config::ContractConfig;

/// How long a ledger row's column may still change before it is safe to
/// treat as final — [`SettleBound`] widened with a "not derived" case for a
/// model/group with no established key-temporal-locality slice at all
/// (`incremental_shapes.md` §"Key temporal locality"). Never a fabricated or
/// zero interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettleLabel {
    After {
        margin: crate::analysis::source_bounds::Seconds,
    },
    Never,
    AfterRecurrenceBound {
        r: crate::analysis::source_bounds::Seconds,
        margin: crate::analysis::source_bounds::Seconds,
    },
    /// No [`KeyLocality`] was established for this model (a `grain:
    /// partition` model, or a `grain: key` model with no `timeseries:`
    /// block) — there is nothing to derive a settle bound from.
    NotDerived,
}

impl SettleLabel {
    /// Widen an admitted [`SettleBound`], or `None` (no locality
    /// established), into a [`SettleLabel`] — never re-derives the bound
    /// itself, only restates [`locality::settle_bound`](super::locality::settle_bound)'s
    /// already-computed verdict.
    pub fn from_settle_bound(bound: Option<&SettleBound>) -> Self {
        match bound {
            None => SettleLabel::NotDerived,
            Some(SettleBound::After { margin }) => SettleLabel::After { margin: *margin },
            Some(SettleBound::Never) => SettleLabel::Never,
            Some(SettleBound::AfterRecurrenceBound { r, margin }) => {
                SettleLabel::AfterRecurrenceBound {
                    r: *r,
                    margin: *margin,
                }
            }
        }
    }

    /// The `settle:` clause's rendered value, shared by the text and JSON
    /// surfaces.
    pub fn render(&self) -> String {
        match self {
            SettleLabel::After { margin } => format!("after {}s", margin.0),
            SettleLabel::Never => "never".to_string(),
            SettleLabel::AfterRecurrenceBound { r, margin } => {
                format!("after {}s (recurrence bound) + {}s margin", r.0, margin.0)
            }
            SettleLabel::NotDerived => "not derived".to_string(),
        }
    }
}

/// What a ledger row's `guarantee` column names: the group's effective
/// equivalence contract point, or — for a column the determinism verdict
/// marks `Run`/`Row` — its determinism exemption instead
/// (`incremental_models.md` §"The determinism scope": "the conformance
/// oracle's comparison exempts those columns, and `smelt explain`'s
/// per-column guarantee ledger prints the exemption in place of an
/// equivalence contract").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColumnGuarantee {
    /// `smelt_logical::contract::EffectiveContract::render_label`'s own
    /// text, verbatim.
    Contract(String),
    /// The column's determinism exemption, naming which level exempted it.
    DeterminismExemption(String),
}

impl ColumnGuarantee {
    pub fn render(&self) -> &str {
        match self {
            ColumnGuarantee::Contract(label) => label,
            ColumnGuarantee::DeterminismExemption(label) => label,
        }
    }
}

/// One row of the per-column guarantee ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuaranteeRow {
    pub column: String,
    /// The owning column group's display name (`ColumnGroup::name`).
    pub group: String,
    pub guarantee: ColumnGuarantee,
    pub settle: SettleLabel,
}

/// A `Run`/`Row`-determinism column's exemption label. `Clean` never reaches
/// this function — callers only invoke it once a column's determinism level
/// is known non-`Clean`.
fn determinism_exemption_label(level: Determinism) -> String {
    match level {
        Determinism::Run => {
            "run-nondeterministic — exempt from the equivalence contract (one value per run)"
                .to_string()
        }
        Determinism::Row => {
            "row-nondeterministic — exempt from the equivalence contract (runs as-is, never pinned)"
                .to_string()
        }
        Determinism::Clean => String::new(),
    }
}

/// Derive the per-column guarantee ledger: one row per output column across
/// every group in `column_groups`.
///
/// A group's effective contract is resolved via
/// [`crate::contract::effective_contract`] against the group's own first
/// (lexicographically least) `mutation_sensitivity` source — the same
/// address convention [`super::Trigger::NewData`]'s `source` field uses
/// (`ColumnGroup`'s own doc comment: "Name as it appears in ...
/// `ColumnGroup::mutation_sensitivity`" mirrors `SourceFacts::name`, which
/// is what a cell's trigger source is built from). A group triggered by more
/// than one source with differing cell-level `deferral` overrides reports
/// only the first trigger's resolution — `frozen_horizon` and a model-level
/// `deferral` default are trigger-independent, so this only under-reports a
/// per-cell override on a genuinely multi-trigger group, never fabricates
/// one. A group with no mutation-sensitivity source (never mutated after
/// creation) resolves against the empty trigger address, which still
/// applies a model-level `frozen_horizon`/`deferral` default.
///
/// `key_locality` is `None` for a `grain: partition` model or a `grain: key`
/// model with no `timeseries:` block — every row then carries
/// [`SettleLabel::NotDerived`], never a fabricated interval. The bound
/// applies uniformly to every row: [`KeyLocality::settle_bound`] is a
/// model-wide verdict, not derived per column.
pub fn derive_guarantee_ledger(
    column_groups: &[ColumnGroup],
    contract_cfg: Option<&ContractConfig>,
    key_locality: Option<&KeyLocality>,
    determinism: &[ColumnDeterminism],
) -> Vec<GuaranteeRow> {
    let settle = SettleLabel::from_settle_bound(key_locality.map(|kl| &kl.settle_bound));
    let mut rows = Vec::new();
    for group in column_groups {
        let trigger_address = group
            .mutation_sensitivity
            .iter()
            .next()
            .cloned()
            .unwrap_or_default();
        let contract =
            crate::contract::effective_contract(contract_cfg, &trigger_address, &group.columns);
        let contract_label = contract.render_label();
        let group_name = group.name();
        for column in &group.columns {
            let level = determinism
                .iter()
                .find(|d| d.output.eq_ignore_ascii_case(column))
                .map(|d| d.level);
            let guarantee = match level {
                Some(level) if level != Determinism::Clean => {
                    ColumnGuarantee::DeterminismExemption(determinism_exemption_label(level))
                }
                _ => ColumnGuarantee::Contract(contract_label.clone()),
            };
            rows.push(GuaranteeRow {
                column: column.clone(),
                group: group_name.clone(),
                guarantee,
                settle,
            });
        }
    }
    rows
}

/// One rendered refusal: `<code>: <reason>` — never `{:?}` of [`Refusal`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusalSummary {
    pub code: String,
    pub message: String,
}

impl RefusalSummary {
    pub fn render(&self) -> String {
        format!("{}: {}", self.code, self.message)
    }
}

/// Render one [`Refusal`] as its `<DiagnosticCode>: <reason>` summary
/// (`docs/specs/diagnostics.md`'s `Maintenance*`/`KeyedForbidsTimeseries`
/// catalogue rows name the code strings used here). The pre-execution
/// refusal block (`smelt explain`'s text and `--json` surfaces) reads this
/// verbatim — it is the single formatter for a refusal, never a second
/// ad hoc rendering.
pub fn render_refusal(refusal: &Refusal) -> RefusalSummary {
    match refusal {
        Refusal::SkeletonColumnAdded { column } => RefusalSummary {
            code: "MaintenanceSkeletonChanged".to_string(),
            message: format!("column '{column}' was added or changed in a skeleton position"),
        },
        Refusal::ScanUnbounded { source, why } => RefusalSummary {
            code: "MaintenanceScanUnbounded".to_string(),
            message: format!("scan over '{source}' cannot be partition-bounded: {why}"),
        },
        Refusal::NoAdmissibleTechnique { trigger, why } => RefusalSummary {
            code: "MaintenanceNoAdmissibleTechnique".to_string(),
            message: format!("no technique survives admission for trigger '{trigger}': {why}"),
        },
        Refusal::ReachNotDerivable { edge, why } => RefusalSummary {
            code: "MaintenanceReachNotDerivable".to_string(),
            message: format!("event-time clock for edge '{edge}' cannot be derived: {why}"),
        },
        Refusal::UnsupportedGrain {
            grain,
            tracking_plan,
        } => RefusalSummary {
            code: "MaintenanceUnsupportedGrain".to_string(),
            message: format!("grain '{grain}' is not yet supported (tracked in {tracking_plan})"),
        },
        Refusal::LocalityNotEstablished { message } => {
            let reason = message
                .strip_prefix("KeyedForbidsTimeseries: ")
                .unwrap_or(message);
            RefusalSummary {
                code: "KeyedForbidsTimeseries".to_string(),
                message: reason.to_string(),
            }
        }
        Refusal::RepairKeysNotDiscoverable { source, why } => RefusalSummary {
            code: "MaintenanceRepairKeysNotDiscoverable".to_string(),
            message: format!("affected-key discovery for '{source}' failed: {why}"),
        },
        Refusal::RepairSliceUnbounded { source, why } => RefusalSummary {
            code: "MaintenanceRepairSliceUnbounded".to_string(),
            message: format!("per-group read for '{source}' could not be bounded: {why}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::source_bounds::Seconds;
    use crate::maintenance::locality::{LocalitySlice, SettleBound};
    use std::collections::BTreeSet;

    fn group(columns: &[&str], mutation_sensitivity: &[&str]) -> ColumnGroup {
        ColumnGroup {
            columns: columns.iter().map(|c| c.to_string()).collect(),
            mutation_sensitivity: mutation_sensitivity.iter().map(|s| s.to_string()).collect(),
            membership_sensitivity: BTreeSet::new(),
        }
    }

    #[test]
    fn ledger_row_per_output_column_carries_its_group_contract() {
        use smelt_core::config::{ContractCellConfig, DataLatency};

        let group_a = group(&["amount"], &["sources.raw.events"]);
        let group_b = group(&["weight"], &["sources.dims"]);
        let cfg = ContractConfig {
            frozen_horizon: None,
            deferral: DataLatency::parse("6 hours"),
            cells: vec![ContractCellConfig {
                columns: vec!["amount".to_string()],
                on: "sources.raw.events".to_string(),
                deferral: DataLatency::parse("1 day"),
            }],
        };
        let rows = derive_guarantee_ledger(&[group_a, group_b], Some(&cfg), None, &[]);
        assert_eq!(rows.len(), 2);
        let amount = rows.iter().find(|r| r.column == "amount").unwrap();
        let weight = rows.iter().find(|r| r.column == "weight").unwrap();
        assert_eq!(
            amount.guarantee,
            ColumnGuarantee::Contract("deferral 1 day (cell)".to_string())
        );
        assert_eq!(
            weight.guarantee,
            ColumnGuarantee::Contract("deferral 6 hours".to_string())
        );
        assert_ne!(amount.guarantee, weight.guarantee);
    }

    #[test]
    fn ledger_settle_bound_reads_established_locality() {
        let group_a = group(&["order_id"], &["sources.orders"]);
        let route1 = KeyLocality {
            slice: LocalitySlice::Window {
                partition_column: "order_date".to_string(),
                margin_before: Seconds(3600),
                margin_after: Seconds(0),
                recurrence_bounded: false,
            },
            settle_bound: SettleBound::After {
                margin: Seconds(3600),
            },
        };
        let rows =
            derive_guarantee_ledger(std::slice::from_ref(&group_a), None, Some(&route1), &[]);
        assert!(rows.iter().all(|r| r.settle
            == SettleLabel::After {
                margin: Seconds(3600)
            }));

        let route2 = KeyLocality {
            slice: LocalitySlice::DeltaValues {
                partition_column: "order_date".to_string(),
            },
            settle_bound: SettleBound::Never,
        };
        let rows = derive_guarantee_ledger(&[group_a], None, Some(&route2), &[]);
        assert!(rows.iter().all(|r| r.settle == SettleLabel::Never));
    }

    #[test]
    fn ledger_settle_bound_not_derived_without_locality() {
        let group_a = group(&["revenue"], &["sources.orders"]);
        let rows = derive_guarantee_ledger(&[group_a], None, None, &[]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].settle, SettleLabel::NotDerived);
        assert_eq!(rows[0].settle.render(), "not derived");
    }

    #[test]
    fn ledger_volatile_column_prints_determinism_exemption() {
        let group_a = group(&["loaded_at", "amount"], &["sources.orders"]);
        let determinism = vec![
            ColumnDeterminism {
                output: "loaded_at".to_string(),
                level: Determinism::Run,
            },
            ColumnDeterminism {
                output: "amount".to_string(),
                level: Determinism::Clean,
            },
        ];
        let rows = derive_guarantee_ledger(&[group_a], None, None, &determinism);
        let loaded_at = rows.iter().find(|r| r.column == "loaded_at").unwrap();
        let amount = rows.iter().find(|r| r.column == "amount").unwrap();
        assert!(matches!(
            loaded_at.guarantee,
            ColumnGuarantee::DeterminismExemption(_)
        ));
        assert!(matches!(amount.guarantee, ColumnGuarantee::Contract(_)));
        assert_eq!(
            amount.guarantee,
            ColumnGuarantee::Contract("default".to_string())
        );
    }

    #[test]
    fn refusal_summary_names_code_and_reason() {
        let refusal = Refusal::ScanUnbounded {
            source: "sources.raw.events".to_string(),
            why: "no clocked column".to_string(),
        };
        let summary = render_refusal(&refusal);
        assert_eq!(summary.code, "MaintenanceScanUnbounded");
        assert_eq!(
            summary.render(),
            "MaintenanceScanUnbounded: scan over 'sources.raw.events' cannot be \
             partition-bounded: no clocked column"
        );

        let locality_refusal = Refusal::LocalityNotEstablished {
            message: "KeyedForbidsTimeseries: model 'x' declares a `timeseries:` block \
                      but key temporal locality could not be established"
                .to_string(),
        };
        let summary = render_refusal(&locality_refusal);
        assert_eq!(summary.code, "KeyedForbidsTimeseries");
        assert!(!summary.message.starts_with("KeyedForbidsTimeseries"));
    }

    #[test]
    fn skeleton_refusal_names_the_renamed_code() {
        let refusal = Refusal::SkeletonColumnAdded {
            column: "user_id".to_string(),
        };
        let summary = render_refusal(&refusal);
        assert_eq!(summary.code, "MaintenanceSkeletonChanged");
        assert!(summary.render().starts_with("MaintenanceSkeletonChanged: "));
    }
}
