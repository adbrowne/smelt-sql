use super::*;

/// A Salsa-friendly (`PartialEq`) projection of a
/// [`smelt_logical::maintenance::Refusal`] — the two refusal kinds this
/// phase maps onto `Maintenance*` diagnostics. Mirrors the pure `Refusal`
/// enum's data exactly; it exists only so `MaintenancePlanDiagnostics` can
/// be a Salsa tracked-query return value without requiring `PartialEq` on
/// every type in `smelt-logical::maintenance` (out of this phase's allowed
/// files).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaintenanceRefusal {
    ScanUnbounded {
        source: String,
        why: String,
    },
    NoAdmissibleTechnique {
        trigger: String,
        why: String,
    },
    UnsupportedGrain {
        grain: String,
        tracking_plan: String,
    },
    LocalityNotEstablished {
        message: String,
    },
    /// `KeyedRecurrenceDeclarationMismatch` — a declared `key_recurrence`
    /// disagrees with route 3's statically-derived recurrence bound over
    /// the same key (key-grain rule 16).
    KeyedRecurrenceDeclarationMismatch {
        message: String,
    },
    /// `GrainAssertionMismatch` — a `grain: key` model with no declared
    /// top-level `unique_key:` and no GROUP-BY-derivable identity either.
    IdentityNotDerivable {
        message: String,
    },
    /// `MaintenanceSkeletonChanged` — an added or changed column occupies a
    /// row-membership/identity (skeleton) position, a grain change rather
    /// than a column backfill (EX-39, `definition_deltas.md` §"The verdict per column group").
    SkeletonChanged {
        column: String,
    },
    /// `MaintenanceSkeletonChanged` — the model's skeleton *clause* itself
    /// changed against a prior deployed snapshot (a changed `GROUP BY`, a
    /// changed `FROM` target, a changed join shape), proven by
    /// `smelt_logical::maintenance::derive::skeleton_clause_changed`'s
    /// clause-level factoring rather than by a `ColumnAdded` trigger
    /// landing in a skeleton position. Maps to the same
    /// `MaintenanceSkeletonChanged` diagnostic code as `SkeletonChanged`
    /// above — one code, two refusal shapes.
    SkeletonClauseChanged {
        reason: String,
    },
    /// `MaintenancePartitionColumnChanged` — the model's declared
    /// `timeseries.partition_column` differs from the address recorded in
    /// the deployed-schema snapshot at last deploy
    /// (`docs/specs/incremental_shapes.md` §"The partition grain").
    PartitionColumnChanged {
        from: String,
        to: String,
    },
    /// `MaintenanceColumnAddNotBackfillable` — a non-skeleton column
    /// addition that cannot be backfilled in place; the run proceeds with a
    /// Warning rather than refusing (`definition_deltas.md` §"Detection").
    DefinitionChangeNotBackfillable {
        columns: Vec<String>,
        why: String,
    },
    /// `KeyedRetractableContribution` — a retractable enrichment-join
    /// contribution the repair family cannot admit a per-group recompute
    /// for (`incremental_shapes.md` §"Enrichment joins").
    KeyedRetractableContribution {
        source: String,
        columns: Vec<String>,
        why: String,
    },
}

