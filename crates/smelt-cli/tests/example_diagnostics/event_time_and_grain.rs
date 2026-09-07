use crate::support::*;
use crate::support_ext::*;

/// Phase 6 TDD: `examples/incremental_broken_union_event_time/` produces
/// exactly one `EventTimeColumnNotVisibleAtOuterSelect` Error diagnostic
/// anchored at `models/union_mart.sql`.
///
/// The model declares `event_time_column: event_date` but uses a UNION ALL
/// query: injecting a WHERE clause would only filter the first branch and
/// produce incorrect results.
#[test]
fn incremental_broken_union_event_time() {
    check_workspace_emits_event_time_not_visible_diagnostic(
        "examples/incremental_broken_union_event_time",
        "models/union_mart.sql",
    );
}

/// Phase 2 TDD: `examples/data_checks/` — a workspace containing a `smelt.check`
/// and a regular model loads with zero diagnostics. The check is excluded from
/// materialization (not in the explain catalog) and produces no warnings.
#[test]
fn check_excluded_from_run_and_explain() {
    // The data_checks fixture contains:
    //   models/revenue.sql         — a regular model
    //   checks/no_negative_amounts.sql — a smelt.check declaration
    // The check must load without diagnostics (it is not materialized, not in
    // the explain/catalog, and must not cause the workspace to emit warnings).
    check_workspace_no_diagnostics("examples/data_checks");
}

/// Phase A0 TDD (`docs/plans/20260715-composed-axes-conditional-maintenance.md`):
/// `examples/timeseries_broken_key_per_partition/models/trajectory.sql`
/// declares `refresh: incremental` with a `timeseries:` clock and a
/// `unique_key:` identity whose `partition_column` is a member — derives the
/// `key_per_partition` grain, which maintenance-plan derivation does not yet
/// support. It must produce exactly
/// one `MaintenanceUnsupportedGrain` diagnostic naming the grain and the
/// tracking plan, not a silently-derived keyed plan with an empty
/// `unique_key` (`crates/smelt-db/src/queries/maintenance.rs`).
#[test]
fn timeseries_broken_key_per_partition_emits_unsupported_grain() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, DiagnosticCode, Workspace};

    let expected_file = "models/trajectory.sql";

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/timeseries_broken_key_per_partition");

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut target: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_target_code = |code: Option<&DiagnosticCode>| -> bool {
        code == Some(&DiagnosticCode::MaintenanceUnsupportedGrain)
    };

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        let rel = model
            .path
            .strip_prefix(&path)
            .unwrap()
            .display()
            .to_string();
        let is_target = rel.replace('\\', "/").ends_with(expected_file);

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !is_target_code(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target.push(d.clone());
            } else {
                other.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_target_code(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target.push(d.0.clone());
            } else {
                other.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other.is_empty(),
        "expected zero MaintenanceUnsupportedGrain diagnostics from files other than \
         '{expected_file}', got {}:\n  {}",
        other.len(),
        other
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target.len(),
        1,
        "expected exactly 1 MaintenanceUnsupportedGrain from '{expected_file}', got {}:\n  {}",
        target.len(),
        target
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    assert_eq!(
        target[0].code,
        Some(DiagnosticCode::MaintenanceUnsupportedGrain)
    );
    assert!(
        target[0].message.contains("key_per_partition"),
        "message must name the unsupported grain: {}",
        target[0].message
    );
    assert!(
        target[0]
            .message
            .contains("20260715-composed-axes-conditional-maintenance.md"),
        "message must name the tracking plan: {}",
        target[0].message
    );
}

/// `examples/web_analytics/models/silver/events_parsed.sql` uses the
/// top-level `safety_overrides:` key. Reverting it to the retired
/// `batched.safety_overrides` sub-block spelling is now a hard parse-time
/// error, not an accepted alternate spelling — the fix-it names the
/// top-level replacement carrying the caller's own declared flag
/// (`docs/specs/models.md` §"The Relation Contract").
#[test]
fn events_parsed_reverted_to_batched_sub_block_is_hard_refused() {
    use std::path::Path;

    let live_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/web_analytics");
    let sql_path = live_dir.join("models/silver/events_parsed.sql");
    let content = std::fs::read_to_string(&sql_path).unwrap();
    let reverted = content.replacen(
        "safety_overrides:\n  allow_window_functions: true\n---",
        "batched:\n  safety_overrides:\n    allow_window_functions: true\n---",
        1,
    );
    assert_ne!(
        content, reverted,
        "sanity: the top-level safety_overrides: spelling must be present in the fixture \
         for this test to exercise a real revert"
    );

    let err = smelt_core::metadata::extract_file_metadata(&reverted)
        .expect_err("the reverted batched.safety_overrides sub-block must be hard-refused");
    let message = err.to_string();
    assert!(
        message.contains("safety_overrides") && message.contains("allow_window_functions"),
        "fix-it must name safety_overrides: and the caller's own declared flag; got: {message}"
    );
}

/// `docs/outcomes/20260904-decided-gap-residue/phases/01-plan.md`:
/// `examples/broken/models/contract_frozen_horizon_mutable_source.sql` — a
/// `grain: partition` model declaring `contract.frozen_horizon` driven by
/// `contract_mutable_orders`, a `mutation_profile: mutable_snapshot` source
/// — refused with `ContractFrozenHorizonInvalid`, and that code fires from
/// no other file in the shared `examples/broken/` workspace.
///
/// Spec: `docs/specs/incremental_models.md` §"The contract lattice".
#[test]
fn broken_contract_frozen_horizon_mutable_source() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticCode, Workspace};

    let expected_file = "models/contract_frozen_horizon_mutable_source.sql";

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/broken");

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut target: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        let rel = model
            .path
            .strip_prefix(&path)
            .unwrap()
            .display()
            .to_string();
        let is_target = rel.replace('\\', "/").ends_with(expected_file);

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if d.code != Some(DiagnosticCode::ContractFrozenHorizonInvalid) {
                continue;
            }
            if is_target {
                target.push(d.clone());
            } else {
                other.push((rel.clone(), d.clone()));
            }
        }
    }

    assert!(
        other.is_empty(),
        "expected zero ContractFrozenHorizonInvalid diagnostics from files other than \
         '{expected_file}', got {}:\n  {}",
        other.len(),
        other
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target.len(),
        1,
        "expected exactly 1 ContractFrozenHorizonInvalid from '{expected_file}', got {}:\n  {}",
        target.len(),
        target
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}
