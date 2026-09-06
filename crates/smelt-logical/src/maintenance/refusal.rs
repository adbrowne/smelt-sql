/// A fail-loud refusal: the trigger has no admissible technique, or admitting
/// one would be dishonest (`01-framework.md` §10; `06-proof-obligations.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A field was added or changed in a skeleton position — a grain
    /// change, not a column backfill (EX-39).
    SkeletonChanged { column: String },
    /// The model's skeleton *clause* itself changed against a prior
    /// deployed snapshot (a changed `GROUP BY`, a changed `FROM` target, a
    /// changed join shape) — a grain change proven by
    /// [`crate::backbuild::diff::definition_diff`]'s clause-level factoring
    /// rather than by a `Trigger::ColumnAdded` landing in a skeleton
    /// position. Maps onto the same `MaintenanceSkeletonChanged` diagnostic
    /// code as [`Refusal::SkeletonChanged`] — one code, two refusal shapes
    /// (`docs/specs/definition_deltas.md` §"Detection").
    SkeletonClauseChanged { reason: String },
    /// A `grain: partition` model's declared `timeseries.partition_column` —
    /// the address every partition-grain maintenance write targets — differs
    /// from the column recorded in the deployed-schema snapshot at last
    /// deploy. Unlike `SkeletonClauseChanged`, this is not a proof over SQL
    /// text: the address is a world-fact carried on
    /// [`super::derive::ModelInputs::old_partition_col`], compared
    /// case-insensitively against the output's own current
    /// `partition_col`. Maps onto `MaintenancePartitionColumnChanged`
    /// (`docs/specs/incremental_shapes.md` §"The partition grain"). Fails
    /// closed: `old_partition_col: None` (no snapshot, or one written before
    /// this field existed) derives no refusal.
    PartitionColumnChanged { from: String, to: String },
    /// The derived scan/footprint cannot be partition-bounded and the K8
    /// guardrail (`require: partition_local`, the ratified default) refuses
    /// rather than shipping a silent full-table operation.
    ScanUnbounded { source: String, why: String },
    /// No technique survives admission for this trigger — fail loud, never
    /// silently fall back (`06-proof-obligations.md` §1.1).
    NoAdmissibleTechnique { trigger: String, why: String },
    /// An upstream maintained-model edge (`incremental_models.md` §"Upstream
    /// model edges") whose event-time clock cannot be derived — the upstream
    /// declares no `timeseries:` and none is inferable — so its
    /// creation-trigger cell cannot be clamped. Recorded (never a silent
    /// drop), naming the edge (the `MaintenanceReachNotDerivable` refusal).
    ReachNotDerivable { edge: String, why: String },
    /// A declared `grain:` this phase of maintenance-plan derivation does not
    /// yet support (e.g. `key_per_partition`). Names the grain and the plan
    /// tracking the missing support, rather than silently deriving a plan for
    /// a grain shape that was never actually admitted (the
    /// `MaintenanceUnsupportedGrain` refusal).
    UnsupportedGrain {
        grain: String,
        tracking_plan: String,
    },
    /// A `grain: key` model declares a `timeseries:` block but key temporal
    /// locality could not be established — no route applies (§"Key
    /// temporal locality"). `message` is the rendered `KeyedForbidsTimeseries`
    /// diagnostic (`locality::LocalityRefusal::message`): it names all
    /// three routes and the nearest missing fact.
    LocalityNotEstablished { message: String },
    /// A `grain: key` model's route-3 statically-derived recurrence bound
    /// disagrees with a declared `key_recurrence` over the same key
    /// (key-grain rule 16, `docs/specs/incremental_shapes.md` §"Key
    /// temporal locality"). Maps onto the `KeyedRecurrenceDeclarationMismatch`
    /// diagnostic. `message` is the rendered
    /// `locality::LocalityRefusal::RecurrenceDeclarationMismatch` text —
    /// names both values and the driving source.
    KeyedRecurrenceDeclarationMismatch { message: String },
    /// A `grain: key` model declares no top-level `unique_key:` and its own
    /// outermost SELECT's `GROUP BY` derives no key either (empty or
    /// absent) — there is no identity to maintain against. Maps onto the
    /// `GrainAssertionMismatch` diagnostic code, naming the asserted grain
    /// and the empty derived key (`models.md` §"Constraint violations").
    IdentityNotDerivable { message: String },
    /// The repair family's affected-key obligation (P7,
    /// `model_properties.md` §"Affected-key discovery") could not resolve a
    /// finite key set for `source`'s delta — the
    /// `MaintenanceRepairKeysNotDiscoverable` diagnostic. `why` carries
    /// [`crate::analysis::affected_keys::AffectedKeys::NotDiscoverable`]'s
    /// own reason, verbatim.
    RepairKeysNotDiscoverable { source: String, why: String },
    /// The repair family's per-group read could not be bounded to a slice
    /// (no reach / key-temporal-locality route applies) — the
    /// `MaintenanceRepairSliceUnbounded` diagnostic.
    RepairSliceUnbounded { source: String, why: String },
    /// A non-skeleton `Trigger::ColumnAdded` column cannot be backfilled in
    /// place — an unbounded scan for the column-scoped merge, no admissible
    /// technique, an unresolvable expression, or added columns in one group
    /// disagreeing on their definition-change classification. Unlike every
    /// other `Refusal` variant, this one does NOT block the model's ongoing
    /// maintenance plan: a run proceeds, ALTERs the column in, and leaves
    /// historical rows `NULL` until `smelt migrate` backfills them — the
    /// posture `docs/specs/definition_deltas.md` §"Detection" states, and
    /// mirrors `smelt-runtime`'s own run gate, which already exempts a pure
    /// column addition outright. Reported as a Warning
    /// (`MaintenanceColumnAddNotBackfillable`), never an Error.
    DefinitionChangeNotBackfillable { columns: Vec<String>, why: String },
    /// A `grain: key` model folds a retractable enrichment-join contribution
    /// (`analysis::join_shape::join_contribution_monotone` refuses it — a
    /// fanned-out or otherwise non-monotone join feeding a combiner that
    /// cannot undo a retraction) into a source whose repair admission
    /// (`repair::admit_per_group_recompute`) also cannot cover that
    /// retraction with a per-group recompute — the
    /// `KeyedRetractableContribution` diagnostic
    /// (`incremental_shapes.md` §"Enrichment joins"). Names the source, the
    /// affected fold column(s), and why (the join-contribution reason plus
    /// the failing repair obligation, verbatim). Always pushed alongside the
    /// pre-existing `NoAdmissibleTechnique` + `RepairKeysNotDiscoverable`/
    /// `RepairSliceUnbounded` refusals for the same trigger — additive, not
    /// a replacement. Steers toward `refresh: materialized_view` or
    /// composing the enrichment as a separate model.
    KeyedRetractableContribution {
        source: String,
        columns: Vec<String>,
        why: String,
    },
    /// An undeclared-grain `refresh: incremental` model's SQL did not prove
    /// the keyed-succession shape
    /// (`analysis::succession::classify_keyed_succession` returned
    /// `NotSuccession`) — the `SuccessionPatternUnrecognized` diagnostic and
    /// the ten more specific `Succession*` codes
    /// (`docs/outcomes/20260906-scd2-keyed-succession/outcome.md` criterion
    /// 2) map `reason` onto their own code; this refusal shape itself
    /// carries the classifier's reason verbatim, never re-derived.
    SuccessionNotRecognized {
        reason: crate::analysis::succession::NotSuccessionReason,
    },
}

