use crate::support::*;
use crate::support_ext::*;

/// Phase C TDD: `examples/meta_columns/` produces zero diagnostics.
/// The clean fixture exercises the coalesce_numeric function end-to-end.
#[test]
fn meta_columns_clean_workspace() {
    check_workspace_no_diagnostics("examples/meta_columns");
}

/// Phase C TDD: `examples/meta_columns_broken_columns_of_requires_table_expr/` produces
/// exactly one `ColumnsOfRequiresTableExpr` diagnostic.
#[test]
fn meta_columns_broken_columns_of_requires_table_expr() {
    check_workspace_emits_exactly_one_phase_c_diagnostic(
        "examples/meta_columns_broken_columns_of_requires_table_expr",
        "models/columns_of_requires_table_expr.sql",
        smelt_db::DiagnosticCode::ColumnsOfRequiresTableExpr,
    );
}

/// Phase C TDD: `examples/meta_columns_broken_columns_of_named_argument/` produces
/// exactly one `ColumnsOfNamedArgument` diagnostic.
#[test]
fn meta_columns_broken_columns_of_named_argument() {
    check_workspace_emits_exactly_one_phase_c_diagnostic(
        "examples/meta_columns_broken_columns_of_named_argument",
        "models/columns_of_named_argument.sql",
        smelt_db::DiagnosticCode::ColumnsOfNamedArgument,
    );
}

/// Phase C TDD: `examples/meta_columns_broken_columns_of_unresolvable_schema/` produces
/// exactly one `ColumnsOfUnresolvableSchema` diagnostic.
#[test]
fn meta_columns_broken_columns_of_unresolvable_schema() {
    check_workspace_emits_exactly_one_phase_c_diagnostic(
        "examples/meta_columns_broken_columns_of_unresolvable_schema",
        "models/columns_of_unresolvable_schema.sql",
        smelt_db::DiagnosticCode::ColumnsOfUnresolvableSchema,
    );
}

/// Phase C TDD: `examples/meta_columns_broken_column_ref_field_unknown/` produces
/// exactly one `ColumnRefFieldUnknown` diagnostic.
///
/// The fixture uses a model-level HOF call `map(smelt.columns_of(orders), fn c => c.invalid)`
/// where `c` is ColumnRef-typed (bound by the HOF source list) and `invalid` is not
/// in the closed field set {name, type, is_numeric}.
#[test]
fn meta_columns_broken_column_ref_field_unknown() {
    check_workspace_emits_exactly_one_phase_c_diagnostic(
        "examples/meta_columns_broken_column_ref_field_unknown",
        "models/bad_field_access.sql",
        smelt_db::DiagnosticCode::ColumnRefFieldUnknown,
    );
}

/// Phase 5b TDD Test 3: After broken/ fixtures are migrated to `smelt.functions.*`,
/// the `all_examples_use_path_syntax` scan must include broken/ with no violations.
/// This test FAILS before migration because broken/ still contains `smelt.fn.*`.
#[test]
fn all_examples_use_path_syntax_including_broken() {
    let examples_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples");
    let mut legacy_usages: Vec<String> = Vec::new();
    for entry in walkdir::WalkDir::new(&examples_dir) {
        let entry = entry.unwrap();
        if entry.path().extension().and_then(|e| e.to_str()) != Some("sql") {
            continue;
        }
        // No skip for broken/ — this test covers ALL examples including broken/
        let content = std::fs::read_to_string(entry.path()).unwrap();
        for (line_no, line) in content.lines().enumerate() {
            // Skip comment lines
            let trimmed = line.trim_start();
            if trimmed.starts_with("--") {
                continue;
            }
            for pattern in &["smelt.ref(", "smelt.source(", "smelt.fn."] {
                if line.contains(pattern) {
                    legacy_usages.push(format!(
                        "{}:{}: {}",
                        entry.path().display(),
                        line_no + 1,
                        line.trim()
                    ));
                }
            }
        }
    }
    assert!(
        legacy_usages.is_empty(),
        "Found legacy smelt syntax in examples (including broken/; must be migrated to smelt.<path>):\n{}",
        legacy_usages.join("\n")
    );
}

