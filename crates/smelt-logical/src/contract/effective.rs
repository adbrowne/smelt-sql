use serde::Serialize;
use smelt_core::config::{ContractCellConfig, ContractConfig, DataLatency, RetainDeparted};

/// Which declaration a cell's effective `deferral` window came from — the
/// narrower-wins ladder [`effective_contract`] resolves, mirroring
/// `maintenance::choice`'s own model-vs-cell override reporting
/// (`docs/outcomes/20260809-contract-lattice-v1/phases/07-plan.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferralOrigin {
    /// The model-level `contract.deferral` default.
    Model,
    /// A `contract.cells[]` entry narrowing the model default for this cell.
    Cell,
}

/// A cell's effective `deferral` window plus which declaration it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveDeferral {
    pub window: DataLatency,
    pub origin: DeferralOrigin,
}

/// The effective contract lattice point(s) that apply to one maintenance
/// cell — default (both fields `None`) or the applicable relaxations with
/// their declared parameters (`docs/specs/incremental_models.md` §"The
/// contract lattice"). `frozen_horizon` is always model-level (it clamps the
/// model's write eligibility, never a single cell's); `deferral` may be
/// narrowed per cell.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectiveContract {
    pub frozen_horizon: Option<DataLatency>,
    pub deferral: Option<EffectiveDeferral>,
    /// `retain_departed` is model-level only, like `frozen_horizon` — it
    /// governs the model's own reconcile write, never a single cell's.
    pub retain_departed: Option<RetainDeparted>,
}

impl EffectiveContract {
    /// True when neither relaxation applies — the default point.
    pub fn is_default(&self) -> bool {
        self.frozen_horizon.is_none() && self.deferral.is_none() && self.retain_departed.is_none()
    }

    /// A one-line description used by both the text and JSON renderings
    /// (`smelt-cli::explain`): `"default"`, or the applicable relaxations
    /// joined by `, `, a cell-level `deferral` labelled `(cell)`.
    pub fn render_label(&self) -> String {
        if self.is_default() {
            return "default".to_string();
        }
        let mut parts = Vec::new();
        if let Some(h) = &self.frozen_horizon {
            parts.push(format!("frozen_horizon {}", h.display));
        }
        if let Some(d) = &self.deferral {
            match d.origin {
                DeferralOrigin::Model => parts.push(format!("deferral {}", d.window.display)),
                DeferralOrigin::Cell => parts.push(format!("deferral {} (cell)", d.window.display)),
            }
        }
        if let Some(r) = &self.retain_departed {
            match r {
                RetainDeparted::Bool(_) => parts.push("retain_departed".to_string()),
                RetainDeparted::Tombstone { tombstone } => {
                    parts.push(format!("retain_departed (tombstone: {tombstone})"))
                }
            }
        }
        parts.join(", ")
    }
}

/// Match one `contract.cells[]` entry against `trigger_address` and
/// `group_columns`, mirroring `maintenance::choice`'s own `matching_cell`
/// addressing semantics ("names any member", not "equals exactly").
fn matching_contract_cell<'a>(
    cells: &'a [ContractCellConfig],
    trigger_address: &str,
    group_columns: &[String],
) -> Option<&'a ContractCellConfig> {
    cells.iter().find(|c| {
        c.on == trigger_address
            && c.columns
                .iter()
                .any(|col| group_columns.iter().any(|g| g == col))
    })
}

/// Resolve the effective contract for one cell: `cfg`'s model-level
/// `frozen_horizon` applies unconditionally (there is no per-cell
/// refinement); `deferral` applies a `contract.cells[]` match if present,
/// else the model-level default, else `None`. `cfg` absent (no `contract:`
/// block) resolves to the default point.
pub fn effective_contract(
    cfg: Option<&ContractConfig>,
    trigger_address: &str,
    group_columns: &[String],
) -> EffectiveContract {
    let Some(cfg) = cfg else {
        return EffectiveContract::default();
    };
    let cell_deferral = matching_contract_cell(&cfg.cells, trigger_address, group_columns)
        .and_then(|c| c.deferral.as_ref());
    let deferral = match cell_deferral {
        Some(window) => Some(EffectiveDeferral {
            window: window.clone(),
            origin: DeferralOrigin::Cell,
        }),
        None => cfg.deferral.as_ref().map(|window| EffectiveDeferral {
            window: window.clone(),
            origin: DeferralOrigin::Model,
        }),
    };
    EffectiveContract {
        frozen_horizon: cfg.frozen_horizon.clone(),
        deferral,
        retain_departed: cfg.retain_departed.clone(),
    }
}

