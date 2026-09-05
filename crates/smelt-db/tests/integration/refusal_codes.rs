//! `smelt_logical::maintenance::refusal_code` agreement gate (ruling R2,
//! `docs/outcomes/20260905-property-diff/phases/02-plan.md` task 3).
//!
//! `DiagnosticCode` lives in `smelt-db`, above `smelt-logical`
//! (`CLAUDE.md` §"Layered single-ownership"), so a profile refusal
//! (`smelt_logical::analysis::profile::ProfileRefusal`) can only carry the
//! diagnostic code's *name* as `&'static str`, not the enum value. That
//! trades a compile-time guarantee (an unrecognised `DiagnosticCode` variant
//! doesn't compile) for a runtime one — this test buys the guarantee back:
//! every name `refusal_code` can return must (a) name a real `DiagnosticCode`
//! variant and (b), for every `Refusal` variant `smelt-db`'s own
//! `MaintenanceRefusal` → `DiagnosticCode` match
//! (`crates/smelt-db/src/lib.rs`) actually maps to a diagnostic today, equal
//! the code that match emits for the same refusal shape.

use smelt_db::diagnostics_types::DiagnosticCode;
use smelt_logical::maintenance::{refusal_code, Refusal};

/// (name, the `DiagnosticCode` variant it must name) — each entry is a
/// compile-time assertion that the name really is a `DiagnosticCode`
/// variant: if `refusal_code` ever returns a name not listed here, or a
/// listed name stops matching its variant's own `{:?}` rendering, the test
/// fails rather than silently accepting a made-up string.
fn assert_names_real_code(name: &str, expected_variant: DiagnosticCode) {
    assert_eq!(
        name,
        format!("{expected_variant:?}"),
        "refusal_code returned '{name}', which does not name the real \
         DiagnosticCode variant {expected_variant:?}"
    );
}

/// Every name `refusal_code` can return must parse to a real `DiagnosticCode`
/// variant. Exhaustive over every `Refusal` variant, mirroring
/// `smelt-logical`'s own `every_refusal_has_a_code` sample so a variant added
/// to one is caught missing from the other by ordinary code review, not
/// silently.
#[test]
fn refusal_code_names_are_real_variants() {
    let cases: Vec<(Refusal, DiagnosticCode)> = vec![
        (
            Refusal::SkeletonChanged {
                column: "c".to_string(),
            },
            DiagnosticCode::MaintenanceSkeletonChanged,
        ),
        (
            Refusal::SkeletonClauseChanged {
                reason: "r".to_string(),
            },
            DiagnosticCode::MaintenanceSkeletonChanged,
        ),
        (
            Refusal::PartitionColumnChanged {
                from: "a".to_string(),
                to: "b".to_string(),
            },
            DiagnosticCode::MaintenancePartitionColumnChanged,
        ),
        (
            Refusal::ScanUnbounded {
                source: "s".to_string(),
                why: "w".to_string(),
            },
            DiagnosticCode::MaintenanceScanUnbounded,
        ),
        (
            Refusal::NoAdmissibleTechnique {
                trigger: "t".to_string(),
                why: "w".to_string(),
            },
            DiagnosticCode::MaintenanceNoAdmissibleTechnique,
        ),
        (
            Refusal::UnsupportedGrain {
                grain: "g".to_string(),
                tracking_plan: "p".to_string(),
            },
            DiagnosticCode::MaintenanceUnsupportedGrain,
        ),
        (
            Refusal::LocalityNotEstablished {
                message: "m".to_string(),
            },
            DiagnosticCode::KeyedForbidsTimeseries,
        ),
        (
            Refusal::IdentityNotDerivable {
                message: "m".to_string(),
            },
            DiagnosticCode::GrainAssertionMismatch,
        ),
        (
            Refusal::DefinitionChangeNotBackfillable {
                columns: vec!["c".to_string()],
                why: "w".to_string(),
            },
            DiagnosticCode::MaintenanceColumnAddNotBackfillable,
        ),
        (
            Refusal::KeyedRetractableContribution {
                source: "s".to_string(),
                columns: vec!["c".to_string()],
                why: "w".to_string(),
            },
            DiagnosticCode::KeyedRetractableContribution,
        ),
        // The three refusals `smelt-db/src/queries/maintenance.rs` still
        // maps to `None` (no `DiagnosticCode` variant of their own yet).
        // `refusal_code` still must name a *real* code — today the closest
        // one covering the same "no technique admits this trigger" failure
        // — even though `smelt-db` does not itself emit that code for these
        // refusals (see `refusal_code`'s own doc comment).
        (
            Refusal::ReachNotDerivable {
                edge: "e".to_string(),
                why: "w".to_string(),
            },
            DiagnosticCode::MaintenanceNoAdmissibleTechnique,
        ),
        (
            Refusal::RepairKeysNotDiscoverable {
                source: "s".to_string(),
                why: "w".to_string(),
            },
            DiagnosticCode::MaintenanceNoAdmissibleTechnique,
        ),
        (
            Refusal::RepairSliceUnbounded {
                source: "s".to_string(),
                why: "w".to_string(),
            },
            DiagnosticCode::MaintenanceNoAdmissibleTechnique,
        ),
    ];

    for (refusal, expected) in &cases {
        assert_names_real_code(refusal_code(refusal), *expected);
    }
}