/// Phase 5b TDD Test 4: Guard that key diagnostic codes still fire after
/// broken/ is migrated to `smelt.functions.*`. This is a regression guard —
/// it should pass both before and after migration.
#[test]
fn broken_workspace_diagnostics_still_fire() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::collections::HashSet;

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

    let mut codes: HashSet<String> = HashSet::new();
    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if let Some(code) = &d.code {
                codes.insert(format!("{:?}", code));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if let Some(code) = &d.0.code {
                codes.insert(format!("{:?}", code));
            }
        }
    }

    // These diagnostic codes must still fire after migration to smelt.functions.*
    for required_code in &[
        "ArgTypeMismatch",
        "MissingArgument",
        "FunctionCallCycle",
        "UnknownSmeltFn",
        "UnknownIdentifier",
    ] {
        assert!(
            codes.contains(*required_code),
            "Expected diagnostic code {:?} to fire in broken workspace, but it was absent.\n\
             Codes present: {:?}",
            required_code,
            codes
        );
    }
}

/// MP6 TDD: `examples/broken/models/maintenance_scan_unbounded.sql` — a
/// `grain: partition` model whose `enrichment_category` group is mutation-
/// sensitive to an unclocked `maintenance_enrichment` source with no
/// `allow_full_scan` acceptance — produces exactly one
/// `MaintenanceScanUnbounded` diagnostic per membership-sensitive payload
/// group, anchored at that file, and no `MaintenanceScanUnbounded`/
/// `MaintenanceNoAdmissibleTechnique` diagnostic fires from any other file
/// in the shared `examples/broken/` workspace. `maintenance_enrichment` is
/// read only in the JOIN's ON predicate — never in a select item for
/// `o.order_id` — so BOTH payload groups (`{order_id}` and
/// `{enrichment_category}`) are membership-sensitive to it
/// (`docs/specs/model_properties.md` §"Per-column mutation-sensitivity /
/// column provenance", membership paragraph) and each refuses independently.
///
/// Spec: `docs/specs/incremental_models.md` §Semantics "Partition-local
/// maintenance (the K8 guardrail)".
#[test]
fn broken_workspace_maintenance_scan_unbounded() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, DiagnosticCode, Workspace};

    const MAINTENANCE_CODES: &[DiagnosticCode] = &[
        DiagnosticCode::MaintenanceScanUnbounded,
        DiagnosticCode::MaintenanceNoAdmissibleTechnique,
    ];
    let expected_file = "models/maintenance_scan_unbounded.sql";

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
            if !d
                .code
                .as_ref()
                .is_some_and(|c| MAINTENANCE_CODES.contains(c))
            {
                continue;
            }
            if is_target {
                target.push(d.clone());
            } else {
                other.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !d
                .0
                .code
                .as_ref()
                .is_some_and(|c| MAINTENANCE_CODES.contains(c))
            {
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
        "expected zero maintenance diagnostics from files other than '{expected_file}', got {}:\n  {}",
        other.len(),
        other
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target.len(),
        2,
        "expected exactly 2 maintenance diagnostics (one per membership-sensitive \
         payload group) from '{expected_file}', got {}:\n  {}",
        target.len(),
        target
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    assert!(
        target
            .iter()
            .all(|d| d.code == Some(DiagnosticCode::MaintenanceScanUnbounded)),
        "target: {:?}",
        target
    );
}

/// `docs/outcomes/20260904-decision-residue` phase 1:
/// `examples/broken/models/partition_grain_forbids_metrics.sql` — a
/// `grain: partition` model whose body calls `smelt.metric(...)` — produces
/// exactly one `PartitionGrainForbidsMetrics` diagnostic from that file, and
/// none from any other file in the shared `examples/broken/` workspace.
///
/// Spec: `docs/specs/incremental_shapes.md` §"Functions inside
/// partition-grain bodies".
#[test]
fn broken_workspace_partition_grain_forbids_metrics() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, DiagnosticCode, Workspace};

    const TARGET_CODE: DiagnosticCode = DiagnosticCode::PartitionGrainForbidsMetrics;
    let expected_file = "models/partition_grain_forbids_metrics.sql";

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
            if d.code != Some(TARGET_CODE) {
                continue;
            }
            if is_target {
                target.push(d.clone());
            } else {
                other.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if d.0.code != Some(TARGET_CODE) {
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
        "expected zero PartitionGrainForbidsMetrics diagnostics from files other than \
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
        "expected exactly 1 PartitionGrainForbidsMetrics diagnostic from '{expected_file}', \
         got {}:\n  {}",
        target.len(),
        target
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// `docs/outcomes/20260904-decision-residue` phase 5:
/// `examples/broken/models/retired_data_latency.sql` — a per-column
/// `data_latency:` key produces exactly one `YamlParseError` naming
/// `mutation_profile.lateness` from that file, and none from any other file
/// in the shared `examples/broken/` workspace.
///
/// Spec: `docs/specs/models.md` §Diagnostics.
#[test]
fn broken_workspace_retired_data_latency() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, DiagnosticCode, Workspace};

    const TARGET_CODE: DiagnosticCode = DiagnosticCode::YamlParseError;
    let expected_file = "models/retired_data_latency.sql";
    let expected_text = "mutation_profile.lateness";

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
            if d.code != Some(TARGET_CODE) || !d.message.contains(expected_text) {
                continue;
            }
            if is_target {
                target.push(d.clone());
            } else {
                other.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if d.0.code != Some(TARGET_CODE) || !d.0.message.contains(expected_text) {
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
        "expected zero retired-data_latency diagnostics from files other than \
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
        "expected exactly 1 retired-data_latency diagnostic from '{expected_file}', got {}:\n  {}",
        target.len(),
        target
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// MP14 TDD: `examples/broken/models/maintenance_granularity_mismatch.sql` —
/// a `grain: partition` model declaring `granularity: hour` while its own
/// `order_date` projection only truncates to `day` — a narrowing declaration
/// (finer than what the grouping actually derives), refused with
/// `MaintenanceGranularityMismatch` and no other maintenance diagnostic
/// firing anywhere else in the shared `examples/broken/` workspace.
///
/// Spec: `docs/specs/incremental_models.md` §Design "Grain is declared" /
/// "Widen-never-narrow".
#[test]
fn broken_workspace_maintenance_granularity_mismatch() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticCode, Workspace};

    const GRANULARITY_CODES: &[DiagnosticCode] = &[DiagnosticCode::MaintenanceGranularityMismatch];
    let expected_file = "models/maintenance_granularity_mismatch.sql";

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
            if !d
                .code
                .as_ref()
                .is_some_and(|c| GRANULARITY_CODES.contains(c))
            {
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
        "expected zero MaintenanceGranularityMismatch diagnostics from files other than \
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
        "expected exactly 1 MaintenanceGranularityMismatch from '{expected_file}', got {}:\n  {}",
        target.len(),
        target
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    assert_eq!(
        target[0].code,
        Some(DiagnosticCode::MaintenanceGranularityMismatch)
    );
}

/// `docs/outcomes/20260906-scd2-keyed-succession/phases/03a-plan.md` test 7:
/// eleven `examples/broken/models/succession_*.sql` fixtures, one per
/// `Succession*` diagnostic code — for each, exactly its own code fires at
/// that file and at no other file in the shared `examples/broken/`
/// workspace. The advisory fixture
/// (`succession_pre_filter_negates_flag.sql`) is `Warning` severity and
/// reports no `Succession*` Error at its own file.
#[test]
fn broken_workspace_succession_codes() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticCode, DiagnosticSeverity, Workspace};

    const SUCCESSION_CODES: &[DiagnosticCode] = &[
        DiagnosticCode::SuccessionWindowFunctionNotLead,
        DiagnosticCode::SuccessionPartitionKeyMismatch,
        DiagnosticCode::SuccessionOrderNotMonotoneClock,
        DiagnosticCode::SuccessionRowLocalColumnViolation,
        DiagnosticCode::SuccessionIdentityNotProjected,
        DiagnosticCode::SuccessionSingleSourceOnly,
        DiagnosticCode::SuccessionDrivingSourceNotAppendOnly,
        DiagnosticCode::SuccessionPreFilterNotRowLocal,
        DiagnosticCode::SuccessionDeleteFilterMisplaced,
        DiagnosticCode::SuccessionPreFilterNegatesFlag,
        DiagnosticCode::SuccessionPatternUnrecognized,
    ];

    // (file, expected code, expected severity)
    let expectations: &[(&str, DiagnosticCode, DiagnosticSeverity)] = &[
        (
            "models/succession_window_function_not_lead.sql",
            DiagnosticCode::SuccessionWindowFunctionNotLead,
            DiagnosticSeverity::Error,
        ),
        (
            "models/succession_partition_key_mismatch.sql",
            DiagnosticCode::SuccessionPartitionKeyMismatch,
            DiagnosticSeverity::Error,
        ),
        (
            "models/succession_order_not_monotone_clock.sql",
            DiagnosticCode::SuccessionOrderNotMonotoneClock,
            DiagnosticSeverity::Error,
        ),
        (
            "models/succession_row_local_column_violation.sql",
            DiagnosticCode::SuccessionRowLocalColumnViolation,
            DiagnosticSeverity::Error,
        ),
        (
            "models/succession_identity_not_projected.sql",
            DiagnosticCode::SuccessionIdentityNotProjected,
            DiagnosticSeverity::Error,
        ),
        (
            "models/succession_single_source_only.sql",
            DiagnosticCode::SuccessionSingleSourceOnly,
            DiagnosticSeverity::Error,
        ),
        (
            "models/succession_driving_source_not_append_only.sql",
            DiagnosticCode::SuccessionDrivingSourceNotAppendOnly,
            DiagnosticSeverity::Error,
        ),
        (
            "models/succession_pre_filter_not_row_local.sql",
            DiagnosticCode::SuccessionPreFilterNotRowLocal,
            DiagnosticSeverity::Error,
        ),
        (
            "models/succession_delete_filter_misplaced.sql",
            DiagnosticCode::SuccessionDeleteFilterMisplaced,
            DiagnosticSeverity::Error,
        ),
        (
            "models/succession_pre_filter_negates_flag.sql",
            DiagnosticCode::SuccessionPreFilterNegatesFlag,
            DiagnosticSeverity::Warning,
        ),
        (
            "models/succession_pattern_unrecognized.sql",
            DiagnosticCode::SuccessionPatternUnrecognized,
            DiagnosticSeverity::Error,
        ),
    ];

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

    // file -> every succession diagnostic that fired there
    let mut by_file: std::collections::HashMap<String, Vec<smelt_db::Diagnostic>> =
        std::collections::HashMap::new();

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
            .to_string()
            .replace('\\', "/");
        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !d
                .code
                .as_ref()
                .is_some_and(|c| SUCCESSION_CODES.contains(c))
            {
                continue;
            }
            by_file.entry(rel.clone()).or_default().push(d.clone());
        }
    }

    for (expected_file, expected_code, expected_severity) in expectations {
        let diags = by_file.get(*expected_file).cloned().unwrap_or_default();
        assert_eq!(
            diags.len(),
            1,
            "expected exactly 1 succession diagnostic from '{expected_file}', got {}:\n  {}",
            diags.len(),
            diags
                .iter()
                .map(|d| format!("[{:?}]: {}", d.code, d.message))
                .collect::<Vec<_>>()
                .join("\n  ")
        );
        assert_eq!(
            diags[0].code,
            Some(*expected_code),
            "wrong code for '{expected_file}'"
        );
        assert_eq!(
            diags[0].severity, *expected_severity,
            "wrong severity for '{expected_file}'"
        );
    }

    let expected_files: std::collections::HashSet<&str> =
        expectations.iter().map(|(f, ..)| *f).collect();
    let stray: Vec<(String, smelt_db::Diagnostic)> = by_file
        .into_iter()
        .filter(|(f, _)| !expected_files.contains(f.as_str()))
        .flat_map(|(f, ds)| ds.into_iter().map(move |d| (f.clone(), d)))
        .collect();
    assert!(
        stray.is_empty(),
        "expected zero succession diagnostics from files other than the eleven fixtures, got {}:\n  {}",
        stray.len(),
        stray
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}
