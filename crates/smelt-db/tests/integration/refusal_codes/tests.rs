use smelt_db::diagnostics_types::DiagnosticCode;
use smelt_db::queries::maintenance::diagnostic_for_refusal;
use smelt_logical::maintenance::refusal_code;

use super::fixtures::{refusal_with_db_counterpart, refusals_with_no_db_counterpart};

/// Every `Some` name `refusal_code` returns must parse to a real
/// `DiagnosticCode` variant AND agree with `diagnostic_for_refusal` — the
/// mapping `check_file_diagnostics` actually uses — for the equivalent
/// `MaintenanceRefusal`. Exhaustive over every `Refusal` variant that has a
/// `MaintenanceRefusal` counterpart.
#[test]
fn refusal_code_names_are_real_variants_and_agree_with_smelt_db() {
    for (refusal, db_refusal) in refusal_with_db_counterpart() {
        let name = refusal_code(&refusal).unwrap_or_else(|| {
            panic!("refusal_code returned None for {refusal:?}, which has a real smelt-db mapping")
        });
        let (_, db_code, _) = diagnostic_for_refusal(&db_refusal)
            .unwrap_or_else(|| panic!("diagnostic_for_refusal returned None for {db_refusal:?}"));
        assert_eq!(
            name,
            format!("{db_code:?}"),
            "refusal_code returned '{name}' for {refusal:?}, but smelt-db's own \
             diagnostic_for_refusal emits {db_code:?} for the equivalent MaintenanceRefusal"
        );
        // Belt-and-braces: the name must also parse to a real DiagnosticCode
        // variant on its own terms (redundant with the equality above, but
        // keeps the intent legible if the two enums ever diverge in name
        // only).
        let all_variants = [
            DiagnosticCode::MaintenanceSkeletonChanged,
            DiagnosticCode::MaintenancePartitionColumnChanged,
            DiagnosticCode::MaintenanceScanUnbounded,
            DiagnosticCode::MaintenanceNoAdmissibleTechnique,
            DiagnosticCode::MaintenanceUnsupportedGrain,
            DiagnosticCode::KeyedForbidsTimeseries,
            DiagnosticCode::GrainAssertionMismatch,
            DiagnosticCode::MaintenanceColumnAddNotBackfillable,
            DiagnosticCode::KeyedRetractableContribution,
            DiagnosticCode::SuccessionWindowFunctionNotLead,
            DiagnosticCode::SuccessionPartitionKeyMismatch,
            DiagnosticCode::SuccessionOrderNotMonotoneClock,
            DiagnosticCode::SuccessionIdentityNotProjected,
            DiagnosticCode::SuccessionRowLocalColumnViolation,
            DiagnosticCode::SuccessionSingleSourceOnly,
            DiagnosticCode::SuccessionDrivingSourceNotAppendOnly,
            DiagnosticCode::SuccessionPreFilterNotRowLocal,
            DiagnosticCode::SuccessionDeleteFilterMisplaced,
            DiagnosticCode::SuccessionPatternUnrecognized,
        ];
        assert!(
            all_variants.iter().any(|v| format!("{v:?}") == name),
            "refusal_code returned '{name}', which does not name a real DiagnosticCode variant"
        );
    }
}

/// Every `None` `refusal_code` returns must correspond to a `Refusal`
/// variant smelt-db itself raises no diagnostic for today (no
/// `MaintenanceRefusal` counterpart exists to construct).
#[test]
fn refusal_code_none_agrees_with_smelt_db_none() {
    for refusal in refusals_with_no_db_counterpart() {
        assert_eq!(
            refusal_code(&refusal),
            None,
            "refusal_code returned Some(..) for {refusal:?}, but smelt-db raises no diagnostic \
             for this refusal shape (no MaintenanceRefusal counterpart) — update this test if \
             that mapping now exists"
        );
    }
}
