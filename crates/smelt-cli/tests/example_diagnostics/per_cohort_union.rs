use crate::support::*;
use crate::support_ext::*;

/// Phase E2 TDD: `examples/per_cohort_union/` produces zero diagnostics.
#[test]
fn per_cohort_union_example_has_zero_diagnostics() {
    check_workspace_no_diagnostics("examples/per_cohort_union");
}

/// Phase 2 (VALUES-derived-table-typing) TDD item 3: `examples/per_cohort_union/models/orders.sql`
/// exports five concrete-typed columns via a VALUES-derived table.
///
/// `orders.sql` selects from a `(VALUES (…), …) AS t(id, user_id, region, revenue, created_at)`
/// subquery. Phase 2 integrates VALUES-column typing into `typed_model_schema`, so each of the
/// five columns must resolve to a concrete, non-Unknown type.
///
/// Note: `all_cohorts_unioned.sql` and the generator-emitted cohort models are NOT asserted here
/// — their schemas are still Unknown because they depend on generator-emission schema inference,
/// which is out of scope for this phase (tracked in `docs/specs/meta_language.md`).
#[test]
fn per_cohort_union_orders_has_concrete_typed_schema() {
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

    // Find orders.sql
    let orders_path = path.join("models").join("orders.sql");
    let orders_file = db
        .source_file(&orders_path)
        .expect("orders.sql not found in workspace");

    let schema = smelt_db::typed_model_schema(&db, ws, orders_file);

    assert_eq!(
        schema.columns.len(),
        5,
        "orders.sql should export exactly 5 columns, got {}:\n  {:?}",
        schema.columns.len(),
        schema.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
    );

    for col in &schema.columns {
        let typed = col.data_type.as_ref().unwrap_or_else(|| {
            panic!(
                "orders.sql column '{}' has no TypedColumn (data_type is None)",
                col.name
            )
        });
        assert_ne!(
            typed.data_type,
            smelt_db::DataType::unknown_dynamic(),
            "orders.sql column '{}' should have a concrete type, got Unknown",
            col.name
        );
    }
}

/// Phase E2 TDD: `examples/staging_from_sources/` produces zero diagnostics.
#[test]
fn staging_from_sources_example_has_zero_diagnostics() {
    check_workspace_no_diagnostics("examples/staging_from_sources");
}

/// D5 probe (clean path): `seed_source_type_join` joins a seed (inferred types
/// via CSV + sidecar using aliases INT/BOOL/DECIMAL) with a source (declared
/// aliases INT8/TIMESTAMPTZ/TEXT). All type aliases must resolve to recognised
/// smelt DataTypes; no LSP diagnostics must fire.
#[test]
fn seed_source_type_join_has_zero_diagnostics() {
    check_workspace_no_diagnostics("examples/seed_source_type_join");
}

/// Non-ASCII fixture: `examples/non_ascii_columns/` has Greek-letter column
/// aliases and must produce zero diagnostics.
#[test]
fn non_ascii_columns_example_has_zero_diagnostics() {
    check_workspace_no_diagnostics("examples/non_ascii_columns");
}

/// Phase E2 TDD: `GeneratesUnknownValue` — `generates: views` fires.
#[test]
fn per_cohort_union_broken_generates_unknown_value() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_generates_unknown_value",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::GeneratesUnknownValue,
    );
}

/// Phase E2 TDD: `GeneratesMixedWithBareModel` — `generates: models` combined with `name:`.
#[test]
fn per_cohort_union_broken_generates_mixed_with_name_field() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_generates_mixed_with_name_field",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::GeneratesMixedWithBareModel,
    );
}

/// Phase E2 TDD: `GeneratesMixedWithBareModel` — `generates: models` combined with section delimiter.
#[test]
fn per_cohort_union_broken_generates_mixed_with_section_delimiter() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_generates_mixed_with_section_delimiter",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::GeneratesMixedWithBareModel,
    );
}

