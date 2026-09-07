use std::collections::{HashMap, HashSet};

use anyhow::Result;
use chrono::Utc;

use smelt_backend::Backend;
use smelt_core::config::Config;
use smelt_state::file_store::FileStore;
use smelt_state::{RunManifest, RunReport};

use crate::check_runner::{run_single_check, CheckOutcome, CheckStatus};
use crate::compile::CompilerRegistry;
use crate::reporter::RunReporter;
use crate::types::RunOutcome;
use crate::EphemeralResolver;

/// Derive a [`RunReport`] from `manifest` and persist it alongside the
/// manifest at `.smelt/targets/<target>/reports/<run_id>.json`
/// (`docs/specs/run_state.md` §"Run report"). Called at every one of
/// `execute_project`'s manifest-save sites — success, cancelled, and
/// aborted — since a report is due whenever a manifest is, and a report
/// derived from an incomplete manifest (`completed_at: None`) is still a
/// meaningful partial summary for `--resume`/tooling to read.
pub(crate) fn write_run_report(file_store: &FileStore, manifest: &RunManifest) -> Result<()> {
    file_store.save_report(&RunReport::from_manifest(manifest))
}

/// Build a model's `ProbePolicy` from the project's `probes:` cadence and
/// its prior-run count in `prior_runs` (`docs/specs/model_properties.md`
/// §"Probe cadence"): the run ordinal is 0 for a model's first run.
pub(crate) fn probe_policy_for_model(
    config: &Config,
    prior_runs: &[RunManifest],
    model_name: &str,
) -> crate::probes::ProbePolicy {
    let run_ordinal = smelt_state::history::HistoryQuery::new(prior_runs)
        .for_model(model_name)
        .len() as u64;
    crate::probes::ProbePolicy::new(config.probes.cadence, run_ordinal)
}

pub(crate) fn build_outcome(
    run_id: &str,
    started_at: chrono::DateTime<Utc>,
    completed_at: Option<chrono::DateTime<Utc>>,
    manifest: RunManifest,
    total_rows: usize,
    check_results: Vec<CheckOutcome>,
) -> RunOutcome {
    RunOutcome {
        run_id: run_id.to_string(),
        started_at,
        completed_at,
        models: manifest.models,
        total_rows,
        plan_summary: None,
        check_results,
    }
}

/// Execute all checks registered for `model_name` after it materializes.
///
/// Returns `(outcomes, models_to_skip)` where:
/// - `outcomes` is the per-check result list to append to `check_results`
/// - `models_to_skip` is the downstream closure to add to `skip_set` when an
///   error-severity check fails (derived from `upstream_map`)
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_model_checks(
    model_name: &str,
    checks_by_model: &HashMap<String, Vec<smelt_core::ModelFile>>,
    compilers: &CompilerRegistry,
    backends: &HashMap<String, Box<dyn Backend>>,
    target_assignments: &HashMap<String, String>,
    ephemeral_resolvers: &HashMap<String, EphemeralResolver>,
    config: &smelt_core::config::Config,
    upstream_map: &HashMap<String, HashSet<String>>,
    selected: &[String],
    reporter: &dyn RunReporter,
    run_id: &str,
) -> (Vec<CheckOutcome>, HashSet<String>) {
    use smelt_core::metadata::CheckSeverity;

    let Some(check_files) = checks_by_model.get(model_name) else {
        return (vec![], HashSet::new());
    };

    let model_target = target_assignments
        .get(model_name)
        .map(|s| s.as_str())
        .unwrap_or(model_name);

    let Some(backend) = backends.get(model_target) else {
        return (vec![], HashSet::new());
    };

    let schema = &config.targets[model_target].schema;
    let compiler = compilers.get(model_target);

    static EMPTY_RESOLVER: std::sync::OnceLock<EphemeralResolver> = std::sync::OnceLock::new();
    let resolver = ephemeral_resolvers
        .get(model_target)
        .unwrap_or_else(|| EMPTY_RESOLVER.get_or_init(EphemeralResolver::empty));

    let ephemeral_names = &resolver.ephemeral_names;

    let mut outcomes: Vec<CheckOutcome> = Vec::new();
    let mut any_error_check_failed = false;

    for check_model in check_files {
        let severity: CheckSeverity = check_model
            .metadata
            .as_ref()
            .and_then(|m| m.check.as_ref())
            .map(|c| c.severity.clone())
            .unwrap_or_default();

        let outcome = match run_single_check(
            compiler,
            backend.as_ref(),
            schema,
            check_model,
            severity,
            ephemeral_names,
            resolver,
        )
        .await
        {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!("check '{}' error: {}", check_model.name, e);
                CheckOutcome {
                    name: check_model.name.clone(),
                    severity: CheckSeverity::Error,
                    status: CheckStatus::Fail,
                    row_count: 0,
                    sample: vec![],
                    message: Some(e.to_string()),
                    sql: None,
                }
            }
        };

        let status_str = match outcome.status {
            CheckStatus::Pass => "pass",
            CheckStatus::Fail => "fail",
            CheckStatus::Warn => "warn",
            CheckStatus::TargetNotBuilt => "target_not_built",
        };

        reporter.check_result(run_id, &outcome.name, status_str, outcome.row_count);

        if matches!(
            (&outcome.severity, &outcome.status),
            (
                CheckSeverity::Error,
                CheckStatus::Fail | CheckStatus::TargetNotBuilt
            )
        ) {
            any_error_check_failed = true;
        }

        outcomes.push(outcome);
    }

    // Compute downstream closure to skip (only for error-severity failures).
    let models_to_skip: HashSet<String> = if any_error_check_failed {
        selected
            .iter()
            .filter(|m| {
                upstream_map
                    .get(*m)
                    .is_some_and(|ups| ups.contains(model_name))
            })
            .cloned()
            .collect()
    } else {
        HashSet::new()
    };

    (outcomes, models_to_skip)
}
