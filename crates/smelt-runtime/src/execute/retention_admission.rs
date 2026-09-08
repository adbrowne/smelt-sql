//! Run-time re-evaluation of a source's retained bound against the run's own
//! window age (`docs/specs/sources.md` §Semantics 5 "Retention refusal",
//! `docs/outcomes/20260906-trimmed-history-sources/outcome.md` criterion 5).
//!
//! Pure fold over [`smelt_logical::maintenance::MaintenancePlan::
//! retention_reaches`] — this module derives nothing from a model's SQL
//! itself (maintenance-plan purity, `CLAUDE.md` §"Maintenance-plan purity").
//! It only ages the bounded proof the plan already carries by the run's own
//! window age and reports the result; the plan derivation that produces
//! `retention_reaches` happens once, in `smelt-logical`/`smelt-db`.

use chrono::NaiveDate;

use smelt_logical::maintenance::{
    full_refresh_retention_verdict, retention_refusals_at_age, run_window_age, FullRefreshLicense,
    FullRefreshRetention, MaintenancePlan, Refusal, RetainedSource, SourceRetentions,
};
use smelt_logical::Seconds;

/// A run over `source` reaches further back than its declared `retention:`
/// bound once the run's own window age is folded onto the model's derived
/// reach — refused before any statement executes for the model
/// (`docs/specs/sources.md` §Semantics 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionAdmissionError {
    pub source: String,
    pub required_lookback: Seconds,
    pub retained: Seconds,
    pub window_start: Option<NaiveDate>,
}

impl std::fmt::Display for RetentionAdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let window = match self.window_start {
            Some(start) => format!("run window starting {start}"),
            None => "a forward-only run window".to_string(),
        };
        write!(
            f,
            "SourceRetentionExceeded: '{}' requires reading back {} seconds, but only {} \
             seconds are retained ({window})",
            self.source, self.required_lookback.0, self.retained.0
        )
    }
}

impl std::error::Error for RetentionAdmissionError {}

/// The run's own window age (`smelt_logical::maintenance::run_window_age`),
/// measured from the run's resolved `--start` (`None` for a forward-only
/// run, which has age zero by construction) to the run clock
/// (`execute_project`'s own `run_start`, never `Utc::now()` inline —
/// `docs/outcomes/20260906-trimmed-history-sources/phases/05-plan.md` task
/// 4).
pub(crate) fn window_age(window_start: Option<NaiveDate>, run_clock: NaiveDate) -> Seconds {
    match window_start {
        Some(start) => run_window_age(start, run_clock),
        None => Seconds::ZERO,
    }
}

/// Derive `model_file`'s maintenance plan for the sole purpose of reading
/// its [`MaintenancePlan::retention_reaches`]/`retention_downgrades` —
/// `None` for a non-`refresh: incremental` model (no plan to derive at all)
/// or one with no `metadata` (parse/validation already refused it upstream).
/// Reuses the same fact-building this file's siblings already use
/// (`build_maint_source_facts`, `build_succession_source_refs`) rather than
/// inventing a third way to assemble `SourceFacts`/`source_refs`, and goes
/// through [`crate::maintenance_availability::derive_resolved`] — the ONLY
/// place `smelt-runtime` may call the raw `smelt-db` maintenance-plan
/// derivation (`cargo test -p smelt-runtime --test availability_seam`'s
/// structural gate). `StateAvailability::all()` since this call never reads
/// `plan.cells`/`state_downgrade` — none of the fields this call has no use
/// for (`explicitly_mutable`, `driving_source_granularity`,
/// `key_recurrences`, deployed-schema facts, referential integrity) reach
/// the retention fold in `smelt-logical`, which is posed against the
/// model's own SQL and its declared `retention:` sources alone.
pub(crate) fn derive_model_retention_plan(
    model_file: &smelt_core::ModelFile,
    source_infos: &[smelt_core::sources::SourceInfo],
) -> Option<MaintenancePlan> {
    let metadata = model_file.metadata.as_deref()?;
    let sql = smelt_parser::strip_frontmatter(&model_file.content);
    let table = model_file.db_name_owned();
    let (sources, _) = super::key_addressed::build_maint_source_facts(model_file, source_infos);
    let source_refs =
        crate::maintenance_driver::build_succession_source_refs(model_file, source_infos);
    let result = crate::maintenance_availability::derive_resolved(
        &sql,
        &table,
        metadata,
        &sources,
        &std::collections::HashSet::new(),
        None,
        &[],
        &[],
        &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
        None,
        None,
        &smelt_logical::maintenance::availability::StateAvailability::all(),
        &source_refs,
    )?;
    Some(result.plan)
}