/// The JSON shape of one cell's effective contract lattice point
/// (`docs/specs/incremental_models.md` §"The contract lattice"): absent
/// relaxations are omitted, never rendered as `null`. Moved here, verbatim,
/// from `smelt-cli`'s `ExplainContractPointJson`
/// (`docs/outcomes/20260905-property-diff/phases/02-plan.md` task 5) so the
/// property profile (`docs/specs/property_diff.md` §"The property profile")
/// and the single-version report share one owner and one serde shape —
/// `smelt-cli` keeps the old name as a type alias over this struct, sourced
/// from [`effective_contract`], never re-resolved.
///
/// **Not the same type as [`super::ContractPoint`]** (in the sibling
/// `point` module): `ContractPoint` is the lattice-oracle enum a contract
/// *point* is drawn from (`default`/`frozen_horizon`/`deferral`/
/// `retain_departed` as a closed set of variants, consumed by the
/// conformance oracle transform); `ContractPointView` is the *rendered*
/// per-cell JSON shape one or more of those points project onto (a struct
/// of optional fields, consumed by `smelt explain --json`, the property
/// profile, and their diff). The name collision (`View` vs. bare) is
/// deliberate friction — the two are deliberately distinct, and a future
/// reader should not merge them.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct ContractPointView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frozen_horizon: Option<String>,
    /// `frozen_horizon`'s interval in seconds — machine-comparable so a
    /// `contract_point` diff can decide "widened" from the value rather than
    /// re-parsing [`Self::frozen_horizon`]'s display string
    /// (`docs/specs/property_diff.md` §"The property profile" item 2; the
    /// re-parsing-our-own-output bug class, `CLAUDE.md`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frozen_horizon_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deferral: Option<String>,
    /// `deferral`'s interval in seconds — see [`Self::frozen_horizon_seconds`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deferral_seconds: Option<u64>,
    /// `"model"` or `"cell"` — which declaration `deferral` came from.
    /// Omitted along with `deferral` when no deferral applies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deferral_origin: Option<String>,
    /// `"true"`, or `"tombstone: <column>"` for the tombstone form —
    /// mirrors [`EffectiveContract::render_label`]'s own `retain_departed`
    /// rendering. Fix round 1, F5: this field was missing entirely, so a
    /// `retain_departed` declaration or change could never be observed
    /// through [`ContractPointView`] — the shape Phase 3's `contract_point`
    /// direction rule (`docs/specs/property_diff.md` §Direction) compares.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retain_departed: Option<String>,
}

impl ContractPointView {
    /// True when no relaxation applies (an empty object once serialized) —
    /// mirrors [`EffectiveContract::is_default`] for a caller that only has
    /// the rendered [`ContractPointView`] in hand.
    pub fn is_default(&self) -> bool {
        self.frozen_horizon.is_none() && self.deferral.is_none() && self.retain_departed.is_none()
    }
}

impl From<EffectiveContract> for ContractPointView {
    fn from(effective: EffectiveContract) -> Self {
        let (deferral, deferral_seconds, deferral_origin) = match effective.deferral {
            Some(d) => {
                let origin = match d.origin {
                    DeferralOrigin::Model => "model",
                    DeferralOrigin::Cell => "cell",
                };
                (
                    Some(d.window.display),
                    Some(d.window.seconds),
                    Some(origin.to_string()),
                )
            }
            None => (None, None, None),
        };
        let retain_departed = effective.retain_departed.map(|r| match r {
            RetainDeparted::Bool(_) => "true".to_string(),
            RetainDeparted::Tombstone { tombstone } => format!("tombstone: {tombstone}"),
        });
        ContractPointView {
            frozen_horizon_seconds: effective.frozen_horizon.as_ref().map(|h| h.seconds),
            frozen_horizon: effective.frozen_horizon.map(|h| h.display),
            deferral,
            deferral_seconds,
            deferral_origin,
            retain_departed,
        }
    }
}