/// The `(severity, code, message)` a `MaintenanceRefusal` of this shape
/// raises through the ordinary diagnostics pipeline — the single owner of
/// that mapping. `crate::lib::check_file_diagnostics` (`smelt-db/src/lib.rs`)
/// is this function's production caller: it folds every `maintenance_plan`
/// refusal onto a diagnostic by calling this, never by re-matching
/// `MaintenanceRefusal` itself. `smelt-db`'s
/// `refusal_codes::refusal_code_names_are_real_variants` integration test
/// (`tests/integration/refusal_codes.rs`) is the other caller — driving the
/// agreement leg (ruling R2) from this function directly, rather than from a
/// `DiagnosticCode` typed into the test, so a change here cannot drift from
/// what the test asserts. `None` is not reachable today (`MaintenanceRefusal`
/// carries no variant this pipeline declines to diagnose — the three
/// `Refusal` variants with no `DiagnosticCode` of their own are filtered out
/// before construction, see this module's `Refusal` → `MaintenanceRefusal`
/// mapping); the `Option` return type future-proofs the signature against a
/// refusal shape that legitimately raises no diagnostic, matching
/// `smelt_logical::maintenance::refusal_code`'s own shape.
///
/// **Visibility deviation**: the phase-2 fix-round work order specified
/// `pub(crate)`, but `tests/integration/*.rs` compiles as a separate crate
/// (a Cargo integration-test binary) that cannot see `pub(crate)` items —
/// `pub(crate)` here would make the agreement test unable to call this
/// function at all, defeating F2's whole point. `pub` (not re-exported from
/// the crate root) is the minimal change that keeps the test able to read
/// the real mapping.
pub fn diagnostic_for_refusal(
    refusal: &MaintenanceRefusal,
) -> Option<(
    crate::diagnostics_types::DiagnosticSeverity,
    crate::diagnostics_types::DiagnosticCode,
    String,
)> {
    use crate::diagnostics_types::{DiagnosticCode, DiagnosticSeverity};
    Some(match refusal {
        MaintenanceRefusal::ScanUnbounded { source, why } => (
            DiagnosticSeverity::Error,
            DiagnosticCode::MaintenanceScanUnbounded,
            format!("maintenance scan over '{source}' cannot be partition-bounded: {why}"),
        ),
        MaintenanceRefusal::NoAdmissibleTechnique { trigger, why } => (
            DiagnosticSeverity::Error,
            DiagnosticCode::MaintenanceNoAdmissibleTechnique,
            format!("no maintenance technique admits trigger {trigger}: {why}"),
        ),
        MaintenanceRefusal::LocalityNotEstablished { message } => (
            DiagnosticSeverity::Error,
            DiagnosticCode::KeyedForbidsTimeseries,
            message.clone(),
        ),
        MaintenanceRefusal::KeyedRecurrenceDeclarationMismatch { message } => (
            DiagnosticSeverity::Error,
            DiagnosticCode::KeyedRecurrenceDeclarationMismatch,
            message.clone(),
        ),
        MaintenanceRefusal::IdentityNotDerivable { message } => (
            DiagnosticSeverity::Error,
            DiagnosticCode::GrainAssertionMismatch,
            message.clone(),
        ),
        MaintenanceRefusal::SkeletonChanged { column } => (
            DiagnosticSeverity::Error,
            DiagnosticCode::MaintenanceSkeletonChanged,
            format!(
                "column '{column}' occupies a row-membership/identity (skeleton) \
                 position — a grain change, never a column backfill (EX-39, \
                 docs/specs/incremental_models.md §\"The definition-change trigger\")",
            ),
        ),
        MaintenanceRefusal::SkeletonClauseChanged { reason } => (
            DiagnosticSeverity::Error,
            DiagnosticCode::MaintenanceSkeletonChanged,
            format!(
                "the model's skeleton clause changed against its deployed schema \
                 snapshot: {reason} — a grain change, never a column backfill (EX-39, \
                 docs/specs/incremental_models.md §\"The definition-change trigger\")",
            ),
        ),
        MaintenanceRefusal::PartitionColumnChanged { from, to } => (
            DiagnosticSeverity::Error,
            DiagnosticCode::MaintenancePartitionColumnChanged,
            format!(
                "declared timeseries.partition_column changed from '{from}' to '{to}' \
                 since this model was last deployed — the recorded address every \
                 partition-grain maintenance write targets no longer matches; this is a \
                 pre-execution refusal that no run flag bypasses (the analyzer gate \
                 blocks on any Error-severity diagnostic unconditionally), so delete the \
                 model's recorded snapshot (.smelt/targets/<target>/schemas/<model>.json) \
                 and re-run `smelt run` to re-address the table under the new column",
            ),
        ),
        MaintenanceRefusal::UnsupportedGrain {
            grain,
            tracking_plan,
        } => (
            DiagnosticSeverity::Error,
            DiagnosticCode::MaintenanceUnsupportedGrain,
            format!(
                "grain: {grain} is not yet supported by maintenance-plan derivation \
                 (tracked in {tracking_plan}); declare a supported grain \
                 (partition or key) or use refresh: full",
            ),
        ),
        MaintenanceRefusal::DefinitionChangeNotBackfillable { columns, why } => (
            DiagnosticSeverity::Warning,
            DiagnosticCode::MaintenanceColumnAddNotBackfillable,
            format!(
                "added column(s) {} cannot be backfilled in place: {why} — the run will \
                 ALTER them in and leave historical rows NULL until `smelt migrate` \
                 backfills them",
                columns.join(", "),
            ),
        ),
        MaintenanceRefusal::KeyedRetractableContribution {
            source,
            columns,
            why,
        } => (
            DiagnosticSeverity::Error,
            DiagnosticCode::KeyedRetractableContribution,
            format!(
                "enrichment join against '{source}' feeds a retractable contribution to \
                 column(s) {}: {why} — use `refresh: materialized_view`, or compose the \
                 enrichment as a separate model",
                columns.join(", "),
            ),
        ),
    })
}

