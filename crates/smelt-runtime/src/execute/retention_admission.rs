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
/// for (`explicitly_mutable`, `key_recurrences`, deployed-schema facts,
/// referential integrity) reach the retention fold in `smelt-logical`,
/// which is posed against the model's own SQL and its declared
/// `retention:` sources alone. `driving_source_granularity` DOES reach the
/// retention fold indirectly: `establish_locality`'s granularity-equality
/// precondition gates the whole plan (including `retention_reaches`) behind
/// `Refusal::LocalityNotEstablished` for a `timeseries:`-bearing model, so
/// it must be resolved here the same way `smelt-db`'s
/// `maintenance_refs/plan.rs` (the diagnostics path this derivation must
/// agree with) resolves it: `single_clocked_granularity` over the
/// declared-source candidate pool UNION every referenced upstream
/// maintained model this workspace admits key temporal locality for
/// (`composed_sources`, resolved once in `execute_project` via
/// `crate::propagation::composed_source_granularities` — the same
/// converged candidate `clamp_locality.rs`'s `grain: key`-gated form
/// folds in). This closes the divergence phases 7/8 left open: leg 3 of
/// the closure argument (`docs/outcomes/20260906-trimmed-history-sources/
/// phases/09-plan.md`) — "adding a composed-upstream candidate can only
/// take `Some → None`, never `None → Some`" — is FALSE when the
/// declared-source pool is empty (a `grain: key` model whose SOLE clocked
/// candidate is an upstream model's composed output): adding that one
/// candidate resolves `None → Some`
/// (`smelt-logical`'s `adding_a_candidate_to_an_empty_pool_resolves_an_
/// undecided_granularity` pins the counterexample). Without this fold, that
/// exact model shape would have resolved `driving_source_granularity: None`
/// here — refused by `establish_locality`'s own precondition before the
/// retention fold ever ran, even though `smelt-db`'s diagnostics path (and a
/// real `smelt build`) admits it.
pub(crate) fn derive_model_retention_plan(
    model_file: &smelt_core::ModelFile,
    source_infos: &[smelt_core::sources::SourceInfo],
    composed_sources: &std::collections::BTreeMap<
        String,
        (
            smelt_logical::maintenance::SourceFacts,
            smelt_core::config::Granularity,
        ),
    >,
) -> Option<MaintenancePlan> {
    let metadata = model_file.metadata.as_deref()?;
    let sql = smelt_parser::strip_frontmatter(&model_file.content);
    let table = model_file.db_name_owned();
    let (mut sources, _) = super::key_addressed::build_maint_source_facts(model_file, source_infos);
    let source_refs =
        crate::maintenance_driver::build_succession_source_refs(model_file, source_infos);
    let mut clocked_granularities: Vec<smelt_core::config::Granularity> = source_refs
        .iter()
        .filter_map(|(_, info)| info.as_ref().and_then(|i| i.timeseries.as_ref()))
        .map(|t| t.granularity)
        .collect();
    // Mirrors `smelt-db::maintenance_refs::plan.rs`'s own `grain: key`-gated
    // composed-upstream fold: every referenced address that resolves in the
    // converged `composed_sources` map contributes both a `SourceFacts`
    // candidate (so `resolve_driving_source` can anchor on it, the same way
    // a declared source anchors) and a clocked-granularity candidate.
    if metadata.resolved_grain() == Some(smelt_core::config::Grain::Key) {
        for r in &model_file.refs {
            let addr = r.smelt_ref.to_path().join(".");
            let Some((facts, granularity)) = composed_sources.get(&addr) else {
                continue;
            };
            if !sources.iter().any(|s| s.name == facts.name) {
                sources.push(facts.clone());
                clocked_granularities.push(*granularity);
            }
        }
    }
    let driving_source_granularity =
        smelt_logical::maintenance::locality::single_clocked_granularity(clocked_granularities);
    let result = crate::maintenance_availability::derive_resolved(
        &sql,
        &table,
        metadata,
        &sources,
        &std::collections::HashSet::new(),
        driving_source_granularity,
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
    use std::collections::BTreeMap;

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

    // ---- `driving_source_granularity` resolution at this call site
    // (`docs/outcomes/20260906-trimmed-history-sources/phases/07-plan.md`
    // tests 1-3) — staged real files through `ModelDiscovery`/
    // `discover_source_infos` (no backend needed) rather than hand-built
    // `ModelFile`/`SourceInfo` literals, so the `refs`/`address_segments`
    // wiring `derive_model_retention_plan` actually reads is exercised the
    // same way a real project produces it.

    fn stage_source(dir: &std::path::Path, name: &str, granularity: &str, retention: bool) {
        std::fs::create_dir_all(dir.join("models/sources")).unwrap();
        let retention_line = if retention {
            "retention: '45 days'\n"
        } else {
            ""
        };
        let yml = format!(
            "description: Test source.\ncolumns:\n  - name: device_id\n    type: INTEGER\n  \
             - name: event_date\n    type: DATE\n  - name: amount\n    type: DOUBLE\n\
             timeseries:\n  event_time_column: event_date\n  partition_column: event_date\n  \
             granularity: {granularity}\nmutation_profile:\n  kind: append_only\n{retention_line}"
        );
        std::fs::write(dir.join(format!("models/sources/{name}.yml")), yml).unwrap();
    }

    fn discover(
        dir: &std::path::Path,
        model_name: &str,
    ) -> (smelt_core::ModelFile, Vec<smelt_core::sources::SourceInfo>) {
        let paths = vec!["models".to_string()];
        let source_infos = smelt_core::discover_source_infos(dir, &paths);
        let models = smelt_core::ModelDiscovery::new(dir.to_path_buf(), paths)
            .discover_models()
            .expect("discover_models");
        let model = models
            .into_iter()
            .find(|m| m.name == model_name)
            .unwrap_or_else(|| panic!("model '{model_name}' not discovered"));
        (model, source_infos)
    }

    /// Test 1: a `grain: key` model whose own `timeseries:` block clears
    /// route 1 (key-embedded — `event_date` is both the `partition_column`
    /// and a `unique_key`/`GROUP BY` column) over one clocked,
    /// `retention:`-bearing source reaches the retention fold: the derived
    /// plan carries a non-empty `retention_reaches` and no
    /// `Refusal::LocalityNotEstablished`. RED before this phase's fix
    /// (`driving_source_granularity: None` fails `establish_locality`'s
    /// granularity-equality precondition, so the plan comes back
    /// `locality_refused_plan` — empty reaches — before the retention fold
    /// ever runs).
    #[test]
    fn keyed_model_with_timeseries_reaches_the_retention_fold() {
        let tmp = tempfile::TempDir::new().unwrap();
        stage_source(tmp.path(), "events", "day", true);
        let model_sql = r#"---
materialization: table
refresh: incremental
grain: key
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
maintenance:
  scan_bounds:
    per_source:
      events:
        allow_full_scan: true
---
SELECT device_id, event_date, SUM(amount) AS total_amount
FROM smelt.sources.events
GROUP BY 1, 2
"#;
        std::fs::create_dir_all(tmp.path().join("models")).unwrap();
        std::fs::write(tmp.path().join("models/keyed.sql"), model_sql).unwrap();

        let (model, source_infos) = discover(tmp.path(), "keyed");
        let plan = derive_model_retention_plan(&model, &source_infos, &BTreeMap::new())
            .expect("refresh: incremental with metadata must derive a plan");

        assert!(
            !plan.retention_reaches.is_empty(),
            "the retention fold must run and record a bounded reach: {plan:?}"
        );
        assert!(
            !plan.refusals.iter().any(|r| matches!(
                r,
                smelt_logical::maintenance::Refusal::LocalityNotEstablished { .. }
            )),
            "an admissible route-1 model must not be refused for locality: {:?}",
            plan.refusals
        );
    }

    /// Test 2: two referenced clocked sources of different granularities
    /// leave the resolution `None` (ambiguous) — the shared
    /// `single_clocked_granularity` "exactly one else `None`" rule is used
    /// here, not a bespoke re-derivation that might pick one arbitrarily.
    #[test]
    fn two_clocked_sources_leave_the_granularity_undecided() {
        let tmp = tempfile::TempDir::new().unwrap();
        stage_source(tmp.path(), "events_day", "day", true);
        stage_source(tmp.path(), "events_week", "week", false);
        let model_sql = r#"---
materialization: table
refresh: incremental
grain: key
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
maintenance:
  scan_bounds:
    per_source:
      events_day:
        allow_full_scan: true
      events_week:
        allow_full_scan: true
---
SELECT d.device_id, d.event_date, SUM(d.amount) AS total_amount
FROM smelt.sources.events_day d
JOIN smelt.sources.events_week w ON w.device_id = d.device_id
GROUP BY 1, 2
"#;
        std::fs::create_dir_all(tmp.path().join("models")).unwrap();
        std::fs::write(tmp.path().join("models/keyed.sql"), model_sql).unwrap();

        let (model, source_infos) = discover(tmp.path(), "keyed");
        let clocked_granularities =
            crate::maintenance_driver::build_succession_source_refs(&model, &source_infos)
                .iter()
                .filter_map(|(_, info)| info.as_ref().and_then(|i| i.timeseries.as_ref()))
                .map(|t| t.granularity)
                .collect::<Vec<_>>();
        assert_eq!(clocked_granularities.len(), 2);
        assert_eq!(
            smelt_logical::maintenance::locality::single_clocked_granularity(clocked_granularities),
            None,
            "two differently-granular clocked candidates must leave resolution undecided"
        );
    }

    /// Test 3 (regression pin): a `grain: partition` model derives the same
    /// `retention_reaches` before and after this change — the granularity
    /// resolution added to `derive_model_retention_plan` only ever
    /// contributes a *value*, it does not alter partition-grain plan
    /// derivation, which never consulted `driving_source_granularity`.
    #[test]
    fn partition_grain_derivation_is_unchanged() {
        let tmp = tempfile::TempDir::new().unwrap();
        stage_source(tmp.path(), "events", "day", true);
        let model_sql = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_date,
       SUM(amount) OVER (PARTITION BY event_date ORDER BY event_date
           RANGE BETWEEN INTERVAL '7 days' PRECEDING AND CURRENT ROW) AS total_amount
FROM smelt.sources.events
"#;
        std::fs::create_dir_all(tmp.path().join("models")).unwrap();
        std::fs::write(tmp.path().join("models/agg.sql"), model_sql).unwrap();

        let (model, source_infos) = discover(tmp.path(), "agg");
        let plan = derive_model_retention_plan(&model, &source_infos, &BTreeMap::new())
            .expect("refresh: incremental with metadata must derive a plan");

        assert_eq!(plan.retention_reaches.len(), 1);
        assert_eq!(plan.retention_reaches[0].source, "events");
        assert_eq!(
            plan.retention_reaches[0].required_lookback,
            Seconds::days(7)
        );
        assert_eq!(plan.retention_reaches[0].retained, Seconds::days(45));
    }

    // ---- Phase 9: composed-upstream granularity at this call site
    // (`docs/outcomes/20260906-trimmed-history-sources/phases/09-plan.md`
    // tests 4-5). Test 2 in `smelt-logical`'s own `locality` module pins the
    // counterexample that falsifies this phase's planning-stage leg-3
    // argument ("adding a candidate can only take `Some → None`, never
    // `None → Some`"): a `grain: key` model with ZERO declared clocked
    // source refs whose SOLE clocked candidate is an upstream maintained
    // model's own composed output resolves `None` at this call site without
    // the fold below, but `Some` once `composed_sources` is threaded in
    // (matching `smelt-db`'s `maintenance_refs/plan.rs` diagnostics path).

    /// Test 4: a `grain: key` model ("b") whose sole clocked candidate is
    /// an upstream maintained model ("a")'s composed output — no declared
    /// `sources:` ref of its own. WITHOUT `composed_sources` (the pre-fix
    /// shape), the plan comes back `Refusal::LocalityNotEstablished` (empty
    /// declared-source pool resolves `None`), even though a real project
    /// (`smelt-db`'s diagnostics path, which already folds composed
    /// upstreams) admits it — this is the divergence phase 9 closes.
    /// WITH the real `composed_source_granularities` map
    /// (`crate::propagation::composed_source_granularities`, the same
    /// fixed point `execute_project` now threads through), locality is
    /// established and the spurious refusal disappears. Neither case
    /// leaves the model "running with a silently empty `retention_reaches`
    /// that should have been non-empty" — "b" references no `retention:`
    /// source at all (declared or composed; test 5 pins why a composed
    /// upstream can never carry a retained bound), so an empty
    /// `retention_reaches` is the CORRECT answer once locality resolves,
    /// not a skipped check.
    #[test]
    fn a_keyed_model_over_a_composed_upstream_resolves_locality_via_composed_sources() {
        let tmp = tempfile::TempDir::new().unwrap();
        stage_source(tmp.path(), "events", "day", false);
        let model_a_sql = r#"---
materialization: table
refresh: incremental
grain: key
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
maintenance:
  scan_bounds:
    per_source:
      events:
        allow_full_scan: true
---
SELECT device_id, event_date, SUM(amount) AS total_amount
FROM smelt.sources.events
GROUP BY 1, 2
"#;
        let model_b_sql = r#"---
materialization: table
refresh: incremental
grain: key
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
maintenance:
  scan_bounds:
    per_source:
      a:
        allow_full_scan: true
---
SELECT device_id, event_date, SUM(total_amount) AS grand_total
FROM smelt.a
GROUP BY 1, 2
"#;
        std::fs::create_dir_all(tmp.path().join("models")).unwrap();
        std::fs::write(tmp.path().join("models/a.sql"), model_a_sql).unwrap();
        std::fs::write(tmp.path().join("models/b.sql"), model_b_sql).unwrap();

        let paths = vec!["models".to_string()];
        let source_infos = smelt_core::discover_source_infos(tmp.path(), &paths);
        let all_models = smelt_core::ModelDiscovery::new(tmp.path().to_path_buf(), paths)
            .discover_models()
            .expect("discover_models");
        let model_b = all_models
            .iter()
            .find(|m| m.name == "b")
            .expect("model 'b' discovered")
            .clone();

        // Pre-fix shape: no composed candidates threaded in.
        let plan_without_fold =
            derive_model_retention_plan(&model_b, &source_infos, &BTreeMap::new())
                .expect("refresh: incremental with metadata must derive a plan");
        assert!(
            plan_without_fold.refusals.iter().any(|r| matches!(
                r,
                smelt_logical::maintenance::Refusal::LocalityNotEstablished { .. }
            )),
            "with an empty composed-source pool, 'b' has zero clocked candidates and must be \
             spuriously locality-refused: {:?}",
            plan_without_fold.refusals
        );
        assert!(plan_without_fold.retention_reaches.is_empty());

        // Real fold, the same one `execute_project` now threads through.
        let composed =
            crate::propagation::composed_source_granularities(&all_models, &source_infos)
                .expect("acyclic model-ref graph converges");
        assert!(
            composed.contains_key("a"),
            "'a' is an admitted grain: key + timeseries model — it must be a composed candidate: \
             {composed:?}"
        );
        let plan_with_fold = derive_model_retention_plan(&model_b, &source_infos, &composed)
            .expect("refresh: incremental with metadata must derive a plan");
        assert!(
            !plan_with_fold.refusals.iter().any(|r| matches!(
                r,
                smelt_logical::maintenance::Refusal::LocalityNotEstablished { .. }
            )),
            "with the composed-source fold, 'b' has exactly one clocked candidate ('a') and \
             locality must establish: {:?}",
            plan_with_fold.refusals
        );
        // No `retention:` source in reach (declared or composed) — an
        // empty fold here is the correct answer, not a skipped check.
        assert!(plan_with_fold.retention_reaches.is_empty());
    }

    /// Test 5: `model_source_retentions` reads DECLARED `sources:` refs
    /// only — a referenced upstream *model* never contributes a retained
    /// bound, because `retention:` is a source-only declaration
    /// (`smelt-core`'s `RetentionWithoutTimeseries` refusal). This is why
    /// test 4's "b" (composed-upstream-only) can never have retention
    /// exposure to silently skip: the only way a model acquires retention
    /// exposure is by directly referencing a `retention:`-bearing source,
    /// which is itself always a declared clocked candidate — never a
    /// composed-only one.
    #[test]
    fn a_composed_upstream_contributes_no_retained_bound() {
        let tmp = tempfile::TempDir::new().unwrap();
        stage_source(tmp.path(), "events", "day", false);
        let model_a_sql = r#"---
materialization: table
refresh: incremental
grain: key
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
maintenance:
  scan_bounds:
    per_source:
      events:
        allow_full_scan: true
---
SELECT device_id, event_date, SUM(amount) AS total_amount
FROM smelt.sources.events
GROUP BY 1, 2
"#;
        let model_b_sql = r#"---
materialization: table
refresh: incremental
grain: key
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
maintenance:
  scan_bounds:
    per_source:
      a:
        allow_full_scan: true
---
SELECT device_id, event_date, SUM(total_amount) AS grand_total
FROM smelt.a
GROUP BY 1, 2
"#;
        std::fs::create_dir_all(tmp.path().join("models")).unwrap();
        std::fs::write(tmp.path().join("models/a.sql"), model_a_sql).unwrap();
        std::fs::write(tmp.path().join("models/b.sql"), model_b_sql).unwrap();

        let (model_b, source_infos) = discover(tmp.path(), "b");
        let retentions = model_source_retentions(&model_b, &source_infos);
        assert!(
            retentions.is_empty(),
            "a referenced upstream model must never contribute a retained bound: {retentions:?}"
        );
    }
}