#[cfg(test)]
mod effective_contract_tests {
    use super::*;

    #[test]
    fn effective_contract_defaults_to_the_default_point() {
        let effective = effective_contract(None, "sources.raw.events", &["amount".to_string()]);
        assert_eq!(effective, EffectiveContract::default());
        assert!(effective.is_default());
        assert_eq!(effective.render_label(), "default");
    }

    #[test]
    fn effective_contract_applies_model_level_frozen_horizon_to_every_cell() {
        let cfg = ContractConfig {
            frozen_horizon: DataLatency::parse("90 days"),
            deferral: None,
            retain_departed: None,
            cells: vec![],
        };
        let effective = effective_contract(Some(&cfg), "backfill", &[]);
        assert_eq!(effective.frozen_horizon, DataLatency::parse("90 days"));
        assert_eq!(effective.deferral, None);
        assert_eq!(effective.render_label(), "frozen_horizon 90 days");

        // Reaches a cell regardless of its trigger/columns.
        let other = effective_contract(Some(&cfg), "sources.raw.events", &["revenue".to_string()]);
        assert_eq!(other.frozen_horizon, DataLatency::parse("90 days"));
    }

    #[test]
    fn effective_contract_cell_deferral_overrides_the_model_default() {
        let cfg = ContractConfig {
            frozen_horizon: None,
            deferral: DataLatency::parse("6 hours"),
            retain_departed: None,
            cells: vec![ContractCellConfig {
                columns: vec!["amount".to_string()],
                on: "sources.raw.events".to_string(),
                deferral: DataLatency::parse("1 day"),
            }],
        };
        let effective = effective_contract(
            Some(&cfg),
            "sources.raw.events",
            &["amount".to_string(), "user_id".to_string()],
        );
        assert_eq!(effective.render_label(), "deferral 1 day (cell)");
        let deferral = effective.deferral.expect("deferral applies");
        assert_eq!(deferral.window, DataLatency::parse("1 day").unwrap());
        assert_eq!(deferral.origin, DeferralOrigin::Cell);
    }

    #[test]
    fn effective_contract_non_matching_cell_entry_keeps_the_model_default() {
        let cfg = ContractConfig {
            frozen_horizon: None,
            deferral: DataLatency::parse("6 hours"),
            retain_departed: None,
            cells: vec![ContractCellConfig {
                columns: vec!["other_column".to_string()],
                on: "sources.raw.events".to_string(),
                deferral: DataLatency::parse("1 day"),
            }],
        };
        let effective =
            effective_contract(Some(&cfg), "sources.raw.events", &["amount".to_string()]);
        let deferral = effective.deferral.expect("model-level deferral applies");
        assert_eq!(deferral.window, DataLatency::parse("6 hours").unwrap());
        assert_eq!(deferral.origin, DeferralOrigin::Model);

        // A different `on:` trigger also keeps the model default.
        let different_trigger =
            effective_contract(Some(&cfg), "backfill", &["other_column".to_string()]);
        assert_eq!(
            different_trigger.deferral.map(|d| d.origin),
            Some(DeferralOrigin::Model)
        );
    }

    #[test]
    fn render_label_includes_retain_departed() {
        let bare = ContractConfig {
            frozen_horizon: None,
            deferral: None,
            retain_departed: Some(RetainDeparted::Bool(true)),
            cells: vec![],
        };
        let effective = effective_contract(Some(&bare), "backfill", &[]);
        assert_eq!(effective.retain_departed, Some(RetainDeparted::Bool(true)));
        assert_eq!(effective.render_label(), "retain_departed");

        let tombstoned = ContractConfig {
            frozen_horizon: None,
            deferral: None,
            retain_departed: Some(RetainDeparted::Tombstone {
                tombstone: "is_departed".to_string(),
            }),
            cells: vec![],
        };
        let effective = effective_contract(Some(&tombstoned), "backfill", &[]);
        assert_eq!(
            effective.render_label(),
            "retain_departed (tombstone: is_departed)"
        );
    }
}