/// A Salsa-friendly (`PartialEq`) projection of a
/// [`smelt_logical::maintenance::WritePinRefusal`] — mirrors the pure enum's
/// data exactly, for the same reason [`MaintenanceRefusal`] exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WritePinDiagnostic {
    /// `MaintenanceWritePatternUnavailable`.
    PatternUnavailable { pattern: String, backend: String },
    /// `MaintenanceWriteAddressingRefused`.
    AddressingRefused {
        cell: String,
        pattern: String,
        why: String,
    },
}

/// One cell's recorded availability downgrade
/// ([`smelt_logical::maintenance::availability::StateDowngrade`]), rendered
/// for `MaintenanceStateDowngraded` (`state.md` §Diagnostics). Salsa-safe
/// (`PartialEq`) projection — mirrors [`MaintenanceRefusal`]'s own reason
/// for existing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDowngradeDiagnostic {
    /// The cell's trigger, rendered the same way [`write_pin_diagnostics`]
    /// labels a cell (`format!("{:?}", trigger)`).
    pub cell: String,
    /// The technique ideal derivation chose, before the downgrade.
    pub original_technique: String,
    /// The state structure that was unavailable.
    pub missing_structure: String,
    /// The first declared backend the downgrade was observed against
    /// (`write_pin_diagnostics`'s own one-per-cell posture).
    pub backend: String,
    /// [`smelt_logical::maintenance::availability::StateDowngrade::reason`].
    pub reason: String,
}

/// A declared contract-lattice point whose semantics require a state
/// structure unavailable on a declared backend — `DeclaredContractRequiresState`
/// (`state.md` §Diagnostics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractStateRefusalDiagnostic {
    /// Names the declaration (`contract.deferral` or `contract.cells[].deferral`
    /// for the cell it addresses).
    pub declaration: String,
    /// The state structure the declaration's semantics require.
    pub missing_structure: String,
    /// The first declared backend the refusal was observed against.
    pub backend: String,
}

/// The result `maintenance_plan` (the Salsa query) returns: every admission
/// refusal from the derived plan, mapped to a Salsa-safe shape, plus the
/// `maintenance.cells[]` column-group-span violations. `file_diagnostics`
/// folds both into `Maintenance*` diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaintenancePlanDiagnostics {
    pub refusals: Vec<MaintenanceRefusal>,
    pub cell_column_group_violations: Vec<String>,
    /// The declared-`timeseries.granularity`-vs-derived-grouping check
    /// (`incremental_models.md` §Design "Grain is declared"), when the model
    /// declares a `timeseries:` block and a mismatch was positively
    /// derived. `None` when the model has no `timeseries:` block, the
    /// projection couldn't be located, or its shape didn't resolve to a
    /// known grid unit (undecidable, not a positive disproof) —
    /// [`smelt_logical::maintenance::granularity::check_declared_granularity`]'s
    /// own fail-open posture.
    pub granularity_mismatch: Option<GranularityMismatch>,
    /// Every `maintenance.cells[].write` pin that failed to resolve against
    /// the open write-pattern registry (`incremental_models.md` §"Per-cell
    /// write addressing" → "User pins") — computed by
    /// [`write_pin_diagnostics`].
    pub write_pin_refusals: Vec<WritePinDiagnostic>,
    /// Every source name whose `maintenance.scan_bounds.on_violation: warn`
    /// admitted the derived plan in place of a refusal
    /// (`incremental_models.md` §"Partition-local maintenance (the K8
    /// guardrail)") — `file_diagnostics` folds each into a
    /// `MaintenanceScanUnbounded` diagnostic at `Warning` severity rather
    /// than the `Error` a bare refusal maps to.
    pub scan_bounds_warnings: Vec<String>,
    /// Every plan cell whose ideal technique was downgraded by availability
    /// resolution (`smelt_logical::maintenance::availability::
    /// resolve_availability`) against at least one declared backend —
    /// folded into a `MaintenanceStateDowngraded` Warning diagnostic per
    /// cell (`state.md` §Diagnostics).
    pub state_downgrades: Vec<StateDowngradeDiagnostic>,
    /// Every declared contract-lattice point (model-level `contract.deferral`
    /// or a `contract.cells[].deferral` entry) whose required state
    /// structure is unavailable on at least one declared backend — folded
    /// into a `DeclaredContractRequiresState` Error diagnostic
    /// (`state.md` §Diagnostics).
    pub contract_state_refusals: Vec<ContractStateRefusalDiagnostic>,
}
