use crate::support::*;
use crate::support_ext::*;

/// Phase E1 TDD: `examples/meta_config/` produces zero diagnostics.
/// Exercises smelt.record declaration + inline record types + YAML loader
/// (`Map<Text, Record>` and `List<Record>` root shapes) + per-target overlay.
/// HOF consumption of loader results (`m.entries() |> map(fn e => …)`) is
/// covered by unit tests in `smelt-db::type_inference`; production wiring of
/// the loader-result type through SELECT expressions is a known follow-up
/// (see `docs/plans/20260509-meta-language-E1.md` Deferred — Phase 5).
#[test]
fn meta_config_clean_workspace() {
    check_workspace_no_diagnostics_with_loaders("examples/meta_config");
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_path_not_literal/`
/// produces exactly one `ConfigLoaderPathNotLiteral` diagnostic.
#[test]
fn meta_config_broken_config_loader_path_not_literal() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_path_not_literal",
        "models/config_loader_path_not_literal.sql",
        smelt_db::DiagnosticCode::ConfigLoaderPathNotLiteral,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_path_escapes_workspace/`
/// produces exactly one `ConfigLoaderPathEscapesWorkspace` diagnostic.
#[test]
fn meta_config_broken_config_loader_path_escapes_workspace() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_path_escapes_workspace",
        "models/config_loader_path_escapes_workspace.sql",
        smelt_db::DiagnosticCode::ConfigLoaderPathEscapesWorkspace,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_path_backslash/`
/// produces exactly one `ConfigLoaderPathBackslash` diagnostic.
#[test]
fn meta_config_broken_config_loader_path_backslash() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_path_backslash",
        "models/config_loader_path_backslash.sql",
        smelt_db::DiagnosticCode::ConfigLoaderPathBackslash,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_file_not_found/`
/// produces exactly one `ConfigLoaderFileNotFound` diagnostic.
#[test]
fn meta_config_broken_config_loader_file_not_found() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_file_not_found",
        "models/config_loader_file_not_found.sql",
        smelt_db::DiagnosticCode::ConfigLoaderFileNotFound,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_schema_forbidden/`
/// produces exactly one `ConfigLoaderSchemaForbidden` diagnostic.
#[test]
fn meta_config_broken_config_loader_schema_forbidden() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_schema_forbidden",
        "models/config_loader_schema_forbidden.sql",
        smelt_db::DiagnosticCode::ConfigLoaderSchemaForbidden,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_toml_not_yet_supported/`
/// produces exactly one `ConfigLoaderTomlNotYetSupported` diagnostic.
#[test]
fn meta_config_broken_config_loader_toml_not_yet_supported() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_toml_not_yet_supported",
        "models/config_loader_toml_not_yet_supported.sql",
        smelt_db::DiagnosticCode::ConfigLoaderTomlNotYetSupported,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_parse_error/`
/// produces exactly one `ConfigLoaderParseError` diagnostic.
#[test]
fn meta_config_broken_config_loader_parse_error() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_parse_error",
        "models/config_loader_parse_error.sql",
        smelt_db::DiagnosticCode::ConfigLoaderParseError,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_required_field_missing/`
/// produces exactly one `ConfigLoaderRequiredFieldMissing` diagnostic.
#[test]
fn meta_config_broken_config_loader_required_field_missing() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_required_field_missing",
        "models/config_loader_required_field_missing.sql",
        smelt_db::DiagnosticCode::ConfigLoaderRequiredFieldMissing,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_unknown_field/`
/// produces exactly one `ConfigLoaderUnknownField` diagnostic.
#[test]
fn meta_config_broken_config_loader_unknown_field() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_unknown_field",
        "models/config_loader_unknown_field.sql",
        smelt_db::DiagnosticCode::ConfigLoaderUnknownField,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_type_mismatch/`
/// produces exactly one `ConfigLoaderTypeMismatch` diagnostic.
#[test]
fn meta_config_broken_config_loader_type_mismatch() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_type_mismatch",
        "models/config_loader_type_mismatch.sql",
        smelt_db::DiagnosticCode::ConfigLoaderTypeMismatch,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_root_shape_mismatch/`
/// produces exactly one `ConfigLoaderRootShapeMismatch` diagnostic.
#[test]
fn meta_config_broken_config_loader_root_shape_mismatch() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_root_shape_mismatch",
        "models/config_loader_root_shape_mismatch.sql",
        smelt_db::DiagnosticCode::ConfigLoaderRootShapeMismatch,
    );
}

/// Phase E1 TDD: `ConfigLoaderDuplicateMapKey` — deferred from E2E fixture coverage.
///
/// `marked_yaml` (and `serde_json`) silently deduplicate keys when
/// `error_on_duplicate_keys` is `false` (the default). The validator's
/// `seen_keys` map never observes a second entry for the same key, so no
/// diagnostic fires via a real YAML/JSON file on disk. This code is covered
/// by the synthetic unit test `loader::tests::yaml_parse_map_root_emits_duplicate_key`
/// which injects a `ParsedNode::Mapping` with repeated keys directly.
///
/// To promote this to a live E2E fixture, the `parse_yaml` loader would need
/// to enable `error_on_duplicate_keys` and map the `DuplicateKey` load error
/// to `ConfigLoaderDuplicateMapKey` instead of `ConfigLoaderParseError`.
/// Deferred to a future loader spec edit.
#[test]
#[ignore = "ConfigLoaderDuplicateMapKey cannot be triggered via real YAML/JSON files \
             because YAML/JSON parsers silently deduplicate keys; covered by \
             loader::tests::yaml_parse_map_root_emits_duplicate_key unit test"]
fn meta_config_broken_config_loader_duplicate_map_key() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_duplicate_map_key",
        "models/config_loader_duplicate_map_key.sql",
        smelt_db::DiagnosticCode::ConfigLoaderDuplicateMapKey,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_null_coercion/`
/// produces exactly one `ConfigLoaderNullCoercion` diagnostic (warning severity).
#[test]
fn meta_config_broken_config_loader_null_coercion() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_null_coercion",
        "models/config_loader_null_coercion.sql",
        smelt_db::DiagnosticCode::ConfigLoaderNullCoercion,
    );
}

/// BUG-014 P4 TDD: A schema-violating overlay file surfaces
/// `ConfigLoaderUnknownField` anchored at the generator call site.
///
/// `examples/meta_config_overlay_probe_invalid/` has `target: prod` in
/// `smelt.yml` so the overlay `cohorts.prod.yaml` is always active.  That
/// overlay contains `extra_field` (not in the schema) → exactly one
/// `ConfigLoaderUnknownField` must fire for `models/cohorts.gen.sql`.
#[test]
fn overlay_probe_invalid_overlay_emits_unknown_field() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_overlay_probe_invalid",
        "models/cohorts.gen.sql",
        smelt_db::DiagnosticCode::ConfigLoaderUnknownField,
    );
}
