use crate::support::*;
use crate::support_ext::*;

/// Phase D TDD: `examples/meta_workspace/` produces zero diagnostics.
/// Exercises smelt.models.with_tag, smelt.models.all, smelt.sources.with_tag,
/// smelt.sources.all, ModelRef field projection, and SourceRef field projection.
#[test]
fn meta_workspace_clean_workspace() {
    check_workspace_no_diagnostics("examples/meta_workspace");
}

/// Phase D TDD: `examples/meta_workspace_broken_with_tag_requires_text/` produces
/// exactly one `WithTagRequiresText` diagnostic.
#[test]
fn meta_workspace_broken_with_tag_requires_text() {
    check_workspace_emits_exactly_one_phase_d_diagnostic(
        "examples/meta_workspace_broken_with_tag_requires_text",
        "models/with_tag_requires_text.sql",
        smelt_db::DiagnosticCode::WithTagRequiresText,
    );
}

/// Phase D TDD: `examples/meta_workspace_broken_with_tag_named_argument/` produces
/// exactly one `WithTagNamedArgument` diagnostic.
#[test]
fn meta_workspace_broken_with_tag_named_argument() {
    check_workspace_emits_exactly_one_phase_d_diagnostic(
        "examples/meta_workspace_broken_with_tag_named_argument",
        "models/with_tag_named_argument.sql",
        smelt_db::DiagnosticCode::WithTagNamedArgument,
    );
}

/// Phase D TDD: `examples/meta_workspace_broken_wide_reflection_unknown_accessor/`
/// produces exactly one `WideReflectionUnknownAccessor` diagnostic.
#[test]
fn meta_workspace_broken_wide_reflection_unknown_accessor() {
    check_workspace_emits_exactly_one_phase_d_diagnostic(
        "examples/meta_workspace_broken_wide_reflection_unknown_accessor",
        "models/wide_reflection_unknown_accessor.sql",
        smelt_db::DiagnosticCode::WideReflectionUnknownAccessor,
    );
}

/// Phase D TDD: `examples/meta_workspace_broken_wide_reflection_unexpected_argument/`
/// produces exactly one `WideReflectionUnexpectedArgument` diagnostic.
#[test]
fn meta_workspace_broken_wide_reflection_unexpected_argument() {
    check_workspace_emits_exactly_one_phase_d_diagnostic(
        "examples/meta_workspace_broken_wide_reflection_unexpected_argument",
        "models/wide_reflection_unexpected_argument.sql",
        smelt_db::DiagnosticCode::WideReflectionUnexpectedArgument,
    );
}

/// Phase D TDD: `examples/meta_workspace_broken_model_ref_field_unknown/` produces
/// exactly one `ModelRefFieldUnknown` diagnostic.
#[test]
fn meta_workspace_broken_model_ref_field_unknown() {
    check_workspace_emits_exactly_one_phase_d_diagnostic(
        "examples/meta_workspace_broken_model_ref_field_unknown",
        "models/model_ref_field_unknown.sql",
        smelt_db::DiagnosticCode::ModelRefFieldUnknown,
    );
}

/// Phase D TDD: `examples/meta_workspace_broken_source_ref_field_unknown/` produces
/// exactly one `SourceRefFieldUnknown` diagnostic.
#[test]
fn meta_workspace_broken_source_ref_field_unknown() {
    check_workspace_emits_exactly_one_phase_d_diagnostic(
        "examples/meta_workspace_broken_source_ref_field_unknown",
        "models/source_ref_field_unknown.sql",
        smelt_db::DiagnosticCode::SourceRefFieldUnknown,
    );
}
