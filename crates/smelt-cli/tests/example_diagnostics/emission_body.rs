use crate::support::*;
use crate::support_ext::*;

/// Emission-body TDD: `examples/per_cohort_union_broken_emission_body_undeclared_column/`
/// produces exactly one `UndeclaredColumn` diagnostic anchored in the generator file body.
/// The body references `nonexistent_column` which is not declared in `smelt.orders`.
#[test]
fn per_cohort_union_broken_emission_body_undeclared_column() {
    check_workspace_emits_exactly_one_emission_body_diagnostic(
        "examples/per_cohort_union_broken_emission_body_undeclared_column",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::UndeclaredColumn,
    );

    // Position regression: the diagnostic must land inside the body (line >= 3,
    // 0-indexed), not at the YAML frontmatter delimiter (line 0).
    // The fixture body is on line 3 of broken.gen.sql:
    //   line 0: ---
    //   line 1: generates: models
    //   line 2: ---
    //   line 3: [ModelDef { name: 'x', body: SELECT nonexistent_column FROM smelt.orders }]
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let example_dir = "examples/per_cohort_union_broken_emission_body_undeclared_column";
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();
    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut undeclared_diags: Vec<smelt_db::Diagnostic> = Vec::new();
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
        if !rel.replace('\\', "/").ends_with("models/broken.gen.sql") {
            continue;
        }
        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if d.code == Some(smelt_db::DiagnosticCode::UndeclaredColumn) {
                undeclared_diags.push(d.clone());
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if d.0.code == Some(smelt_db::DiagnosticCode::UndeclaredColumn) {
                undeclared_diags.push(d.0.clone());
            }
        }
    }

    assert_eq!(
        undeclared_diags.len(),
        1,
        "expected exactly one UndeclaredColumn diagnostic"
    );
    // Convert the TextRange byte-offset to a line number for the assertion.
    let broken_model_path = path.join("models/broken.gen.sql");
    let broken_text =
        std::fs::read_to_string(&broken_model_path).expect("could not read broken.gen.sql");
    let li = LineIndex::new(&broken_text);
    let diag_start_line = li.line_col(undeclared_diags[0].range.start()).line;
    assert!(
        diag_start_line >= 3,
        "expected UndeclaredColumn diagnostic anchored inside the body (line >= 3, 0-indexed), \
         got line {} — diagnostic is likely pinned to the frontmatter delimiter (line 0) \
         rather than the body content",
        diag_start_line,
    );
}

/// Emission-body TDD: `examples/per_cohort_union_broken_emission_body_parse_error/`
/// produces exactly one `ParseError` diagnostic anchored at the body span.
/// The body contains a SQL syntax error (`SELEKT` keyword typo).
#[test]
fn per_cohort_union_broken_emission_body_parse_error() {
    check_workspace_emits_exactly_one_emission_body_diagnostic(
        "examples/per_cohort_union_broken_emission_body_parse_error",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::ParseError,
    );
}

/// Emission-body TDD: `examples/per_cohort_union_broken_emission_body_cte_cycle/`
/// produces exactly one `CteCycle` diagnostic anchored in the generator body's WITH clause.
#[test]
fn per_cohort_union_broken_emission_body_cte_cycle() {
    check_workspace_emits_exactly_one_emission_body_diagnostic(
        "examples/per_cohort_union_broken_emission_body_cte_cycle",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::CteCycle,
    );
}

