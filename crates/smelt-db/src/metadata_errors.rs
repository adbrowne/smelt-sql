//! `MetadataError` → `Diagnostic` mapping.
//!
//! Home of `map_metadata_error_to_diagnostic`, the compiler-enforced
//! exhaustiveness gate described in `docs/specs/architecture.md`
//! §"Fail-loud discipline". Pure; no Salsa access.

use smelt_core::metadata::MetadataError;

use crate::*;

/// Map a `MetadataError` to a `Diagnostic`, or `None` when the variant is
/// handled by a dedicated arm elsewhere in `check_file_diagnostics`.
///
/// **This match must remain exhaustive.** Every variant of `MetadataError` is
/// listed explicitly so the compiler refuses to compile when a new variant is
/// added without a corresponding handler. `None` arms are intentional: they
/// document that the variant is handled somewhere else (annotated inline).
/// This is the compiler-enforced gate for the fail-loud discipline —
/// `MetadataError` variant exhaustiveness rule (architecture.md §11).
pub(crate) fn map_metadata_error_to_diagnostic(err: &MetadataError) -> Option<Diagnostic> {
    match err {
        MetadataError::MalformedDelimiter(line) => Some(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!(
                "malformed multi-model section delimiter at line {line}: SQL content must be \
                 inside a '--- name: model_name ---' section; found non-section content before \
                 the first delimiter"
            ),
            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
            code: Some(DiagnosticCode::MalformedSectionDelimiter),
            data: None,
        }),
        MetadataError::UnclosedFrontmatter(_line) => Some(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: "frontmatter not closed: missing closing '---'".to_string(),
            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
            code: Some(DiagnosticCode::UnclosedFrontmatter),
            data: None,
        }),
        MetadataError::MissingModelName(section) => Some(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!("multi-model section {section} is missing a model name"),
            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
            code: Some(DiagnosticCode::MalformedSectionDelimiter),
            data: None,
        }),
        MetadataError::YamlParseError(e) => Some(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!("YAML parse error in frontmatter: {e}"),
            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
            code: Some(DiagnosticCode::YamlParseError),
            data: None,
        }),
        // Raised at extraction time (`fold_top_level_safety_overrides`), like
        // `YamlParseError` above — reuses its `DiagnosticCode` rather than
        // adding a new catalogue entry for this structural conflict error.
        MetadataError::SafetyOverridesDoubleDeclared => Some(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: err.to_string(),
            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
            code: Some(DiagnosticCode::YamlParseError),
            data: None,
        }),
        // Handled by dedicated arms in check_file_diagnostics (with precise span
        // anchoring and early returns):
        MetadataError::GeneratesUnknownValue { .. } => None,
        MetadataError::GeneratesMixedWithBareModel { .. } => None,
        // These variants only arise from validate_timeseries on the Ok(Single)
        // path — they are never returned by extract_file_metadata itself:
        MetadataError::TimeseriesRequiredForPartitionGrain => None,
        MetadataError::MalformedTimeseries { .. } => None,
        MetadataError::PlausibleContractOnSkeletonColumn { .. } => None,
        MetadataError::KeyedForbidsTimeseries => None,
        MetadataError::PartitionGrainRequiresRefreshIncremental => None,
        MetadataError::KeyedForbidsSafetyOverrides => None,
        MetadataError::MaterializedViewForbidsTimeseries => None,
        MetadataError::MaterializedViewForbidsPartitionGrain => None,
        MetadataError::MalformedFunctionalDependency { .. } => None,
        MetadataError::MalformedBoundedDomain { .. } => None,
        MetadataError::GrainRequiredForIncremental => None,
        MetadataError::GrainRequiresIncremental => None,
        MetadataError::GrainAssertionMismatch { .. } => None,
        // Never returned by extract_file_metadata/validate_timeseries — made
        // by `maintenance_plan_diagnostics` (needs the write-pattern
        // registry + backend capabilities) and folded into
        // `Maintenance*` diagnostics in `check_file_diagnostics` below,
        // exactly like `KeyedForbidsTimeseries` above.
        MetadataError::MaintenanceWritePatternUnavailable { .. } => None,
        MetadataError::MaintenanceWriteAddressingRefused { .. } => None,
        // Handled by a dedicated arm in check_file_diagnostics: `UnknownColumnTestKind`
        // is raised by the pure `validate_column_tests` on the `Ok(Single)` path;
        // `ColumnTestOnUnknownColumn` needs `typed_model_schema` (Salsa), which this
        // pure mapper does not have.
        MetadataError::UnknownColumnTestKind { .. } => None,
        MetadataError::ColumnTestOnUnknownColumn { .. } => None,
        // Raised by `extract_single_model`'s strict `contract:` pre-validation
        // (a pure format check, no Salsa data needed) — handled here like
        // `YamlParseError`, sharing its `ContractFrozenHorizonInvalid`
        // diagnostic code with the distinct grain-admissibility check made by
        // `smelt_logical::contract::frozen_horizon::validate_frozen_horizon`
        // (a dedicated arm further down in `check_file_diagnostics`, since
        // that check needs the parsed `ModelMetadata.grain`).
        MetadataError::ContractFrozenHorizonInvalid { .. } => Some(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: err.to_string(),
            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
            code: Some(DiagnosticCode::ContractFrozenHorizonInvalid),
            data: None,
        }),
        // Raised by `extract_single_model`'s strict `contract:` pre-validation,
        // the same site and pattern as `ContractFrozenHorizonInvalid` above —
        // disambiguated by `smelt_core::metadata`'s own field-level check
        // rather than by this mapper.
        MetadataError::ContractDeferralInvalid { .. } => Some(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: err.to_string(),
            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
            code: Some(DiagnosticCode::ContractDeferralInvalid),
            data: None,
        }),
        // Raised by `extract_single_model`'s strict `contract:` pre-validation,
        // the same site and pattern as `ContractFrozenHorizonInvalid`/
        // `ContractDeferralInvalid` above — disambiguated by
        // `smelt_core::metadata`'s own field-level check rather than by this
        // mapper.
        MetadataError::ContractRetainDepartedInvalid { .. } => Some(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: err.to_string(),
            range: rowan::TextRange::empty(rowan::TextSize::from(0)),
            code: Some(DiagnosticCode::ContractRetainDepartedInvalid),
            data: None,
        }),
    }
}