/// The diagnostic-code **name** a refusal of this shape raises through the
/// ordinary diagnostics pipeline (`smelt-db/src/lib.rs`'s `MaintenanceRefusal`
/// → `DiagnosticCode` match). `smelt_db::diagnostics_types::DiagnosticCode`
/// is unreachable from here — it lives in `smelt-db`, above `smelt-logical`
/// (layered single-ownership, `CLAUDE.md` §Architectural invariants) — so
/// `analysis::profile::ProfileRefusal` carries the code's name as
/// `Option<&'static str>` rather than the enum value itself. This match is
/// exhaustive with **no wildcard arm**: a new [`Refusal`] variant is a
/// compile error here until it is given a name (or explicitly given `None`),
/// which is what buys back the compile-time guarantee `DiagnosticCode` would
/// otherwise have given for free (ruling R2,
/// `docs/outcomes/20260905-property-diff/phases/02-plan.md`).
///
/// Every `Some` name returned here is asserted, by `smelt-db`'s
/// `refusal_code_names_are_real_variants` test, to parse to a real
/// `DiagnosticCode` variant and to equal the code
/// `smelt-db/src/queries/maintenance.rs`/`smelt-db/src/lib.rs` actually emit
/// for that refusal shape. Three variants (`ReachNotDerivable`,
/// `RepairKeysNotDiscoverable`, `RepairSliceUnbounded`) raise no diagnostic
/// through the ordinary pipeline today — `smelt-db/src/queries/maintenance.rs`
/// maps all three to `None` (no `DiagnosticCode` variant yet; see its own
/// doc comments) — so this returns `None` for them too, rather than naming a
/// code the pipeline can never actually produce. Whether these three deserve
/// their own `DiagnosticCode` entries is open (`docs/specs/property_diff.md`
/// §Known Divergences).
pub fn refusal_code(refusal: &Refusal) -> Option<&'static str> {
    match refusal {
        Refusal::SkeletonChanged { .. } => Some("MaintenanceSkeletonChanged"),
        Refusal::SkeletonClauseChanged { .. } => Some("MaintenanceSkeletonChanged"),
        Refusal::PartitionColumnChanged { .. } => Some("MaintenancePartitionColumnChanged"),
        Refusal::ScanUnbounded { .. } => Some("MaintenanceScanUnbounded"),
        Refusal::NoAdmissibleTechnique { .. } => Some("MaintenanceNoAdmissibleTechnique"),
        Refusal::ReachNotDerivable { .. } => None,
        Refusal::UnsupportedGrain { .. } => Some("MaintenanceUnsupportedGrain"),
        Refusal::LocalityNotEstablished { .. } => Some("KeyedForbidsTimeseries"),
        Refusal::IdentityNotDerivable { .. } => Some("GrainAssertionMismatch"),
        Refusal::KeyedRecurrenceDeclarationMismatch { .. } => {
            Some("KeyedRecurrenceDeclarationMismatch")
        }
        Refusal::RepairKeysNotDiscoverable { .. } => None,
        Refusal::RepairSliceUnbounded { .. } => None,
        Refusal::DefinitionChangeNotBackfillable { .. } => {
            Some("MaintenanceColumnAddNotBackfillable")
        }
        Refusal::KeyedRetractableContribution { .. } => Some("KeyedRetractableContribution"),
        // Ten of the eleven `Succession*` codes, 1:1 with `NotSuccessionReason`
        // (the eleventh, `SuccessionPreFilterNegatesFlag`, is a
        // `SuccessionAdvisory` carried on the `Recognized` verdict, never a
        // `Refusal`). Exhaustive with no wildcard arm — a new
        // `NotSuccessionReason` variant is a compile error here until named.
        Refusal::SuccessionNotRecognized { reason } => {
            use crate::analysis::succession::NotSuccessionReason::*;
            Some(match reason {
                WindowFunctionNotLead(_) => "SuccessionWindowFunctionNotLead",
                PartitionKeyMismatch(_) => "SuccessionPartitionKeyMismatch",
                OrderNotMonotoneClock(_) => "SuccessionOrderNotMonotoneClock",
                IdentityNotProjected(_) => "SuccessionIdentityNotProjected",
                RowLocalColumnViolation(_) => "SuccessionRowLocalColumnViolation",
                SingleSourceOnly(_) => "SuccessionSingleSourceOnly",
                DrivingSourceNotAppendOnly(_) => "SuccessionDrivingSourceNotAppendOnly",
                PreFilterNotRowLocal(_) => "SuccessionPreFilterNotRowLocal",
                DeleteFilterMisplaced(_) => "SuccessionDeleteFilterMisplaced",
                PatternUnrecognized(_) => "SuccessionPatternUnrecognized",
            })
        }
    }
}