/// Emission-body TDD: discarded-emission suppression.
///
/// `examples/per_cohort_union_broken_emission_body_collision_suppression/` declares a
/// generator that emits two `ModelDef`s with the same name `dup`, where the second body
/// also contains an `UndeclaredColumn` reference. Exactly one `ModelDefDuplicateName`
/// diagnostic fires (the W3 collision diagnostic); zero `UndeclaredColumn` diagnostics
/// fire because the discarded emission's body is not analysed.
#[test]
fn per_cohort_union_broken_emission_body_collision_suppression() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let example_dir = "examples/per_cohort_union_broken_emission_body_collision_suppression";
    let expected_gen = "models/broken.gen.sql";

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut duplicate_name_diags: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut undeclared_column_diags: Vec<smelt_db::Diagnostic> = Vec::new();

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            match d.code {
                Some(smelt_db::DiagnosticCode::ModelDefDuplicateName) => {
                    duplicate_name_diags.push(d.clone());
                }
                Some(smelt_db::DiagnosticCode::UndeclaredColumn) => {
                    undeclared_column_diags.push(d.clone());
                }
                _ => {}
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            match d.0.code {
                Some(smelt_db::DiagnosticCode::ModelDefDuplicateName) => {
                    duplicate_name_diags.push(d.0.clone());
                }
                Some(smelt_db::DiagnosticCode::UndeclaredColumn) => {
                    undeclared_column_diags.push(d.0.clone());
                }
                _ => {}
            }
        }
    }

    // Must have exactly one ModelDefDuplicateName.
    assert_eq!(
        duplicate_name_diags.len(),
        1,
        "expected exactly 1 ModelDefDuplicateName in {}/{}, got {}:\n  {}",
        example_dir,
        expected_gen,
        duplicate_name_diags.len(),
        duplicate_name_diags
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    // Must have zero UndeclaredColumn (discarded emission body not analysed).
    assert!(
        undeclared_column_diags.is_empty(),
        "expected zero UndeclaredColumn diagnostics (discarded emission body must not be analysed) \
         in {}, got {}:\n  {}",
        example_dir,
        undeclared_column_diags.len(),
        undeclared_column_diags
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// Regression: each of `cohorts.us_west`, `cohorts.us_east`, and `cohorts.eu`
/// emitted by `examples/per_cohort_union/models/cohorts.gen.sql` must have
/// five typed (non-Unknown) columns after Phase 1 lands.
///
/// Prior to Phase 1 the emitted-model typed-schema query did not exist and
/// emitted models carried no column type information.
#[test]
fn per_cohort_union_emitted_cohorts_have_typed_schemas() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::Workspace;
    use std::path::Path;

    let example_dir = "examples/per_cohort_union";
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let gen_path = path.join("models").join("cohorts.gen.sql");
    let gen_file = db
        .source_file(&gen_path)
        .expect("cohorts.gen.sql registered in workspace");

    // Each cohort emission must have 5 typed (non-Unknown) columns.
    let emission_names = ["us_west", "us_east", "eu"];
    for name in &emission_names {
        let schema = smelt_db::emitted_model_typed_schema(&db, ws, gen_file, name.to_string());
        assert_eq!(
            schema.columns.len(),
            5,
            "emission '{}' should have 5 columns, got {}: {:?}",
            name,
            schema.columns.len(),
            schema.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
        for col in &schema.columns {
            let dt = col
                .data_type
                .as_ref()
                .map(|tc| tc.data_type.clone())
                .unwrap_or(smelt_db::DataType::unknown_dynamic());
            assert_ne!(
                dt,
                smelt_db::DataType::unknown_dynamic(),
                "emission '{}' column '{}' should be concrete, got Unknown",
                name,
                col.name
            );
        }
    }
}

/// CLI gate: `examples/non_ascii_broken/` produces exactly one `UndeclaredColumn`
/// diagnostic anchored in `models/broken.gen.sql`.
///
/// The generator body `SELECT 1 AS α, nonexistent_column FROM smelt.upstream`
/// uses a 2-byte Greek letter (α, U+03B1) before the undeclared column, so the
/// byte-column position of the diagnostic differs from the UTF-16 position.
/// This fixture is used by the E2E position-encoding test to verify that
/// `publishDiagnostics` emits the correct `character` field under both encodings.
#[test]
fn non_ascii_broken_undeclared_column() {
    check_workspace_emits_exactly_one_emission_body_diagnostic(
        "examples/non_ascii_broken",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::UndeclaredColumn,
    );
}

/// U5 TDD: `examples/frontmatter_function_key_on_model/` emits exactly one
/// Warning-severity `FrontmatterParseError` (inapplicable `deterministic` key)
/// and zero Error-severity diagnostics.  The model's `materialization: table`
/// must be retained — the block is not dropped.
#[test]
fn frontmatter_function_key_on_model_emits_warning_not_error() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, DiagnosticSeverity, Workspace};
    use std::path::Path;

    let example_dir = "examples/frontmatter_function_key_on_model";
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut all_errors: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut fm_warnings: Vec<smelt_db::Diagnostic> = Vec::new();

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            match d.severity {
                DiagnosticSeverity::Error => all_errors.push(d.clone()),
                DiagnosticSeverity::Warning
                    if d.code == Some(smelt_db::DiagnosticCode::FrontmatterParseError) =>
                {
                    fm_warnings.push(d.clone())
                }
                _ => {}
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            match d.0.severity {
                DiagnosticSeverity::Error => all_errors.push(d.0.clone()),
                DiagnosticSeverity::Warning
                    if d.0.code == Some(smelt_db::DiagnosticCode::FrontmatterParseError) =>
                {
                    fm_warnings.push(d.0.clone())
                }
                _ => {}
            }
        }
    }

    assert!(
        all_errors.is_empty(),
        "expected zero Error-severity diagnostics in {}, got {}:\n  {}",
        example_dir,
        all_errors.len(),
        all_errors
            .iter()
            .map(|d| format!("[{:?}] {:?}: {}", d.severity, d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        fm_warnings.len(),
        1,
        "expected exactly 1 FrontmatterParseError Warning in {}, got {}:\n  {}",
        example_dir,
        fm_warnings.len(),
        fm_warnings
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert!(
        fm_warnings[0].message.contains("deterministic"),
        "warning message must name the inapplicable key 'deterministic'; got: {}",
        fm_warnings[0].message
    );
}

/// U5 TDD: `examples/timeseries_broken_invalid_granularity/` emits exactly one
/// `MalformedTimeseries` Error — `granularity: fortnight` is not a valid value.
/// BUG-023 end-to-end regression.
#[test]
fn timeseries_broken_invalid_granularity_emits_malformed_timeseries() {
    check_workspace_emits_timeseries_diagnostic(
        "examples/timeseries_broken_invalid_granularity",
        "models/invalid_granularity.sql",
        smelt_db::DiagnosticCode::MalformedTimeseries,
    );
}

/// U5 TDD: `examples/timeseries_broken_unknown_key/` emits exactly one
/// `MalformedTimeseries` Error — unknown sub-key `partition_columm` (typo).
/// BUG-025 end-to-end regression.
#[test]
fn timeseries_broken_unknown_key_emits_malformed_timeseries() {
    check_workspace_emits_timeseries_diagnostic(
        "examples/timeseries_broken_unknown_key",
        "models/unknown_subkey.sql",
        smelt_db::DiagnosticCode::MalformedTimeseries,
    );
}

/// D-52 rule 7 e2e: `examples/timeseries_broken_nullable_partition/` emits
/// `MalformedTimeseries` — `partition_date` is `CAST(event_ts AS DATE)` which
/// the type inferencer conservatively marks nullable (unknown upstream → true).
#[test]
fn timeseries_broken_nullable_partition_emits_malformed_timeseries() {
    check_workspace_emits_timeseries_diagnostic(
        "examples/timeseries_broken_nullable_partition",
        "models/nullable_partition.sql",
        smelt_db::DiagnosticCode::MalformedTimeseries,
    );
}

/// D-52 rule 8 e2e: `examples/timeseries_broken_hour_date_partition/` emits
/// `MalformedTimeseries` — `partition_date` is a DATE column but `granularity: hour`
/// requires a TIMESTAMP/TIMESTAMPTZ partition column.
#[test]
fn timeseries_broken_hour_date_partition_emits_malformed_timeseries() {
    check_workspace_emits_timeseries_diagnostic(
        "examples/timeseries_broken_hour_date_partition",
        "models/hourly_date_partition.sql",
        smelt_db::DiagnosticCode::MalformedTimeseries,
    );
}

/// U5 TDD: `examples/frontmatter_broken_unknown_key/` emits exactly one
/// `FrontmatterParseError` Error — `mateializaton` is an unknown top-level key.
/// BUG-016 end-to-end regression.
#[test]
fn frontmatter_broken_unknown_key_emits_frontmatter_parse_error() {
    check_workspace_emits_exactly_one_emission_body_diagnostic(
        "examples/frontmatter_broken_unknown_key",
        "models/unknown_key.sql",
        smelt_db::DiagnosticCode::FrontmatterParseError,
    );
}

/// BUG-007 TDD: `examples/expansion_broken_cte_caller_collision/` emits exactly
/// one `CteShadowsCallerCte` Error on `models/collision_model.sql`.
///
/// The model declares a CTE `helper` and calls `smelt.functions.with_helper`
/// whose body also declares a CTE `helper`. At codegen time the two CTEs
/// would collide; the analysis-time check refuses with `CteShadowsCallerCte`.
#[test]
fn expansion_broken_cte_caller_collision_emits_cte_shadows_caller_cte() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let example_dir = "examples/expansion_broken_cte_caller_collision";
    let expected_file = "models/collision_model.sql";

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut target_diags: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_diags: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_cte_shadow = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code.is_some_and(|c| *c == smelt_db::DiagnosticCode::CteShadowsCallerCte)
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
        let is_target = rel
            .replace('\\', "/")
            .ends_with(&expected_file.replace('\\', "/"));

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !is_cte_shadow(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_diags.push(d.clone());
            } else {
                other_diags.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_cte_shadow(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_diags.push(d.0.clone());
            } else {
                other_diags.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_diags.is_empty(),
        "expected zero CteShadowsCallerCte diagnostics from files other than '{}', got {}:\n  {}",
        expected_file,
        other_diags.len(),
        other_diags
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags.len(),
        1,
        "expected exactly 1 CteShadowsCallerCte diagnostic from '{}', got {}:\n  {}",
        expected_file,
        target_diags.len(),
        target_diags
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags[0].code,
        Some(smelt_db::DiagnosticCode::CteShadowsCallerCte),
        "expected CteShadowsCallerCte, got {:?}: {}",
        target_diags[0].code,
        target_diags[0].message
    );
}

/// BUG-017: `examples/types_broken_crossfamily_add/` must emit exactly one
/// `TypeMismatch` Error diagnostic (cross-family `42 + '3'` arithmetic).
#[test]
fn types_broken_crossfamily_add_emits_type_mismatch() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::Workspace;
    use std::path::Path;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/types_broken_crossfamily_add");

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();
    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let models = discovery.discover_models().unwrap();
    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut all_diags = Vec::new();
    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            all_diags.push(d.clone());
        }
    }

    let type_mismatches: Vec<_> = all_diags
        .iter()
        .filter(|d| d.code == Some(smelt_db::DiagnosticCode::TypeMismatch))
        .collect();

    assert_eq!(
        type_mismatches.len(),
        1,
        "examples/types_broken_crossfamily_add must emit exactly 1 TypeMismatch; got {}:\n  {}",
        all_diags.len(),
        all_diags
            .iter()
            .map(|d| format!("[{:?}] {:?}: {}", d.severity, d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    assert_eq!(
        type_mismatches[0].severity,
        smelt_db::DiagnosticSeverity::Error,
        "TypeMismatch for cross-family arithmetic must be Error severity"
    );
}

/// BUG-066 regression: a generator file whose filename does NOT end with
/// `.gen.sql` must be discovered as a generator (no parse errors) and must
/// emit models on `smelt build`.
///
/// Spec: meta_language.md §"Multi-model production" — "The `.gen.sql` extension
/// is a recommended convention; it is **not load-bearing**. The compiler
/// determines a file's status from the frontmatter alone."
#[test]
fn d3_meta_fn_config_generator_without_gen_suffix_no_diagnostics() {
    check_workspace_no_diagnostics("examples/d3_meta_fn_config");
}

/// §17 TDD: `examples/collation_clean/` produces zero diagnostics.
///
/// The model uses `COLLATE "C"` — a binary (portable) collation — so no
/// `NonPortableCollation` diagnostic should fire.
#[test]
fn collation_clean_workspace() {
    check_workspace_no_diagnostics("examples/collation_clean");
}

/// §17 TDD: `examples/collation_broken/` — `models/non_binary_collation.sql`
/// uses `COLLATE NOCASE` and must produce exactly one `NonPortableCollation`
/// Error diagnostic and no other diagnostics.
#[test]
fn collation_broken_non_binary() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};

    let example_dir = "examples/collation_broken";
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let models = discovery.discover_models().unwrap();

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut all_diags: Vec<smelt_db::Diagnostic> = Vec::new();
    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            all_diags.push(d.clone());
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            all_diags.push(d.0.clone());
        }
    }

    let collation_diags: Vec<_> = all_diags
        .iter()
        .filter(|d| d.code == Some(smelt_db::DiagnosticCode::NonPortableCollation))
        .collect();

    assert_eq!(
        collation_diags.len(),
        1,
        "expected exactly 1 NonPortableCollation diagnostic in {example_dir}; got {}:\n  {}",
        all_diags.len(),
        all_diags
            .iter()
            .map(|d| format!("[{:?}] {:?}: {}", d.severity, d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    assert_eq!(
        collation_diags[0].severity,
        smelt_db::DiagnosticSeverity::Error,
        "NonPortableCollation must be Error severity"
    );
    assert_eq!(
        all_diags.len(),
        1,
        "collation_broken must emit exactly 1 diagnostic total (no extra diagnostics); got {}:\n  {}",
        all_diags.len(),
        all_diags
            .iter()
            .map(|d| format!("[{:?}] {:?}: {}", d.severity, d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// §17 standing collation gate: `examples/collation_clean/` — including the
/// binary-string grouping and ordering model — produces zero diagnostics.
///
/// This test is the positive companion to `collation_broken_non_binary`.  It
/// covers `models/binary_groupby_orderby.sql` which uses `GROUP BY` and
/// `ORDER BY` on a `Text` column without any COLLATE clause (implicit binary)
/// alongside `models/binary_collation.sql` which uses an explicit `COLLATE "C"`.
/// Neither model should emit a `NonPortableCollation` diagnostic.
#[test]
fn collation_clean_binary_groupby_orderby() {
    check_workspace_no_diagnostics("examples/collation_clean");
}
