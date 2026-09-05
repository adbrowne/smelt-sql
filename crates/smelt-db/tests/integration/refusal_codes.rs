//! `smelt_logical::maintenance::refusal_code` agreement gate (ruling R2,
//! `docs/outcomes/20260905-property-diff/phases/02-plan.md` task 3;
//! fix round 1, F1/F2).
//!
//! `DiagnosticCode` lives in `smelt-db`, above `smelt-logical`
//! (`CLAUDE.md` §"Layered single-ownership"), so a profile refusal
//! (`smelt_logical::analysis::profile::ProfileRefusal`) can only carry the
//! diagnostic code's *name* as `Option<&'static str>`, not the enum value.
//! That trades a compile-time guarantee (an unrecognised `DiagnosticCode`
//! variant doesn't compile) for a runtime one — this test buys the
//! guarantee back in both directions:
//! - every `Some` name `refusal_code` returns must (a) name a real
//!   `DiagnosticCode` variant and (b) equal the code
//!   `smelt_db::queries::maintenance::diagnostic_for_refusal` — smelt-db's
//!   own, single-owned refusal → diagnostic mapping, also what
//!   `check_file_diagnostics` (`crates/smelt-db/src/lib.rs`) calls — emits
//!   for the same refusal shape, read from that function directly rather
//!   than from a `DiagnosticCode` typed into this test;
//! - every `None` `refusal_code` returns corresponds to a `Refusal` variant
//!   that has no `MaintenanceRefusal` counterpart at all (filtered to `None`
//!   before construction, `crates/smelt-db/src/queries/maintenance.rs`) —
//!   i.e. `smelt-db` really does raise no diagnostic for it today.

use smelt_db::diagnostics_types::DiagnosticCode;
use smelt_db::queries::maintenance::{diagnostic_for_refusal, MaintenanceRefusal};
use smelt_logical::maintenance::{refusal_code, Refusal};

/// Every `Refusal` variant that has a real `MaintenanceRefusal` counterpart,
/// paired with the equivalent `MaintenanceRefusal` value — so this test can
/// drive smelt-db's own `diagnostic_for_refusal` and compare its verdict
/// against `refusal_code`'s, rather than restating an expected
/// `DiagnosticCode` by hand.
fn refusal_with_db_counterpart() -> Vec<(Refusal, MaintenanceRefusal)> {
    vec![
        (
            Refusal::SkeletonChanged {
                column: "c".to_string(),
            },
            MaintenanceRefusal::SkeletonChanged {
                column: "c".to_string(),
            },
        ),
        (
            Refusal::SkeletonClauseChanged {
                reason: "r".to_string(),
            },
            MaintenanceRefusal::SkeletonClauseChanged {
                reason: "r".to_string(),
            },
        ),
        (
            Refusal::PartitionColumnChanged {
                from: "a".to_string(),
                to: "b".to_string(),
            },
            MaintenanceRefusal::PartitionColumnChanged {
                from: "a".to_string(),
                to: "b".to_string(),
            },
        ),
        (
            Refusal::ScanUnbounded {
                source: "s".to_string(),
                why: "w".to_string(),
            },
            MaintenanceRefusal::ScanUnbounded {
                source: "s".to_string(),
                why: "w".to_string(),
            },
        ),
        (
            Refusal::NoAdmissibleTechnique {
                trigger: "t".to_string(),
                why: "w".to_string(),
            },
            MaintenanceRefusal::NoAdmissibleTechnique {
                trigger: "t".to_string(),
                why: "w".to_string(),
            },
        ),
        (
            Refusal::UnsupportedGrain {
                grain: "g".to_string(),
                tracking_plan: "p".to_string(),
            },
            MaintenanceRefusal::UnsupportedGrain {
                grain: "g".to_string(),
                tracking_plan: "p".to_string(),
            },
        ),
        (
            Refusal::LocalityNotEstablished {
                message: "m".to_string(),
            },
            MaintenanceRefusal::LocalityNotEstablished {
                message: "m".to_string(),
            },
        ),
        (
            Refusal::IdentityNotDerivable {
                message: "m".to_string(),
            },
            MaintenanceRefusal::IdentityNotDerivable {
                message: "m".to_string(),
            },
        ),
        (
            Refusal::DefinitionChangeNotBackfillable {
                columns: vec!["c".to_string()],
                why: "w".to_string(),
            },
            MaintenanceRefusal::DefinitionChangeNotBackfillable {
                columns: vec!["c".to_string()],
                why: "w".to_string(),
            },
        ),
        (
            Refusal::KeyedRetractableContribution {
                source: "s".to_string(),
                columns: vec!["c".to_string()],
                why: "w".to_string(),
            },
            MaintenanceRefusal::KeyedRetractableContribution {
                source: "s".to_string(),
                columns: vec!["c".to_string()],
                why: "w".to_string(),
            },
        ),
    ]
}

/// The three `Refusal` variants with no `MaintenanceRefusal` counterpart at
/// all — `smelt-db/src/queries/maintenance.rs` filters them to `None`
/// before a `MaintenanceRefusal` is ever constructed, so there is no
/// `diagnostic_for_refusal` call to make for them; `refusal_code` must
/// return `None` too.
fn refusals_with_no_db_counterpart() -> Vec<Refusal> {
    vec![
        Refusal::ReachNotDerivable {
            edge: "e".to_string(),
            why: "w".to_string(),
        },
        Refusal::RepairKeysNotDiscoverable {
            source: "s".to_string(),
            why: "w".to_string(),
        },
        Refusal::RepairSliceUnbounded {
            source: "s".to_string(),
            why: "w".to_string(),
        },
    ]
}

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