/// Check `plan`'s already-derived [`MaintenancePlan::retention_reaches`]
/// against the run's own window age — the FIRST exceeding reach, if any, is
/// returned as a [`RetentionAdmissionError`] (there is at most one refusal
/// worth reporting per call; the run refuses the whole model rather than
/// enumerating every offending source).
pub(crate) fn check_retention_admission(
    plan: &MaintenancePlan,
    window_start: Option<NaiveDate>,
    run_clock: NaiveDate,
) -> Result<(), RetentionAdmissionError> {
    let age = window_age(window_start, run_clock);
    let refusals = retention_refusals_at_age(&plan.retention_reaches, age);
    match refusals.into_iter().next() {
        Some(Refusal::SourceRetentionExceeded {
            source,
            required_lookback_secs,
            retained_secs,
        }) => Err(RetentionAdmissionError {
            source,
            required_lookback: Seconds(required_lookback_secs),
            retained: Seconds(retained_secs),
            window_start,
        }),
        Some(other) => unreachable!(
            "retention_refusals_at_age only ever produces SourceRetentionExceeded, got {other:?}"
        ),
        None => Ok(()),
    }
}

/// The bare source name → declared `retention:` world-fact map for
/// `model_file`'s own refs — the same ref → bare-name mapping
/// [`super::key_addressed::build_maint_source_facts`] performs, duplicated
/// here rather than exposed as a third return value from that function
/// since only the whole-table-recompute gate needs it. Delegates the
/// map-building itself to [`smelt_db::queries::maintenance::
/// build_source_retentions`] rather than re-deriving it a second way.
pub(crate) fn model_source_retentions(
    model_file: &smelt_core::ModelFile,
    source_infos: &[smelt_core::sources::SourceInfo],
) -> SourceRetentions {
    let refs: Vec<(String, Option<smelt_core::sources::SourceInfo>)> = model_file
        .refs
        .iter()
        .filter_map(|r| {
            let segs = r.smelt_ref.to_path();
            let info = source_infos.iter().find(|s| s.address_segments == segs)?;
            let bare = match segs.split_first() {
                Some((first, rest)) if first == "sources" => rest.join("."),
                _ => segs.join("."),
            };
            Some((bare, Some(info.clone())))
        })
        .collect();
    smelt_db::queries::maintenance::build_source_retentions(&refs)
}

/// A whole-table recompute over `sources` refuses because stored output
/// already exists and no license was given
/// (`docs/specs/sources.md` §Semantics 5 "Retention refusal"). Unlike
/// [`RetentionAdmissionError`] (an aged backfill window), there is no
/// look-back to report — a whole-table recompute's reach is unbounded by
/// construction, so the error names only the retained sources at stake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FullRefreshRetentionError {
    pub sources: Vec<RetainedSource>,
}

impl std::fmt::Display for FullRefreshRetentionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<String> = self
            .sources
            .iter()
            .map(|s| format!("'{}' (retains {} seconds)", s.source, s.retained.0))
            .collect();
        write!(
            f,
            "SourceRetentionExceeded: a whole-table recompute reaches past every finite bound, \
             but stored output already exists and no license was given for: {}",
            names.join(", ")
        )
    }
}

impl std::error::Error for FullRefreshRetentionError {}