#[cfg(test)]
mod refusal_code_tests {
    use super::*;

    /// Every [`Refusal`] variant is classified — either a non-empty `Some`
    /// code, or an explicit `None` for the three that raise no diagnostic
    /// today — a future variant added to the enum without a matching arm
    /// here is a compile error, not a silent gap (ruling R2).
    #[test]
    fn every_refusal_is_classified() {
        let none_variants = [
            "ReachNotDerivable",
            "RepairKeysNotDiscoverable",
            "RepairSliceUnbounded",
        ];
        let sample: Vec<Refusal> = vec![
            Refusal::SkeletonChanged {
                column: "c".to_string(),
            },
            Refusal::SkeletonClauseChanged {
                reason: "r".to_string(),
            },
            Refusal::PartitionColumnChanged {
                from: "a".to_string(),
                to: "b".to_string(),
            },
            Refusal::ScanUnbounded {
                source: "s".to_string(),
                why: "w".to_string(),
            },
            Refusal::NoAdmissibleTechnique {
                trigger: "t".to_string(),
                why: "w".to_string(),
            },
            Refusal::ReachNotDerivable {
                edge: "e".to_string(),
                why: "w".to_string(),
            },
            Refusal::UnsupportedGrain {
                grain: "g".to_string(),
                tracking_plan: "p".to_string(),
            },
            Refusal::LocalityNotEstablished {
                message: "m".to_string(),
            },
            Refusal::IdentityNotDerivable {
                message: "m".to_string(),
            },
            Refusal::RepairKeysNotDiscoverable {
                source: "s".to_string(),
                why: "w".to_string(),
            },
            Refusal::RepairSliceUnbounded {
                source: "s".to_string(),
                why: "w".to_string(),
            },
            Refusal::DefinitionChangeNotBackfillable {
                columns: vec!["c".to_string()],
                why: "w".to_string(),
            },
            Refusal::KeyedRetractableContribution {
                source: "s".to_string(),
                columns: vec!["c".to_string()],
                why: "w".to_string(),
            },
            Refusal::SuccessionNotRecognized {
                reason: crate::analysis::succession::NotSuccessionReason::PatternUnrecognized(
                    "r".to_string(),
                ),
            },
        ];
        for r in &sample {
            let variant_name = format!("{r:?}");
            let expects_none = none_variants.iter().any(|v| variant_name.starts_with(v));
            match refusal_code(r) {
                Some(code) => assert!(
                    !expects_none && !code.is_empty(),
                    "refusal_code returned Some(\"{code}\") for {r:?}, expected None"
                ),
                None => assert!(
                    expects_none,
                    "refusal_code returned None for {r:?}, expected Some"
                ),
            }
        }
    }

