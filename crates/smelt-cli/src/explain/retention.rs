//! Retention reach vs. required reach — `smelt explain` rendering
//! (`docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan report",
//! **Retention reach.** paragraph), split out of `explain.rs` proper from
//! the start since that file already sits at this crate's large-file
//! baseline (`docs/outcomes/20260906-trimmed-history-sources/phases/10-plan.md`).
//!
//! Both the text rows and the JSON entries read
//! [`smelt_logical::maintenance::MaintenancePlan::retention_reaches`]/
//! `retention_downgrades` verbatim — this module derives no retention
//! verdict of its own (`CLAUDE.md` §"Maintenance-plan purity").

use std::fmt::Write as _;

use serde::Serialize;
use smelt_logical::analysis::retention_reach::UnprovableReason;
use smelt_logical::maintenance::{RetentionDowngrade, RetentionReach};

/// Render [`UnprovableReason`] as the short clause the text row and the
/// JSON `reason` field both use — a fourth restatement of the same match
/// already duplicated in `smelt-db`'s `refusal_diag.rs` and `smelt-runtime`'s
/// `retention_admission.rs`; each caller renders it in its own surface's
/// voice, so no shared function unifies them.
fn unprovable_reason_text(reason: UnprovableReason) -> &'static str {
    match reason {
        UnprovableReason::UnboundedReach => "the model's reach into it is unbounded",
        UnprovableReason::ReachNotDerivable => "the model's reach into it could not be derived",
    }
}

/// Append the `Retention:` section to `out` — one row per source carrying a
/// bounded proof (`RetentionReach`) or a recorded downgrade
/// (`RetentionDowngrade`), in `retention_reaches`-then-`retention_downgrades`
/// order (both lists already sorted by source). Omitted entirely (writes
/// nothing) when both lists are empty — no empty-section noise for a model
/// referencing no `retention:` source.
pub fn write_retention_text(
    out: &mut String,
    reaches: &[RetentionReach],
    downgrades: &[RetentionDowngrade],
) {
    if reaches.is_empty() && downgrades.is_empty() {
        return;
    }
    let _ = writeln!(out, "Retention:");
    for reach in reaches {
        if reach.required_lookback <= reach.retained {
            let _ = writeln!(
                out,
                "  - {}: retained {}s, required reach {}s — within bound",
                reach.source, reach.retained.0, reach.required_lookback.0
            );
        } else {
            let _ = writeln!(
                out,
                "  - {}: retained {}s, required reach {}s — exceeds bound \
                 (SourceRetentionExceeded)",
                reach.source, reach.retained.0, reach.required_lookback.0
            );
        }
    }
    for downgrade in downgrades {
        let _ = writeln!(
            out,
            "  - {}: retained {}s, reach unprovable — downgraded (SourceRetentionDowngraded): {}",
            downgrade.source,
            downgrade.retained.0,
            unprovable_reason_text(downgrade.reason)
        );
    }
    let _ = writeln!(out);
}

/// JSON shape of one source's retention row (`smelt explain --json`):
/// `required_lookback_secs` present only for the bounded verdicts
/// (`within`/`exceeds`), `reason` only for `unprovable`
/// (`docs/specs/cli.md` §"Retention reach.").
#[derive(Debug, Serialize)]
pub struct ExplainRetentionJson {
    pub source: String,
    pub verdict: String,
    pub retained_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_lookback_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Build the `retention` array for `smelt explain --json` — empty when the
/// model references no `retention:` source, in the same
/// `retention_reaches`-then-`retention_downgrades` order the text rendering
/// uses.
pub fn retention_json_rows(
    reaches: &[RetentionReach],
    downgrades: &[RetentionDowngrade],
) -> Vec<ExplainRetentionJson> {
    let mut rows: Vec<ExplainRetentionJson> = reaches
        .iter()
        .map(|reach| {
            let verdict = if reach.required_lookback <= reach.retained {
                "within"
            } else {
                "exceeds"
            };
            ExplainRetentionJson {
                source: reach.source.clone(),
                verdict: verdict.to_string(),
                retained_secs: reach.retained.0,
                required_lookback_secs: Some(reach.required_lookback.0),
                reason: None,
            }
        })
        .collect();
    rows.extend(downgrades.iter().map(|downgrade| ExplainRetentionJson {
        source: downgrade.source.clone(),
        verdict: "unprovable".to_string(),
        retained_secs: downgrade.retained.0,
        required_lookback_secs: None,
        reason: Some(unprovable_reason_text(downgrade.reason).to_string()),
    }));
    rows
}