/// Phase E2 TDD: `GenerateFileBareSelectForbidden` — bare SELECT in generator body.
#[test]
fn per_cohort_union_broken_generate_file_bare_select_forbidden() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_generate_file_bare_select_forbidden",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::GenerateFileBareSelectForbidden,
    );
}

/// Phase E2 TDD: `GenerateFileBodyTypeError` — body type is not `List<ModelDef>`.
#[test]
fn per_cohort_union_broken_generate_file_body_type_error() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_generate_file_body_type_error",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::GenerateFileBodyTypeError,
    );
}

/// Phase E2 TDD: `ModelDefOutsideGeneratorFile` — `ModelDef` in a non-generator file.
#[test]
fn per_cohort_union_broken_model_def_outside_generator_file() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_model_def_outside_generator_file",
        "models/broken.sql",
        smelt_db::DiagnosticCode::ModelDefOutsideGeneratorFile,
    );
}

/// Phase E2 TDD: `ModelDefInvalidName` — name with non-path-safe characters.
#[test]
fn per_cohort_union_broken_model_def_invalid_name() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_model_def_invalid_name",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::ModelDefInvalidName,
    );
}

/// Phase E2 TDD: `ModelDefInvalidMaterialization` — materialization not in closed set.
#[test]
fn per_cohort_union_broken_model_def_invalid_materialization() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_model_def_invalid_materialization",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::ModelDefInvalidMaterialization,
    );
}

/// Phase E2 TDD: `ModelDefDuplicateName` — two ModelDefs with the same name.
#[test]
fn per_cohort_union_broken_model_def_duplicate_name() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_model_def_duplicate_name",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::ModelDefDuplicateName,
    );
}

/// Phase E2 TDD: `ModelDefHandAuthoredCollision` — generator emission collides
/// with a hand-authored model `models/collision/my_model.sql`.
#[test]
fn per_cohort_union_broken_model_def_hand_authored_collision() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_model_def_hand_authored_collision",
        "models/collision.gen.sql",
        smelt_db::DiagnosticCode::ModelDefHandAuthoredCollision,
    );
}

/// Phase E2 TDD: `GeneratorBodyForbidsModelReflection` — `smelt.models.with_tag`
/// inside a generator body.
#[test]
fn per_cohort_union_broken_generator_body_forbids_model_reflection() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_generator_body_forbids_model_reflection",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::GeneratorBodyForbidsModelReflection,
    );
}

/// Phase F TDD: `examples/meta_polish/` produces zero diagnostics.
/// Exercises the `concat_with(sep)` parameterised reducer and the
/// `if cond then a else b` ternary with a Boolean-literal condition.
#[test]
fn meta_polish_clean_workspace() {
    check_workspace_no_diagnostics("examples/meta_polish");
}

/// Phase F TDD: `examples/meta_polish_broken_ternary_non_boolean_cond/` produces
/// exactly one `TernaryConditionNotBoolean` diagnostic.
#[test]
fn meta_polish_broken_ternary_non_boolean_cond() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_polish_broken_ternary_non_boolean_cond",
        "models/ternary_non_boolean_cond.sql",
        smelt_db::DiagnosticCode::TernaryConditionNotBoolean,
    );
}

/// Phase F TDD: `examples/meta_polish_broken_ternary_branch_mismatch/` produces
/// exactly one `TernaryBranchTypeMismatch` diagnostic.
#[test]
fn meta_polish_broken_ternary_branch_mismatch() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_polish_broken_ternary_branch_mismatch",
        "models/ternary_branch_mismatch.sql",
        smelt_db::DiagnosticCode::TernaryBranchTypeMismatch,
    );
}

/// Phase F TDD: `examples/meta_polish_broken_reducer_arity/` produces
/// exactly one `ReducerArityMismatch` diagnostic.
#[test]
fn meta_polish_broken_reducer_arity() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_polish_broken_reducer_arity",
        "models/reducer_arity.sql",
        smelt_db::DiagnosticCode::ReducerArityMismatch,
    );
}
