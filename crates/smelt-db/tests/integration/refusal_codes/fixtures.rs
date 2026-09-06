use smelt_db::queries::maintenance::MaintenanceRefusal;
use smelt_logical::maintenance::Refusal;

/// Every `Refusal` variant that has a real `MaintenanceRefusal` counterpart,
/// paired with the equivalent `MaintenanceRefusal` value — so this test can
/// drive smelt-db's own `diagnostic_for_refusal` and compare its verdict
/// against `refusal_code`'s, rather than restating an expected
/// `DiagnosticCode` by hand.
pub(super) fn refusal_with_db_counterpart() -> Vec<(Refusal, MaintenanceRefusal)> {
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
pub(super) fn refusals_with_no_db_counterpart() -> Vec<Refusal> {
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
        Refusal::SuccessionNotRecognized {
            reason: smelt_logical::analysis::succession::NotSuccessionReason::PatternUnrecognized(
                "r".to_string(),
            ),
        },
    ]
}