    /// The ten `NotSuccessionReason` variants each map to their own distinct
    /// `Succession*` name, and none is `None` — the exhaustive inner match's
    /// full coverage, driven data-first rather than sampling one reason.
    #[test]
    fn succession_reasons_each_name_their_own_code() {
        use crate::analysis::succession::NotSuccessionReason::*;
        let reasons = [
            WindowFunctionNotLead("r".to_string()),
            PartitionKeyMismatch("r".to_string()),
            OrderNotMonotoneClock("r".to_string()),
            IdentityNotProjected("r".to_string()),
            RowLocalColumnViolation("r".to_string()),
            SingleSourceOnly("r".to_string()),
            DrivingSourceNotAppendOnly("r".to_string()),
            PreFilterNotRowLocal("r".to_string()),
            DeleteFilterMisplaced("r".to_string()),
            PatternUnrecognized("r".to_string()),
        ];
        let mut names = std::collections::HashSet::new();
        for reason in reasons {
            let refusal = Refusal::SuccessionNotRecognized { reason };
            let name = refusal_code(&refusal).unwrap_or_else(|| {
                panic!("refusal_code returned None for {refusal:?}, expected Some")
            });
            assert!(
                names.insert(name),
                "duplicate code name '{name}' for {refusal:?} — every reason must name its own code"
            );
        }
        assert_eq!(names.len(), 10);
    }
}