/// Render one [`RetainedSource`] a licensed whole-table recompute loses
/// replayability over as the reporter-warning message text
/// (`docs/specs/sources.md` §Semantics 5 "Retention refusal" — never
/// silent).
pub(crate) fn full_refresh_loss_warning_message(loss: &RetainedSource) -> String {
    format!(
        "SourceRetentionDowngraded: whole-table recompute over '{}' reaches past its {} second \
         retention bound — its retained region is no longer claimed replayable",
        loss.source, loss.retained.0
    )
}

/// The whole-table-recompute retention gate
/// (`docs/specs/sources.md` §Semantics 5 "Retention refusal"): reads
/// `model_file`'s declared `retention:` sources directly (never a derived
/// reach — a whole-table recompute's reach is unbounded by construction),
/// and folds `stored_state`/`license` onto
/// [`smelt_logical::maintenance::full_refresh_retention_verdict`]. Returns
/// the licensed-path warning messages to report (empty when nothing is at
/// stake), or the refusal naming every source at stake.
pub(crate) fn check_full_refresh_retention(
    model_file: &smelt_core::ModelFile,
    source_infos: &[smelt_core::sources::SourceInfo],
    stored_state: bool,
    license: FullRefreshLicense,
) -> Result<Vec<String>, FullRefreshRetentionError> {
    let retentions = model_source_retentions(model_file, source_infos);
    match full_refresh_retention_verdict(&retentions, stored_state, license) {
        FullRefreshRetention::Admit => Ok(Vec::new()),
        FullRefreshRetention::Licensed { losses } => Ok(losses
            .iter()
            .map(full_refresh_loss_warning_message)
            .collect()),
        FullRefreshRetention::Refuse { losses } => {
            Err(FullRefreshRetentionError { sources: losses })
        }
    }
}

/// Render one [`smelt_logical::maintenance::RetentionDowngrade`] as the
/// reporter-warning message text (`docs/specs/sources.md` §Semantics 5
/// "Retention refusal"; test 8's own criterion — surfaced once per run via
/// `RunReporter::maintenance_warning`, never silence).
pub(crate) fn downgrade_warning_message(
    downgrade: &smelt_logical::maintenance::RetentionDowngrade,
) -> String {
    let reason = match downgrade.reason {
        smelt_logical::analysis::retention_reach::UnprovableReason::UnboundedReach => {
            "the model's derived reach into it is unbounded"
        }
        smelt_logical::analysis::retention_reach::UnprovableReason::ReachNotDerivable => {
            "the model's derived reach into it could not be proven"
        }
    };
    format!(
        "SourceRetentionDowngraded: '{}' retains {} seconds but {reason} — its pre-bound \
         region is no longer claimed replayable",
        downgrade.source, downgrade.retained.0
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_logical::maintenance::RetentionReach;

    fn plan_with_reach(required_lookback: Seconds, retained: Seconds) -> MaintenancePlan {
        MaintenancePlan {
            retention_reaches: vec![RetentionReach {
                source: "silver.events".to_string(),
                required_lookback,
                retained,
            }],
            ..MaintenancePlan::default()
        }
    }

    #[test]
    fn a_reach_within_bound_at_zero_age_admits() {
        let plan = plan_with_reach(Seconds::days(7), Seconds::days(45));
        let now = NaiveDate::from_ymd_opt(2026, 9, 9).unwrap();
        assert!(check_retention_admission(&plan, None, now).is_ok());
    }

    #[test]
    fn an_aged_backfill_window_refuses() {
        let plan = plan_with_reach(Seconds::days(7), Seconds::days(45));
        let now = NaiveDate::from_ymd_opt(2026, 9, 9).unwrap();
        let window_start = now - chrono::Duration::days(60);
        let err = check_retention_admission(&plan, Some(window_start), now).unwrap_err();
        assert_eq!(err.source, "silver.events");
        assert_eq!(err.required_lookback, Seconds::days(67));
        assert_eq!(err.retained, Seconds::days(45));
    }
}
