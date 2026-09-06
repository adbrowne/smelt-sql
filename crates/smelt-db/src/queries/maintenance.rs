//! Thin Salsa wrapper around `smelt_logical::maintenance::derive::derive_maintenance_plan`
//! (`incremental_models.md` §Surface "The plan (derived, reported)").
//!
//! Per the Salsa purity rule (`architecture.md` §"Salsa purity rule
//! (analysis)"), this module only *assembles inputs* — resolved source
//! facts, the declared output shape, the derived column groups/skeleton —
//! and calls the pure derivation in `smelt-logical`. It never re-implements
//! admission, locality, or ledger logic itself. The `#[salsa::tracked]`
//! query in `smelt-db/src/lib.rs` (`maintenance_plan`) is the only caller;
//! everything below is a plain function so it can be unit-tested without a
//! Salsa database.

use std::collections::HashMap;

use smelt_core::config::{
    Grain as ConfigGrain, Granularity, MaintenanceConfig, RefreshStrategy, ScanBoundsConfig,
    ScanBoundsRequire, ScanBoundsViolation,
};
use smelt_core::sources::{MutationProfile as SourceMutationKind, SourceInfo};
use smelt_core::ModelMetadata;
use smelt_logical::analysis::{select_stmt_items, SelectItemKind};
use smelt_logical::maintenance::derive::{
    derive_maintenance_plan_with_referential_integrity, FoldSpec, ModelInputs,
    SourceReferentialIntegrity,
};
use smelt_logical::maintenance::granularity::{check_declared_granularity, GranularityMismatch};
use smelt_logical::maintenance::grouping::{derive_column_groups, DegenerateColumn};
use smelt_logical::maintenance::locality::{
    establish_locality, partition_column_provably_not_null, single_clocked_granularity,
    LocalityInputs,
};
use smelt_logical::maintenance::skeleton::skeleton_columns;
use smelt_logical::maintenance::{
    identity_not_derivable_plan, locality_refused_plan, recurrence_mismatch_plan, ColumnGroup,
    Grain as PlanGrain, MaintenancePlan, MutationProfile as PlanMutationProfile, OutputSpec,
    SourceFacts, Trigger,
};
use smelt_logical::rules::cumulative::{
    declared_unique_key_matches, group_by_unique_key as derive_group_by_unique_key,
    OnceWriteAdmission,
};
use smelt_types::SqlFunction;

/// Everything `maintenance_plan` derives for one model: the raw plan (cells
/// and admission refusals) plus the column groups the `maintenance.cells[]`
/// frontmatter check reuses — a single derivation feeds both, per the
/// maintenance-plan-purity invariant ("derived once by pure functions;
/// consumers never re-derive it").
#[derive(Debug, Clone, Default)]
pub struct MaintenancePlanResult {
    pub plan: MaintenancePlan,
    pub column_groups: Vec<ColumnGroup>,
    /// Every column whose provenance couldn't be resolved and whose
    /// derivation fell back to the whole-model group
    /// (`grouping::derive_column_groups`'s `GroupingResult::degenerate`).
    /// Non-empty here is the only reliable signal of a genuine whole-model
    /// collapse — `column_groups.len() == 1` alone is neither necessary nor
    /// sufficient (a legitimately single-group model with 2+ mutable
    /// sources is not degenerate; a degenerate collapse against a
    /// single-source model still has `column_groups.len() == 1` with only
    /// one source in `mutation_sensitivity`).
    pub degenerate: Vec<DegenerateColumn>,
    /// This model's decomposed-state summary — one entry per presented
    /// column that folds through hidden state columns
    /// (`docs/outcomes/20260809-rung2-state-shapes` row 9), empty for a
    /// rung-1 model or one this function derives without classifying (every
    /// site here except `smelt_db::maintenance_plan_report`, which is the
    /// single caller that runs the keyed classifier and populates this
    /// field — `smelt-db/src/lib.rs`'s Salsa purity rule: this crate's own
    /// internal derivation never re-decides which columns are
    /// state-bearing).
    pub state_columns: Vec<smelt_logical::StateColumnSummary>,
    /// This model's three derived execution postures
    /// (`incremental_shapes.md` §"Derived execution postures",
    /// `docs/outcomes/20260815-keyed-grain-residue` phase 4) — `None` for a
    /// model that never classifies as `grain: key` (nothing to derive
    /// postures over), populated by the same `smelt-db/src/lib.rs` caller
    /// that fills `state_columns` from the same classification call.
    pub execution_postures: Option<smelt_logical::ExecutionPostures>,
    /// The run shape [`execution_postures`] qualifies — `Some(true)` for
    /// snapshot-reconcile (zero clocked driving sources), `Some(false)` for
    /// window-forward, `None` alongside `execution_postures: None`. A
    /// second field rather than folded into `ExecutionPostures` itself: the
    /// run shape depends on the classification's `driving_source`, not on
    /// `aggregator_columns` alone, so it can't be derived by
    /// `execution_postures`'s pure column-slice signature.
    pub is_snapshot_reconcile: Option<bool>,
    /// This model's per-column change-comparability (P3,
    /// `model_properties.md` §"Change comparability") — the SAME
    /// `analysis::walk::model_property_vector` call `derive_fold_spec` (or,
    /// for a `grain: partition` model with no fold spec, a dedicated call
    /// below) already makes, surfaced here so a `write:` pin's equivalence
    /// proof ([`smelt_logical::maintenance::cell_equivalence_proof`]) and
    /// `smelt explain` both read the one derivation rather than re-walking
    /// the model's SQL (`CLAUDE.md` §"Maintenance-plan purity"). Empty for
    /// every early-refusal path (`key_per_partition`, a declared/derived
    /// `unique_key` mismatch, a locality refusal) — those never reach the
    /// walk at all.
    pub comparability: Vec<smelt_logical::analysis::walk::ColumnComparability>,
}

/// Build one [`SourceFacts`] from a resolved source declaration (`None` when
/// the ref did not resolve to a known source) and the effective
/// `allow_full_scan` acceptance for it.
pub fn source_facts(name: &str, info: Option<&SourceInfo>, allow_full_scan: bool) -> SourceFacts {
    let partition_col = info
        .and_then(|s| s.timeseries.as_ref())
        .map(|t| t.partition_column.clone());
    let mutation = match info
        .and_then(|s| s.mutation_profile.as_ref())
        .map(|m| m.kind)
    {
        Some(SourceMutationKind::AppendOnly) => PlanMutationProfile::AppendOnly,
        Some(SourceMutationKind::ChangeFeed) => PlanMutationProfile::ChangeFeed,
        // Undeclared and `Mutable` both fail closed to the stricter
        // posture rather than assume append-only.
        _ => PlanMutationProfile::MutableSnapshot,
    };
    // The source's own declared `unique_key:` (`sources.md` §"Row
    // identity"). This is what `derive_column_groups`'
    // `top_level_join_context` (`smelt-logical/src/maintenance/grouping.rs`)
    // needs to prove a `JOIN ... ON`'s equality conjunct is 1:1 against this
    // source — without it every join against this source fails closed to
    // `Open` (never pruned), so an otherwise-closeable LEFT JOIN enrichment
    // stays membership-sensitive forever, regardless of what the source YAML
    // declares. Previously hardcoded to `vec![]` — this left
    // `Technique::ColumnScopedMerge` structurally unreachable from any real
    // source declaration (`docs/plans/20260809-sensitivity-precision.md`
    // Phase 5).
    let unique_key = info
        .and_then(|s| s.unique_key.as_ref())
        .cloned()
        .unwrap_or_default();
    SourceFacts {
        name: name.to_string(),
        mutation,
        partition_col,
        unique_key,
        allow_full_scan,
    }
}

/// Resolve the effective `maintenance.scan_bounds` for `source_address`:
/// the model's own block wins over the project baseline in `smelt.yml`
/// (`incremental_models.md` §Surface "Frontmatter": "A project-level default
/// in `smelt.yml` sets the baseline; per-model blocks refine it"). The
/// `on_violation` severity resolves with the SAME narrower-wins ladder as
/// `require` (default `Error`) — `incremental_models.md` §"Partition-local
/// maintenance (the K8 guardrail)": `warn` admits the derived plan and
/// reports the violation as a Warning; `error` (the default) refuses.
///
/// Returns `(allow_full_scan, require, on_violation)`.
pub fn effective_scan_bounds(
    source_address: &str,
    model: Option<&ScanBoundsConfig>,
    project: Option<&ScanBoundsConfig>,
) -> (bool, ScanBoundsRequire, ScanBoundsViolation) {
    let require = model
        .and_then(|s| s.require)
        .or_else(|| project.and_then(|s| s.require))
        .unwrap_or(ScanBoundsRequire::PartitionLocal);
    let on_violation = model
        .and_then(|s| s.on_violation)
        .or_else(|| project.and_then(|s| s.on_violation))
        .unwrap_or(ScanBoundsViolation::Error);
    // `require: none` disables the guardrail outright: every source reads as
    // though it were explicitly accepted, since the operator declared no
    // partition-locality expectation to check the derived plan against.
    if require == ScanBoundsRequire::None {
        return (true, require, on_violation);
    }
    let allow = model
        .map(|s| s.allow_full_scan(source_address))
        .filter(|b| *b)
        .or_else(|| project.map(|s| s.allow_full_scan(source_address)))
        .unwrap_or(false);
    (allow, require, on_violation)
}

/// Detect a fold candidate for a `grain: key` model's own outermost
/// `SELECT`: every non-key-by column that is a direct call to a recognised
/// aggregate function, each carrying **its own** combiner (a mixed fold —
/// `COUNT`→`SUM` alongside `MIN`→`MIN`/`MAX`→`MAX` over the same key — is
/// the common multi-column shape, not a single shared combiner).
///
/// This is *input assembly*, not admission: a wrong or missing guess here
/// only ever narrows to "no fold candidate" (the derivation then refuses
/// the `NewData` cell rather than fabricate one), never admits a fold the
/// derivation doesn't independently re-check via `combiner_discriminants`
/// and the source-posture obligation. Fail-closed: any non-key-by column
/// that is an aggregate but does not resolve to a recognised combiner
/// refuses the *whole* derivation (`None`), never a partial fold over just
/// the columns that did resolve.
///
/// An `ArgMax`/`ArgMin` (`MAX_BY`/`MIN_BY`) column decomposes to hidden
/// `(v, o)` state (`docs/outcomes/20260809-rung2-state-shapes` row 5) — no
/// companion projection is required. The runtime classifier
/// (`smelt_logical::rules::cumulative::classify_order_monotone_column`)
/// admits identically off the same arity check, so the two layers never
/// diverge; only the wrong-arity shape (not exactly 2 arguments) still
/// refuses the whole derivation (`None`), the same way an unrecognised
/// combiner does — this keeps `smelt explain`/LSP diagnostics from
/// reporting a `KeyedFold` admission the runtime then refuses with
/// `KeyedUnknownCombiner` (`CLAUDE.md` §"Fail-loud discipline"). The
/// decomposed-fold family (`AVG`/`STDDEV_*`/`VAR_*`,
/// `docs/outcomes/20260809-rung2-state-shapes` row 7) mirrors this exactly,
/// with an exact-1-argument/no-`DISTINCT` shape check instead.
///
/// `declared_functional_dependencies` is the model's own declared
/// `functional_dependencies:` block, threaded straight into
/// [`smelt_logical::rules::cumulative::classify_once_write`] — the SAME
/// shared helper the runtime classifier
/// (`smelt_logical::rules::cumulative::classify_cumulative`) uses for the
/// once-write (`COALESCE`) family, so this plan-layer derivation and the
/// runtime admission never diverge (`docs/plans/20260809-keyed-frontier.md`
/// Phase 4's "single shared helper" review lesson from Phases 1/3).
pub fn derive_fold_spec(
    sql: &str,
    declared_functional_dependencies: &[smelt_core::config::FunctionalDependency],
) -> Option<FoldSpec> {
    let parse = smelt_parser::parse(sql);
    let file = smelt_parser::File::cast(parse.syntax())?;
    let select = file.select_stmt()?;
    let items = select_stmt_items(&select)?;
    let unique_key = derive_group_by_unique_key(sql);
    // The model's own GROUP BY expressions — `classify_once_write` consults
    // them so a null-safe composite key (`COALESCE(x, 'n/a')` grouped by the
    // same expression) stays a KEY column rather than being claimed by the
    // once-write family.
    let group_by_exprs = smelt_logical::analyze_select(sql)
        .map(|a| a.group_by_exprs)
        .unwrap_or_default();
    let vector = smelt_logical::analysis::walk::model_property_vector(
        sql,
        &smelt_logical::analysis::join_shape::JoinContext::new(),
    );
    let mut add_columns: Vec<(String, SqlFunction)> = Vec::new();
    for item in &items {
        match item {
            SelectItemKind::OtherAggregate { alias, expr, .. } => {
                let func = expr.as_function_call()?;
                let name = func.name()?;
                let combiner = SqlFunction::from_name(&name.to_uppercase())?;
                // `ANY_VALUE` is the plain-overwrite family
                // (`docs/specs/incremental_shapes.md` §"The column-family
                // catalogue") — not a fold-family combiner at all (no
                // target/delta combine, incoming row always wins), so it never
                // enters a `FoldSpec`. A model whose non-key columns are ONLY
                // `ANY_VALUE` calls derives an empty `add_columns` here (→
                // `None` below), which `derive_new_data`'s `Grain::Key` arm
                // reads as "no fold-family column over this source" — the
                // snapshot-reconcile shape, not a refusal
                // (`docs/plans/20260809-keyed-frontier.md` Phase 3).
                if combiner == SqlFunction::AnyValue {
                    continue;
                }
                if matches!(combiner, SqlFunction::ArgMax | SqlFunction::ArgMin) {
                    // Exact-2-argument requirement, mirroring the runtime
                    // classifier's own arity check — the wrong-arity shape
                    // still refuses the whole derivation.
                    let args = func.arguments();
                    if args.len() != 2 {
                        return None;
                    }
                }
                // The decomposed-fold family (`AVG`/`STDDEV_*`/`VAR_*`,
                // `docs/outcomes/20260809-rung2-state-shapes` row 7) mirrors
                // the `ArgMax`/`ArgMin` precedent above: exact-1-argument, no
                // `DISTINCT` — the same shape
                // `rules::cumulative::classify_decomposed_fold_column` admits
                // via `decompose_to_state`. Either violation refuses the
                // whole derivation, the same way an unrecognised combiner
                // does, so `smelt explain`/LSP never reports a `KeyedFold`
                // admission the runtime classifier then refuses.
                if matches!(
                    combiner,
                    SqlFunction::Avg
                        | SqlFunction::Variance
                        | SqlFunction::Stddev
                        | SqlFunction::StddevPop
                        | SqlFunction::StddevSamp
                        | SqlFunction::VarPop
                        | SqlFunction::VarSamp
                ) {
                    let args = func.arguments();
                    if args.len() != 1 || smelt_logical::analysis::has_distinct_keyword(&func) {
                        return None;
                    }
                }
                add_columns.push((alias.clone(), combiner));
            }
            SelectItemKind::GroupByKey { text, alias, expr } => {
                // The once-write family (`COALESCE`) is a non-aggregate
                // scalar call, so it classifies as `GroupByKey`, not
                // `OtherAggregate` — mirrors `classify_cumulative`'s own
                // GroupByKey arm exactly.
                match smelt_logical::rules::cumulative::classify_once_write(
                    text,
                    expr,
                    &unique_key,
                    &group_by_exprs,
                    declared_functional_dependencies,
                    vector.as_ref(),
                    alias,
                ) {
                    OnceWriteAdmission::Admitted { .. } => {
                        add_columns.push((alias.clone(), SqlFunction::Coalesce));
                    }
                    // A once-write column whose provenance proof does not hold
                    // refuses the WHOLE derivation, exactly like an
                    // unrecognised combiner — dropping it would leave a
                    // partial `FoldSpec` over the model's other columns, so
                    // `smelt explain`/LSP would report a `KeyedFold`
                    // admission the runtime classifier refuses with
                    // `KeyedOnceWriteUnproven` (`CLAUDE.md` §"Fail-loud
                    // discipline").
                    OnceWriteAdmission::Unproven { .. } => return None,
                    // Not a once-write projection at all (a plain key column,
                    // a null-safe composite GROUP BY key): no fold column,
                    // no refusal.
                    OnceWriteAdmission::NotOnceWrite => {}
                }
            }
            SelectItemKind::CountDistinct { .. } => {}
        }
    }
    if add_columns.is_empty() {
        return None;
    }
    Some(FoldSpec { add_columns })
}

/// Assemble [`ModelInputs`] from already-resolved facts and derive the
/// plan. `sql` is the model body with frontmatter stripped. `sources` is
/// every source the model's `FROM`/`JOIN` clauses reference, already
/// resolved by the caller (mirrors `smelt-db::lib::ref_timeseries_config`'s
/// resolution, reused here for `mutation_profile` /
/// `allow_full_scan` instead of just `timeseries`).
///
/// Returns `None` when the model has no maintenance plan to derive: only
/// `refresh: incremental` models carry one (`incremental_models.md` §Surface
/// "The plan (derived, reported)": "Every non-`full` model has a
/// maintenance plan").
/// `driving_source_granularity` is the model's driving source's own declared
/// granularity, when the caller can determine it (used only by a `grain:
/// key` model that also declares its own `timeseries:` block, to check the
/// key-temporal-locality gate's granularity-equality structural
/// precondition — `incremental_shapes.md` §"Key temporal locality").
/// `None` fails that precondition closed (an unproven match is never
/// admitted); the runtime execution path (`smelt-runtime::cumulative`,
/// which has the driving source's `TimeseriesConfig` directly from the
/// classifier) is today's actual consumer of an admitted route.
/// `key_recurrences` is every referenced source's declared `key_recurrence`
/// bound (`sources.md` §"`mutation_profile` — the structured block"), keyed
/// by bare source name (the same convention `SourceFacts::name` and
/// `resolve_driving_source`'s resolved `driving.name` use) — consulted only
/// by key temporal locality's route 3 (recurrence-bounded) as the declared
/// fallback when no bound is statically derivable from the model's own SQL
/// (`docs/specs/incremental_shapes.md` §"Key temporal locality"). Build via
/// [`build_key_recurrences`], the sibling of [`build_source_facts`] over the
/// same `(ref_string, source_info)` pairs.
/// `deployed_column_names` is the model's previously-deployed output
/// column names (world-fact, read by the caller from the deployed-schema
/// snapshot the runtime's `schema_evolution` module already consults —
/// `smelt-db` itself does no I/O, per the Salsa-purity rule). An empty
/// slice means "no known deployed schema" and derives no `Trigger::
/// ColumnAdded` at all — the same fail-closed posture as before this
/// parameter existed (`docs/specs/definition_deltas.md` §"The verdict per column group"); every existing `smelt-db`-internal caller
/// (diagnostics, `smelt explain`) has no such snapshot to hand and passes
/// `&[]` unchanged. `smelt-runtime`'s maintenance driver is the one caller
/// with real I/O access to the deployed-schema store, and is the only one
/// that ever supplies a non-empty slice.
/// `source_referential_integrity` is every referenced source's declared
/// `referential_integrity` world-fact (`sources.md` §"Referential
/// integrity"), keyed by bare source name — threaded into every
/// `UpstreamMutation` cell's P1 skeleton-source-closure proof exactly like
/// [`derive_maintenance_plan_with_referential_integrity`] does. An empty map
/// (the caller's own default when it has not resolved the declaration)
/// behaves byte-identically to this function's behaviour before this
/// parameter existed — this only *adds* closure attempts for the sources
/// the caller names.
#[allow(clippy::too_many_arguments)]
pub fn derive_model_maintenance_plan(
    sql: &str,
    table: &str,
    metadata: &ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &std::collections::HashSet<String>,
    driving_source_granularity: Option<Granularity>,
    key_recurrences: &[(String, smelt_core::sources::KeyRecurrence)],
    deployed_column_names: &[String],
    source_referential_integrity: &SourceReferentialIntegrity,
    deployed_model_sql: Option<&str>,
    deployed_partition_column: Option<&str>,
    // The `(ref, SourceInfo)` pairs the keyed-succession classifier's
    // `SuccessionContext` is built from (`build_succession_context`) — a
    // side channel alongside `sources: &[SourceFacts]`, consulted only when
    // `metadata.resolved_grain()` is `None`. An empty slice degrades to the
    // classifier's own fail-closed refusal (`SingleSourceOnly`/
    // `DrivingSourceNotAppendOnly`), never a panic — callers with no source
    // declarations in scope (most `smelt-runtime` execution-path callers,
    // pre-succession) pass `&[]`.
    source_refs: &[(String, Option<SourceInfo>)],
) -> Option<MaintenancePlanResult> {
    if metadata.refresh != Some(RefreshStrategy::Incremental) {
        return None;
    }
    // The declared `grain:` check-only assertion when written (already
    // validated against the declared facts by
    // `smelt_core::metadata::validate_timeseries`), otherwise the label
    // derived from the two shape-defining facts (`timeseries:` /
    // `unique_key:`) — `docs/specs/models.md` §"Refresh axis". Reading the
    // resolved label here (rather than the raw `grain` field) is what admits
    // `refresh: incremental` on the facts alone, with no `grain:` written.
    //
    // `None` here (no declared/derivable `timeseries:`/`unique_key:`) is no
    // longer "not incremental" — it's the keyed-succession grain's own
    // undeclared-admission shape (`docs/specs/incremental_shapes.md`
    // §"Succession-grain admission (no declaration)"): the leaf classifier
    // decides admission on the model's own SQL, never a declared grain.
    let Some(grain) = metadata.resolved_grain() else {
        let ctx = build_succession_context(sql, source_refs);
        let verdict = match smelt_logical::analysis::walk::QueryTree::from_sql(sql) {
            Some(tree) => smelt_logical::analysis::walk::model_keyed_succession(&tree, &ctx),
            None => smelt_logical::analysis::succession::SuccessionVerdict::NotSuccession {
                reason:
                    smelt_logical::analysis::succession::NotSuccessionReason::PatternUnrecognized(
                        "SQL has no SELECT statement".to_string(),
                    ),
            },
        };
        let derivation =
            smelt_logical::maintenance::succession::derive_succession_plan(&verdict, table);
        return Some(MaintenancePlanResult {
            plan: derivation.plan,
            column_groups: Vec::new(),
            degenerate: Vec::new(),
            state_columns: Vec::new(),
            execution_postures: None,
            is_snapshot_reconcile: None,
            comparability: Vec::new(),
        });
    };
    if grain == ConfigGrain::KeyPerPartition {
        // Not yet supported: deriving a real plan for `key_per_partition`
        // needs trajectory/backfill machinery that doesn't exist yet
        // (`docs/plans/20260715-composed-axes-conditional-maintenance.md`
        // Phase A0). Refuse fail-loud instead of silently collapsing into a
        // keyed plan with an empty `unique_key` — there is nothing
        // meaningful to derive here, so this bypasses
        // `derive_maintenance_plan` entirely rather than feeding it inputs
        // built from a grain it was never taught to admit.
        return Some(MaintenancePlanResult {
            plan: smelt_logical::maintenance::unsupported_grain_plan("key_per_partition"),
            column_groups: Vec::new(),
            degenerate: Vec::new(),
            state_columns: Vec::new(),
            execution_postures: None,
            is_snapshot_reconcile: None,
            comparability: Vec::new(),
        });
    }
    let partition_col = metadata
        .timeseries
        .as_ref()
        .map(|t| t.partition_column.clone());
    // The admitted key-temporal-locality verdict for a `grain: key` model
    // that also declares a `timeseries:` block — captured here (the `Ok`
    // branch of `establish_locality`, below) and folded onto the derived
    // plan's `key_locality` after `derive_maintenance_plan` runs, so
    // `smelt-db`'s diagnostics and `smelt explain` can read the
    // already-admitted verdict instead of re-deriving it
    // (`docs/plans/20260715-composed-axes-conditional-maintenance.md`
    // Phase A5).
    let mut established_key_locality: Option<smelt_logical::maintenance::locality::LocalitySlice> =
        None;
    let plan_grain = match grain {
        ConfigGrain::Partition => PlanGrain::Partition {
            partition_col: partition_col.clone().unwrap_or_default(),
        },
        ConfigGrain::Key => {
            // The model's real derived `unique_key` — the GROUP BY columns
            // of its own outermost SELECT (the same derivation the keyed
            // classifier, `rules::cumulative::classify_cumulative`,
            // performs) — rather than a hardcoded empty vec. Threading it
            // here does not change which techniques any existing plan
            // admits: `derive_maintenance_plan`'s admission logic does not
            // yet branch on `Grain::Key`'s `unique_key` contents.
            let unique_key = derive_group_by_unique_key(sql);
            // A declared top-level `unique_key:` (`docs/specs/models.md`
            // §"Refresh axis") must agree with the GROUP-BY-derived key —
            // never a silent preference for either list
            // (`models.md` §"Constraint violations": "For aggregated key
            // bodies: `unique_key` ≠ the `GROUP BY` column set → hard error
            // (checked restatement)"). A model with no declared top-level
            // `unique_key:` (the pre-existing surface, relying on the
            // GROUP-BY derivation alone) has nothing to check against.
            if let Some(declared) = metadata.unique_key.as_deref() {
                if let Err((declared, derived)) = declared_unique_key_matches(declared, sql) {
                    return Some(MaintenancePlanResult {
                        plan: locality_refused_plan(format!(
                            "model '{table}' declares unique_key: {declared:?} but its \
                             outermost SELECT's GROUP BY derives {derived:?} — the declared \
                             identity must restate the GROUP BY column set exactly \
                             (docs/specs/models.md §\"Constraint violations\")"
                        )),
                        column_groups: Vec::new(),
                        degenerate: Vec::new(),
                        state_columns: Vec::new(),
                        execution_postures: None,
                        is_snapshot_reconcile: None,
                        comparability: Vec::new(),
                    });
                }
            } else if unique_key.is_empty() {
                // No declared top-level `unique_key:` and the model's own
                // GROUP BY derives no key either — there is no identity to
                // check anything against. Checked here (frontmatter-time,
                // reached by `file_diagnostics()` and `smelt explain`
                // without a run) rather than left to fail later, opaquely,
                // wherever a plan first consults `unique_key`
                // (`docs/specs/models.md` §"Constraint violations").
                return Some(MaintenancePlanResult {
                    plan: identity_not_derivable_plan(format!(
                        "model '{table}' asserts grain: key but declares no top-level \
                         unique_key: and its outermost SELECT's GROUP BY derives no key \
                         (empty) — a keyed model must have a derivable identity, either a \
                         declared unique_key: or a non-empty GROUP BY \
                         (docs/specs/models.md §\"Constraint violations\")"
                    )),
                    column_groups: Vec::new(),
                    degenerate: Vec::new(),
                    state_columns: Vec::new(),
                    execution_postures: None,
                    is_snapshot_reconcile: None,
                    comparability: Vec::new(),
                });
            }
            // A `grain: key` model that also declares a `timeseries:`
            // block must clear the key-temporal-locality gate before a
            // plan is derived at all — the single entry point deciding
            // keyed+timeseries admissibility
            // (`smelt_logical::maintenance::locality::establish_locality`,
            // `docs/specs/incremental_shapes.md` §"Key temporal locality").
            if let Some(own_ts) = metadata.timeseries.as_ref() {
                // The driving source is the single alias-scoped FROM/JOIN
                // input that both is a referenced source and declares its
                // own `timeseries:` clock — resolved by the shared
                // `locality::resolve_driving_source` helper, the same
                // anchor resolution `classify_cumulative` uses at runtime
                // (`smelt_logical::maintenance::locality::
                // resolve_driving_source`'s doc comment), so this static
                // plan-derivation call site and the runtime execution path
                // (`smelt-runtime::cumulative`) agree on which source drives
                // the model rather than each resolving it independently.
                // Neither "no clocked candidate" nor "ambiguous" (more than
                // one alias-scoped candidate) resolve a driving source here;
                // both fail the gate's structural preconditions closed.
                let (
                    driving_source_name,
                    driving_source_has_clock,
                    driving_source_partition_column,
                ) = match smelt_logical::maintenance::locality::resolve_driving_source(sql, sources)
                {
                    Ok(Some(driving)) => {
                        (driving.name.clone(), true, driving.partition_col.clone())
                    }
                    Ok(None) | Err(_) => (String::new(), false, None),
                };
                let partition_column_not_null = partition_column_provably_not_null(
                    sql,
                    &unique_key,
                    &own_ts.partition_column,
                    driving_source_partition_column.as_deref(),
                );
                let driving_source_key_recurrence = key_recurrences
                    .iter()
                    .find(|(name, _)| name == &driving_source_name)
                    .map(|(_, kr)| kr);
                let inputs = LocalityInputs {
                    model_name: table.to_string(),
                    unique_key: unique_key.clone(),
                    partition_column: own_ts.partition_column.clone(),
                    granularity: own_ts.granularity,
                    partition_column_not_null,
                    driving_source_name,
                    driving_source_has_clock,
                    driving_source_granularity,
                    driving_source_partition_column,
                    declared_functional_dependencies: &metadata.functional_dependencies,
                    driving_source_key_recurrence,
                    sql,
                };
                match establish_locality(&inputs) {
                    Err(
                        refusal @ smelt_logical::maintenance::locality::LocalityRefusal::RecurrenceDeclarationMismatch {
                            ..
                        },
                    ) => {
                        return Some(MaintenancePlanResult {
                            plan: recurrence_mismatch_plan(refusal.message(table)),
                            column_groups: Vec::new(),
                            degenerate: Vec::new(),
                            state_columns: Vec::new(),
                            execution_postures: None,
                            is_snapshot_reconcile: None,
                            comparability: Vec::new(),
                        });
                    }
                    Err(refusal) => {
                        return Some(MaintenancePlanResult {
                            plan: locality_refused_plan(refusal.message(table)),
                            column_groups: Vec::new(),
                            degenerate: Vec::new(),
                            state_columns: Vec::new(),
                            execution_postures: None,
                            is_snapshot_reconcile: None,
                            comparability: Vec::new(),
                        });
                    }
                    // Admitted: the derived `LocalitySlice` is folded onto
                    // the plan's `key_locality` below (after
                    // `derive_maintenance_plan` runs) rather than
                    // discarded — `smelt-db`'s diagnostics and `smelt
                    // explain` are consumers of the same admitted verdict
                    // the runtime execution path (`smelt-runtime::
                    // cumulative`) already slice-prunes with, not a second
                    // re-derivation of it.
                    Ok(slice) => established_key_locality = Some(slice),
                }
            }
            PlanGrain::Key { unique_key }
        }
        ConfigGrain::KeyPerPartition => unreachable!("handled above"),
    };
    let skeleton = skeleton_columns(sql, &[], partition_col.as_deref());
    let grouping = derive_column_groups(sql, sources, &skeleton);
    let fold = match grain {
        ConfigGrain::Key => derive_fold_spec(sql, &metadata.functional_dependencies),
        _ => None,
    };
    let output = OutputSpec {
        table: table.to_string(),
        grain: plan_grain,
        skeleton_columns: skeleton,
    };
    // The definition-change trigger's inputs: `None`/unclassifiable and
    // "no deployed snapshot supplied" both fall back to "no old columns, no
    // added columns" — fail-closed, never a guessed `ColumnAdded` trigger
    // (`definition_deltas.md` §"The verdict per column group").
    let (old_columns, added_columns) = if deployed_column_names.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        smelt_logical::maintenance::derive::diff_deployed_columns(sql, deployed_column_names)
            .unwrap_or_default()
    };
    // A keyed-grain output's declared `timeseries.partition_column` — the
    // axis the footprint question is posed against (`model_properties.md`
    // §"Footprint reflection / bounded write footprint"). `None` for a
    // `Grain::Partition` output (posed against its own partition axis
    // instead) or a keyed output with no declared `timeseries:` block.
    let keyed_time_axis = match &output.grain {
        PlanGrain::Key { .. } => metadata
            .timeseries
            .as_ref()
            .map(|t| t.partition_column.as_str()),
        PlanGrain::Partition { .. } => None,
        // Unreachable: this branch of `derive_model_maintenance_plan` only
        // ever builds a `PlanGrain::Partition`/`PlanGrain::Key` output — a
        // succession-grain output is derived by the separate
        // `maintenance::succession::derive_succession_plan` path, which
        // bypasses this code entirely (see the `resolved_grain()`-is-`None`
        // branch above).
        PlanGrain::Succession { .. } => unreachable!(
            "PlanGrain::Succession is derived by maintenance::succession::derive_succession_plan, \
             never by this branch of derive_model_maintenance_plan"
        ),
    };
    let inputs = ModelInputs {
        sql,
        output,
        sources: sources.to_vec(),
        column_groups: grouping.groups.clone(),
        fold,
        old_columns,
        old_sql: deployed_model_sql,
        keyed_time_axis,
        old_partition_col: deployed_partition_column,
    };

    // Trigger derivation itself is a pure `smelt-logical` function
    // (`derive::derive_triggers`, `incremental_models.md` §"Per-cell
    // admission" → "Which changed inputs get a mutation cell") — this
    // wrapper only assembles the facts (Salsa purity rule).
    let triggers = smelt_logical::maintenance::derive::derive_triggers(
        sources,
        &grouping.groups,
        explicitly_mutable,
        &added_columns,
    );

    let mut plan = derive_maintenance_plan_with_referential_integrity(
        &inputs,
        &triggers,
        source_referential_integrity,
    );
    plan.key_locality = established_key_locality.map(|slice| {
        let bound = smelt_logical::maintenance::locality::settle_bound(&slice);
        smelt_logical::maintenance::KeyLocality {
            slice,
            settle_bound: bound,
        }
    });
    // The single `model_property_vector` call this derivation surfaces to
    // callers (`MaintenancePlanResult::comparability`'s own doc comment) —
    // `derive_fold_spec` above already re-derives the same vector for a
    // `grain: key` model's fold-spec walk, so this call is only load-bearing
    // for a `grain: partition` model (no fold spec) or when the fold-spec
    // walk itself failed to parse; either way, consumers read this field,
    // never re-walk.
    let comparability = smelt_logical::analysis::walk::model_property_vector(
        sql,
        &smelt_logical::analysis::join_shape::JoinContext::new(),
    )
    .map(|v| v.comparability)
    .unwrap_or_default();
    Some(MaintenancePlanResult {
        plan,
        column_groups: grouping.groups,
        degenerate: grouping.degenerate,
        comparability,
        state_columns: Vec::new(),
        execution_postures: None,
        is_snapshot_reconcile: None,
    })
}

/// Like [`derive_model_maintenance_plan`], but additionally folds the
/// creation-trigger cells (and `MaintenanceReachNotDerivable` refusals) for
/// the model's **upstream maintained-model edges** into the plan
/// (`incremental_models.md` §"Upstream model edges").
///
/// `model_edges` is assembled by the caller from each upstream model's own
/// already-validated metadata (the leading `smelt.` stripped from the ref
/// name; `clock_col` from the upstream's `timeseries.partition_column`, or
/// `None` when it declares none). View/`full` upstreams deliver no
/// incremental delta and must not appear here — the caller excludes them, so
/// they contribute neither a creation cell nor a refusal.
///
/// Kept as a wrapper over [`derive_model_maintenance_plan`] so the many
/// source-only callers (`smelt-runtime`'s maintenance driver and propagation
/// walk) are unchanged; both entry points still call one pure derivation.
#[allow(clippy::too_many_arguments)]
pub fn derive_model_maintenance_plan_with_edges(
    sql: &str,
    table: &str,
    metadata: &ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &std::collections::HashSet<String>,
    model_edges: &[smelt_logical::maintenance::derive::ModelEdge],
    driving_source_granularity: Option<Granularity>,
    key_recurrences: &[(String, smelt_core::sources::KeyRecurrence)],
    deployed_column_names: &[String],
    source_referential_integrity: &SourceReferentialIntegrity,
    deployed_model_sql: Option<&str>,
    deployed_partition_column: Option<&str>,
    source_refs: &[(String, Option<SourceInfo>)],
) -> Option<MaintenancePlanResult> {
    let mut result = derive_model_maintenance_plan(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        driving_source_granularity,
        key_recurrences,
        deployed_column_names,
        source_referential_integrity,
        deployed_model_sql,
        deployed_partition_column,
        source_refs,
    )?;
    // Model edges only clamp against a partition-addressed output axis; a
    // key-addressed downstream contributes none (deferred). Reads the
    // resolved (declared-or-derived) grain, matching `derive_model_maintenance_plan`
    // above, so a facts-alone partition-grain model (no `grain:` written)
    // clamps the same way as one that writes `grain: partition` explicitly.
    let output_partition_col = match metadata.resolved_grain() {
        Some(ConfigGrain::Partition) => metadata
            .timeseries
            .as_ref()
            .map(|t| t.partition_column.as_str()),
        _ => None,
    };
    smelt_logical::maintenance::derive::append_model_edge_cells(
        &mut result.plan,
        sql,
        output_partition_col,
        model_edges,
        metadata.unique_key.as_deref().unwrap_or(&[]),
        sources,
        source_referential_integrity,
    );
    Some(result)
}

/// A `maintenance.cells[]` entry whose declared `columns` span more than one
/// derived column group — an error, since it would silently re-partition
/// the plan (`incremental_models.md` §Surface "Frontmatter"). Returns one
/// message per offending cell, naming the cell's `on:` trigger and the
/// distinct group names its columns land in.
pub fn cell_column_group_violations(
    maintenance: &MaintenanceConfig,
    groups: &[ColumnGroup],
) -> Vec<String> {
    let mut violations = Vec::new();
    for cell in &maintenance.cells {
        let mut hit_groups: Vec<String> = Vec::new();
        for col in &cell.columns {
            if let Some(group) = groups.iter().find(|g| g.columns.iter().any(|c| c == col)) {
                let name = group.name();
                if !hit_groups.contains(&name) {
                    hit_groups.push(name);
                }
            }
        }
        if hit_groups.len() > 1 {
            violations.push(format!(
                "maintenance.cells[on: {}].columns spans {} derived column groups ({}); \
                 a cell must address exactly one group — split it into one cell per group",
                cell.on,
                hit_groups.len(),
                hit_groups.join(", "),
            ));
        }
    }
    violations
}

/// The single clocked referenced source's own declared granularity — for
/// the key-temporal-locality gate's granularity-equality structural
/// precondition (`incremental_shapes.md` §"Key temporal locality"). `None`
/// when zero or more than one referenced source declares a `timeseries:`
/// block: an ambiguous or absent driving source fails that precondition
/// closed rather than guess.
///
/// Thin wrapper over the single shared "exactly one clocked candidate, else
/// undecided" rule
/// ([`smelt_logical::maintenance::locality::single_clocked_granularity`]) —
/// declared sources are this function's own candidate pool; a caller that
/// also folds in composed-upstream-model candidates (`maintenance_plan_diagnostics`,
/// `smelt-db/src/lib.rs::maintenance_plan_report`) calls the shared rule
/// directly over the concatenated pool instead of this function.
pub fn single_clocked_source_granularity(
    source_refs: &[(String, Option<SourceInfo>)],
) -> Option<Granularity> {
    single_clocked_granularity(
        source_refs
            .iter()
            .filter_map(|(_, info)| info.as_ref().and_then(|i| i.timeseries.as_ref()))
            .map(|t| t.granularity),
    )
}

/// Build [`SourceFacts`] for every `(ref_string, source_info)` pair a model
/// references, applying the `maintenance.scan_bounds` ladder's `require`
/// half. The second return value names every source whose guardrail
/// resolved to `on_violation: warn` and was not otherwise accepted
/// (`require: none` or a declared `allow_full_scan`) — a CANDIDATE the
/// caller may re-derive with `allow_full_scan` forced on once it knows
/// (from the plan this first pass derives) whether that source's scan is
/// actually unbounded; not every candidate here corresponds to a real
/// violation; only a source that surfaces a `Refusal::ScanUnbounded` in the
/// derived plan actually needs the second pass and the Warning diagnostic.
pub fn build_source_facts(
    refs: &[(String, Option<SourceInfo>)],
    model_scan_bounds: Option<&ScanBoundsConfig>,
    project_scan_bounds: Option<&ScanBoundsConfig>,
) -> (Vec<SourceFacts>, Vec<String>) {
    let mut out = Vec::new();
    let mut warn_candidates = Vec::new();
    let mut seen: HashMap<String, ()> = HashMap::new();
    for (name, info) in refs {
        if seen.contains_key(name) {
            continue;
        }
        seen.insert(name.clone(), ());
        let (allow_full_scan, _require, on_violation) =
            effective_scan_bounds(name, model_scan_bounds, project_scan_bounds);
        if !allow_full_scan && on_violation == ScanBoundsViolation::Warn {
            warn_candidates.push(name.clone());
        }
        out.push(source_facts(name, info.as_ref(), allow_full_scan));
    }
    (out, warn_candidates)
}

/// Build the `(bare source name, key_recurrence)` list for every referenced
/// source that declares one (`sources.md` §"`mutation_profile` — the
/// structured block"), over the same `(ref_string, source_info)` pairs
/// [`build_source_facts`] consumes. Consulted only by key temporal
/// locality's route 3 (recurrence-bounded) as the declared fallback
/// (`smelt_logical::maintenance::locality::LocalityInputs::
/// driving_source_key_recurrence`) — sourced independently of `SourceFacts`
/// (rather than adding a field there) so the many existing `SourceFacts`
/// literal-construction call sites across the workspace stay unaffected by
/// a route this phase alone introduces.
pub fn build_key_recurrences(
    refs: &[(String, Option<SourceInfo>)],
) -> Vec<(String, smelt_core::sources::KeyRecurrence)> {
    let mut out = Vec::new();
    let mut seen: HashMap<String, ()> = HashMap::new();
    for (name, info) in refs {
        if seen.contains_key(name) {
            continue;
        }
        seen.insert(name.clone(), ());
        if let Some(kr) = info
            .as_ref()
            .and_then(|s| s.mutation_profile.as_ref())
            .and_then(|m| m.key_recurrence.clone())
        {
            out.push((name.clone(), kr));
        }
    }
    out
}

/// Build the `SourceReferentialIntegrity` map (bare source name →
/// declared `referential_integrity` columns) [`derive_model_maintenance_
/// plan`]'s `source_referential_integrity` parameter needs, over the same
/// `(ref_string, source_info)` pairs [`build_source_facts`] consumes.
/// Sourced independently of `SourceFacts` (rather than adding a field
/// there) so the many existing `SourceFacts` literal-construction call
/// sites across the workspace stay unaffected by a route this phase alone
/// introduces — the same rationale [`build_key_recurrences`] documents for
/// its own sibling map.
pub fn build_source_referential_integrity(
    refs: &[(String, Option<SourceInfo>)],
) -> SourceReferentialIntegrity {
    let mut out = SourceReferentialIntegrity::new();
    for (name, info) in refs {
        if let Some(ri) = info.as_ref().and_then(|s| s.referential_integrity.clone()) {
            out.insert(name.clone(), ri);
        }
    }
    out
}

/// Build the [`SuccessionContext`] the keyed-succession leaf classifier
/// (`smelt_logical::analysis::walk::model_keyed_succession`) reads, over the
/// same `(ref_string ↔ bare source name, source_info)` pairs
/// [`build_key_recurrences`] walks — a side channel built fresh here rather
/// than two new `SourceFacts` fields, for the same rationale
/// [`build_key_recurrences`]'s own doc comment gives.
///
/// Resolves the model's driving source as the FROM clause's first
/// alias-scoped input (mirroring `smelt-db::file_check`'s own
/// `frozen_horizon` driving-source resolution). `SuccessionContext::
/// source_name` is set to the dot-joined path exactly as
/// `analysis::walk::InputItem::Table::name` spells it (e.g.
/// `"sources.customer_changes"`, never bare) — [`classify_keyed_succession`]
/// compares the two verbatim (rule 1), so stripping the `sources.` segment
/// here would make a real driving source unrecognisable. `refs` is keyed by
/// BARE source name instead (matching [`build_key_recurrences`]'s own
/// convention), so the lookup strips that segment separately. Fails closed
/// to an empty/`None`-carrying context — never a panic — when the SQL has no
/// FROM clause or the resolved source name has no declaration in `refs`:
/// [`smelt_logical::analysis::succession::classify_keyed_succession`]'s own
/// rule 1 (`FROM` is exactly one reference to the declared driving source)
/// then refuses on the merits, since an empty `source_name` can never match
/// a real `FROM` target.
pub fn build_succession_context(
    sql: &str,
    refs: &[(String, Option<SourceInfo>)],
) -> smelt_logical::analysis::succession::SuccessionContext {
    use smelt_logical::analysis::input_delta::MutationProfile as AnalysisMutationProfile;
    use smelt_logical::analysis::succession::SuccessionContext;

    let driving_source_name = smelt_parser::File::cast(smelt_parser::parse(sql).syntax())
        .and_then(|f| f.select_stmt())
        .and_then(|s| s.from_clause())
        .map(|fc| smelt_logical::analysis::source_bounds::from_clause_alias_sources(&fc))
        .and_then(|sources| sources.into_iter().next())
        .map(|(_, source_name)| source_name)
        .unwrap_or_default();
    let bare_name = driving_source_name
        .strip_prefix("sources.")
        .unwrap_or(&driving_source_name);

    let info = refs
        .iter()
        .find(|(name, _)| name == bare_name)
        .and_then(|(_, info)| info.as_ref());

    let mutation_profile = info
        .and_then(|s| s.mutation_profile.as_ref())
        .map(|m| match m.kind {
            SourceMutationKind::AppendOnly => AnalysisMutationProfile::AppendOnly,
            SourceMutationKind::Mutable => AnalysisMutationProfile::Mutable,
            SourceMutationKind::ChangeFeed => AnalysisMutationProfile::ChangeFeed,
        });
    let event_time_column = info
        .and_then(|s| s.timeseries.as_ref())
        .map(|t| t.event_time_column.clone());
    let not_null_columns = info
        .map(|s| {
            s.columns
                .iter()
                .filter(|c| !c.nullable)
                .map(|c| c.name.clone())
                .collect()
        })
        .unwrap_or_default();

    SuccessionContext {
        source_name: driving_source_name,
        mutation_profile,
        event_time_column,
        not_null_columns,
    }
}

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

/// The open write-pattern registry's [`smelt_logical::maintenance::
/// BackendWriteCapabilities`] for a declared backend name (`smelt.yml`
/// `targets.*.type`, lower-cased — the same vocabulary
/// `smelt_logical::lowering::backend_supports_struct_literal` and
/// `project_active_backends` already use). The single owner of the
/// name→struct mapping stays `smelt_dialect::BackendCapabilities`'s own
/// constructors — this only narrows the two booleans the write-pattern
/// registry needs (`CLAUDE.md` §"Layered single-ownership": `smelt-logical`
/// stays below `smelt-dialect`, so it cannot hold this mapping itself).
/// An unrecognised backend name conservatively reports no capability at
/// all — a `write:` pin naming a capability-gated pattern is refused rather
/// than silently assumed available.
pub fn backend_write_capabilities_for(
    backend_name: &str,
) -> smelt_logical::maintenance::BackendWriteCapabilities {
    let caps = match backend_name.to_ascii_lowercase().as_str() {
        "duckdb" => smelt_dialect::BackendCapabilities::duckdb(),
        "spark" | "databricks" => smelt_dialect::BackendCapabilities::spark(),
        _ => {
            return smelt_logical::maintenance::BackendWriteCapabilities::default();
        }
    };
    smelt_logical::maintenance::BackendWriteCapabilities {
        supports_merge: caps.supports_merge,
        supports_column_scoped_merge: caps.supports_column_scoped_merge,
    }
}

/// The [`smelt_dialect::SqlDialect`] a declared backend name (`smelt.yml`
/// `targets.*.type`, lower-cased) prints as — the availability-resolution
/// input `maintenance_plan_diagnostics` feeds
/// [`smelt_logical::maintenance::availability::realisable_state_structures`],
/// mirroring [`backend_write_capabilities_for`]'s own name vocabulary. An
/// unrecognised backend name resolves to `None`, which callers treat as no
/// state structure realisable at all — the same conservative-refusal
/// posture `backend_write_capabilities_for` takes for an unrecognised name,
/// never a silently-assumed dialect.
pub fn backend_dialect_for(backend_name: &str) -> Option<smelt_dialect::SqlDialect> {
    match backend_name.to_ascii_lowercase().as_str() {
        "duckdb" => Some(smelt_dialect::SqlDialect::DuckDB),
        "spark" | "databricks" => Some(smelt_dialect::SqlDialect::SparkSQL),
        "bigquery" => Some(smelt_dialect::SqlDialect::BigQuery),
        _ => None,
    }
}

/// The `on:` address a derived [`Trigger`] resolves to, for matching against
/// a `maintenance.cells[].on` frontmatter entry — mirrors the vocabulary
/// `cells[].on` already writes (`incremental_models.md` §Surface
/// "Frontmatter": "`on: <source-address> | backfill`"). `ColumnAdded` (the
/// definition-change trigger) has no `on:` address of its own — `write:`
/// pins do not address it in this phase.
fn trigger_on_address(trigger: &Trigger) -> Option<String> {
    match trigger {
        Trigger::NewData { source } | Trigger::UpstreamMutation { source } => Some(source.clone()),
        Trigger::Backfill => Some("backfill".to_string()),
        Trigger::ColumnAdded { .. } => None,
    }
}

/// The `maintenance.cells[].write` pin (if any) that addresses `plan_cell`,
/// per the same trigger/column-group matching
/// [`write_pin_diagnostics`] uses — read-only presentation lookup for
/// `smelt explain` (`smelt-cli/src/explain.rs`'s admissible-set + active-pin
/// rows). Never re-derives admission or the registry's admissible set
/// itself (`CLAUDE.md` §"Maintenance-plan purity") — just answers "does a
/// `cells[]` entry name this cell, and if so, what pin did it write".
fn write_pin_matching(
    on_address: &str,
    group: &str,
    column_groups: &[ColumnGroup],
    cells_cfg: &[smelt_core::config::MaintenanceCellConfig],
) -> Option<String> {
    cells_cfg.iter().find_map(|cell_cfg| {
        let pin = cell_cfg.write.as_deref()?;
        if cell_cfg.on != on_address {
            return None;
        }
        let matched_group_name = column_groups
            .iter()
            .find(|g| {
                g.columns
                    .iter()
                    .any(|c| cell_cfg.columns.iter().any(|cc| cc == c))
            })
            .map(|g| g.name());
        let group_matches = group == "{*}" || Some(group.to_string()) == matched_group_name;
        group_matches.then(|| pin.to_string())
    })
}

pub fn matching_write_pin(
    plan_cell: &smelt_logical::maintenance::PlanCell,
    column_groups: &[ColumnGroup],
    cells_cfg: &[smelt_core::config::MaintenanceCellConfig],
) -> Option<String> {
    let on_address = trigger_on_address(&plan_cell.trigger)?;
    write_pin_matching(&on_address, &plan_cell.group, column_groups, cells_cfg)
}

/// The `maintenance.cells[].write` pin (if any) addressing a `refresh: keyed`
/// model's window-forward keyed-fold write (`docs/outcomes/
/// 20260815-definition-delta-migrate/phases/27g-plan.md`). Unlike
/// [`matching_write_pin`], there is no derived [`smelt_logical::maintenance::
/// PlanCell`] to read here — `keyed`'s classifier (`smelt-planner`) runs
/// outside the `MaintenancePlan`/`derive_model_maintenance_plan` machinery
/// entirely — but the keyed fold's cell is always whole-row (`group: "{*}"`),
/// so it matches a `cells[]` entry by its `on:` address alone, using the
/// exact same predicate [`matching_write_pin`] uses (`write_pin_matching`
/// above, with `group` fixed to `"{*}"` and no column groups to consult).
/// Never re-derives admission — a read-only lookup for the runtime write
/// path to resolve the mechanism through
/// [`smelt_logical::maintenance::choice::resolve_keyed_write_mechanism`].
pub fn keyed_fold_write_pin(metadata: &ModelMetadata, driving_source: &str) -> Option<String> {
    let cells_cfg: &[smelt_core::config::MaintenanceCellConfig] = metadata
        .maintenance
        .as_ref()
        .map(|m| m.cells.as_slice())
        .unwrap_or(&[]);
    write_pin_matching(driving_source, "{*}", &[], cells_cfg)
}

/// The `maintenance.defaults`/`maintenance.cells[].prefer`/`cells[].technique`
/// override ladder's effective value for a `refresh: keyed` model's
/// whole-row keyed-fold write-suppression dimension (`docs/outcomes/
/// 20260815-definition-delta-migrate/phases/33-plan.md`). Mirrors
/// [`keyed_fold_write_pin`]'s own reasoning: there is no derived `PlanCell`
/// to consult here, and the keyed fold's cell is always whole-row
/// (`group: "{*}"`), so a `cells[]` entry matches by its `on:` address
/// alone — the same address-only rule [`write_pin_matching`]'s `group ==
/// "{*}"` arm already applies, not [`smelt_logical::maintenance::choice::
/// effective_override`]'s per-column-group `matching_cell`, which would
/// never match a whole-row cell's (typically empty) `columns`.
/// Never re-derives admission — a read-only lookup the runtime write path
/// folds into [`smelt_logical::maintenance::choice::resolve_write_variant`].
pub fn keyed_fold_effective_override(
    metadata: &ModelMetadata,
    driving_source: &str,
) -> smelt_logical::maintenance::choice::EffectiveOverride {
    let maintenance = metadata.maintenance.as_ref();
    let cells_cfg: &[smelt_core::config::MaintenanceCellConfig] =
        maintenance.map(|m| m.cells.as_slice()).unwrap_or(&[]);
    let broad_prefer = maintenance
        .and_then(|m| m.defaults.as_ref())
        .and_then(|d| d.prefer);
    let narrow = cells_cfg.iter().find(|c| c.on == driving_source);
    smelt_logical::maintenance::choice::EffectiveOverride {
        prefer: narrow.and_then(|c| c.prefer).or(broad_prefer),
        technique: narrow.and_then(|c| c.technique),
    }
}

/// Validate every `maintenance.cells[].write` pin against the open
/// write-pattern registry (`incremental_models.md` §"Per-cell write
/// addressing" → "User pins"): an unrecognised name, or one the target
/// backend(s) cannot execute, is `MaintenanceWritePatternUnavailable`; a
/// name the registry and backend admit but whose cell declares none of the
/// pattern's required contract facts (e.g. `write: keyed` on an
/// identity-free cell) is `MaintenanceWriteAddressingRefused`. Checked
/// against every one of the project's `active_backends` — a pin unavailable
/// on any declared target backend refuses, naming that backend, rather than
/// silently passing because a *different* target happens to support it.
///
/// A compare-based pin (`diff_patch`/`keyed_conditional`/`staged_candidate`)
/// is additionally checked against `comparability` — the model's derived
/// P3 column-comparability (`MaintenancePlanResult::comparability`) — via
/// [`smelt_logical::maintenance::cell_equivalence_proof`], so an
/// incomparable compared column or a `WholeRow` cell refuses
/// `MaintenanceWriteAddressingRefused` here too, not just the structural
/// contract-fact check.
///
/// Pure function — the caller ([`maintenance_plan_diagnostics`]) gathers
/// `metadata`/`plan`/`column_groups`/`active_backends`/`comparability` and
/// calls this; it never re-derives the plan itself (Salsa purity rule).
pub fn write_pin_diagnostics(
    metadata: &ModelMetadata,
    plan: &MaintenancePlan,
    column_groups: &[ColumnGroup],
    active_backends: &[String],
    comparability: &[smelt_logical::analysis::walk::ColumnComparability],
) -> Vec<WritePinDiagnostic> {
    use smelt_logical::maintenance::{
        cell_equivalence_proof, resolve_write_pin, OutputContractFacts, RowIdentity,
        WritePinRefusal,
    };

    let Some(maintenance) = metadata.maintenance.as_ref() else {
        return Vec::new();
    };
    let has_partition_axis = metadata.timeseries.is_some();
    let backends: Vec<String> = if active_backends.is_empty() {
        vec!["duckdb".to_string()]
    } else {
        active_backends.to_vec()
    };

    let mut out = Vec::new();
    for cell_cfg in &maintenance.cells {
        let Some(pin) = cell_cfg.write.as_deref() else {
            continue;
        };
        // A whole-row trigger's cell (`NewData`/`Backfill`) carries the
        // `{*}` wildcard group name (`PlanCell::group`'s own doc comment),
        // not a derived `ColumnGroup::name()` — it matches any `cells[]`
        // entry on the same `on:` trigger regardless of `columns`. A
        // per-column-group trigger (`UpstreamMutation`/`ColumnAdded`) only
        // matches a `cells[]` entry whose `columns` land in that same
        // derived group.
        let matched_group_name = column_groups
            .iter()
            .find(|g| {
                g.columns
                    .iter()
                    .any(|c| cell_cfg.columns.iter().any(|cc| cc == c))
            })
            .map(|g| g.name());
        let Some(plan_cell) = plan.cells.iter().find(|c| {
            trigger_on_address(&c.trigger).as_deref() == Some(cell_cfg.on.as_str())
                && (c.group == "{*}" || Some(c.group.clone()) == matched_group_name)
        }) else {
            continue;
        };
        let has_identity = matches!(plan_cell.row_identity.identity, RowIdentity::Key(_));
        let facts = OutputContractFacts {
            has_identity,
            has_partition_axis,
        };
        let cell_label = format!("{:?}", plan_cell.trigger);
        let group_columns: Vec<String> = column_groups
            .iter()
            .find(|g| g.name() == plan_cell.group)
            .map(|g| g.columns.clone())
            .unwrap_or_default();

        for backend_name in &backends {
            let backend_caps = backend_write_capabilities_for(backend_name);
            if let Err(refusal) = resolve_write_pin(
                &cell_label,
                pin,
                backend_name,
                facts,
                backend_caps,
                |pattern| {
                    cell_equivalence_proof(
                        pattern,
                        &group_columns,
                        comparability,
                        &plan_cell.row_identity,
                    )
                },
            ) {
                out.push(match refusal {
                    WritePinRefusal::PatternUnavailable { pattern, backend } => {
                        WritePinDiagnostic::PatternUnavailable { pattern, backend }
                    }
                    WritePinRefusal::AddressingRefused { cell, pattern, why } => {
                        WritePinDiagnostic::AddressingRefused { cell, pattern, why }
                    }
                });
                // One diagnostic per cell is enough — the pin either
                // resolves against every declared backend or it doesn't;
                // reporting per-backend duplicates would just be noise.
                break;
            }
        }
    }
    out
}

/// Assemble inputs (resolved source facts, declared output shape,
/// `maintenance.cells[]`) and derive the plan, mapping its refusals into
/// [`MaintenancePlanDiagnostics`]. `source_refs` is every `smelt.<path>`
/// this model's SQL references that resolves to a source declaration
/// (already resolved by the caller — mirrors
/// `smelt-db::lib::ref_timeseries_config`'s resolution seam).
/// `extra_model_sources` is every referenced upstream model that is itself
/// a locality-admitted composed output (`grain: key` + `timeseries:`),
/// already resolved by the caller (`smelt-db::lib::ref_model_source_facts`)
/// — appended to the declared-source candidate pool `resolve_driving_source`
/// consults, paired with its own granularity folded into
/// `driving_source_granularity`'s "exactly one clocked candidate" rule, so
/// a `grain: key` model may take a composed upstream model's own output as
/// its driving source exactly as it would a declared source
/// (`incremental_shapes.md` §"Key temporal locality (the time-partitioned
/// output)" — "The output as a clocked source").
///
/// Pure function — the `#[salsa::tracked]` wrapper in `smelt-db/src/lib.rs`
/// only gathers `source_refs`/`metadata`/`sql` and calls this.
#[allow(clippy::too_many_arguments)]
pub fn maintenance_plan_diagnostics(
    sql: &str,
    table: &str,
    metadata: &ModelMetadata,
    source_refs: &[(String, Option<SourceInfo>)],
    project_scan_bounds: Option<&ScanBoundsConfig>,
    extra_model_sources: &[(SourceFacts, Granularity)],
    active_backends: &[String],
    warehouse_tables: smelt_core::config::WarehouseTables,
    deployed_column_names: &[String],
    deployed_model_sql: Option<&str>,
    deployed_partition_column: Option<&str>,
) -> MaintenancePlanDiagnostics {
    let model_scan_bounds = metadata
        .maintenance
        .as_ref()
        .and_then(|m| m.scan_bounds.as_ref());
    let (mut sources, scan_bounds_warn_candidates) =
        build_source_facts(source_refs, model_scan_bounds, project_scan_bounds);
    for (facts, _) in extra_model_sources {
        if !sources.iter().any(|s| s.name == facts.name) {
            sources.push(facts.clone());
        }
    }
    let explicitly_mutable: std::collections::HashSet<String> = source_refs
        .iter()
        .filter(|(_, info)| {
            info.as_ref().is_some_and(|i| {
                i.mutation_profile
                    .as_ref()
                    .is_some_and(|m| m.kind == SourceMutationKind::Mutable)
            })
        })
        .map(|(name, _)| name.clone())
        .collect();
    let granularity_mismatch = metadata
        .timeseries
        .as_ref()
        .and_then(|ts| check_declared_granularity(sql, &ts.partition_column, ts.granularity));
    let mut clocked_granularities: Vec<Granularity> = source_refs
        .iter()
        .filter_map(|(_, info)| info.as_ref().and_then(|i| i.timeseries.as_ref()))
        .map(|t| t.granularity)
        .collect();
    clocked_granularities.extend(extra_model_sources.iter().map(|(_, g)| *g));
    let driving_source_granularity = single_clocked_granularity(clocked_granularities);
    let key_recurrences = build_key_recurrences(source_refs);
    let source_referential_integrity = build_source_referential_integrity(source_refs);
    let Some(mut result) = derive_model_maintenance_plan(
        sql,
        table,
        metadata,
        &sources,
        &explicitly_mutable,
        driving_source_granularity,
        &key_recurrences,
        // The deployed-schema snapshot is now a Salsa world-fact input
        // (`workspace_ingest::register_deployed_schemas_from_disk`) the
        // `#[salsa::tracked]` wrapper in `smelt-db/src/lib.rs` resolves and
        // passes down here — `smelt-db` itself still does no I/O, per the
        // Salsa-purity rule; it only forwards what the caller resolved.
        deployed_column_names,
        &source_referential_integrity,
        deployed_model_sql,
        deployed_partition_column,
        source_refs,
    ) else {
        return MaintenancePlanDiagnostics {
            granularity_mismatch,
            ..Default::default()
        };
    };
    // `on_violation: warn` (`incremental_models.md` §"Partition-local
    // maintenance (the K8 guardrail)"): a source in `scan_bounds_warn_
    // candidates` is only a REAL violation when the first pass actually
    // refused it with `ScanUnbounded` — a candidate whose scan turned out
    // to be bounded anyway (e.g. the driving, already-clocked source) must
    // not be reported. Only for the sources that genuinely refused, re-
    // derive once more with `allow_full_scan` forced on for exactly those
    // sources, admitting the plan and surfacing each as a Warning instead
    // of a refusal.
    let scan_bounds_warnings: Vec<String> = result
        .plan
        .refusals
        .iter()
        .filter_map(|r| match r {
            smelt_logical::maintenance::Refusal::ScanUnbounded { source, .. }
                if scan_bounds_warn_candidates.contains(source) =>
            {
                Some(source.clone())
            }
            _ => None,
        })
        .collect();
    if !scan_bounds_warnings.is_empty() {
        for facts in sources.iter_mut() {
            if scan_bounds_warnings.contains(&facts.name) {
                facts.allow_full_scan = true;
            }
        }
        if let Some(admitted) = derive_model_maintenance_plan(
            sql,
            table,
            metadata,
            &sources,
            &explicitly_mutable,
            driving_source_granularity,
            &key_recurrences,
            deployed_column_names,
            &source_referential_integrity,
            deployed_model_sql,
            deployed_partition_column,
            source_refs,
        ) {
            result = admitted;
        }
    }
    let refusals = result
        .plan
        .refusals
        .iter()
        .filter_map(|r| match r {
            smelt_logical::maintenance::Refusal::ScanUnbounded { source, why } => {
                Some(MaintenanceRefusal::ScanUnbounded {
                    source: source.clone(),
                    why: why.clone(),
                })
            }
            smelt_logical::maintenance::Refusal::NoAdmissibleTechnique { trigger, why } => {
                Some(MaintenanceRefusal::NoAdmissibleTechnique {
                    trigger: trigger.clone(),
                    why: why.clone(),
                })
            }
            smelt_logical::maintenance::Refusal::SkeletonChanged { column } => {
                Some(MaintenanceRefusal::SkeletonChanged {
                    column: column.clone(),
                })
            }
            smelt_logical::maintenance::Refusal::SkeletonClauseChanged { reason } => {
                Some(MaintenanceRefusal::SkeletonClauseChanged {
                    reason: reason.clone(),
                })
            }
            smelt_logical::maintenance::Refusal::PartitionColumnChanged { from, to } => {
                Some(MaintenanceRefusal::PartitionColumnChanged {
                    from: from.clone(),
                    to: to.clone(),
                })
            }
            // An underivable upstream-model clock. Recorded in the plan (and
            // surfaced by `smelt explain`'s Refusals section), but not yet
            // folded into `file_diagnostics()` — `MaintenanceReachNotDerivable`
            // has no `DiagnosticCode` variant yet (`diagnostics.md` §Known
            // divergences). Leave unmapped so a future phase's own diagnostic
            // lands it, exactly as `SkeletonChanged` above.
            smelt_logical::maintenance::Refusal::ReachNotDerivable { .. } => None,
            smelt_logical::maintenance::Refusal::UnsupportedGrain {
                grain,
                tracking_plan,
            } => Some(MaintenanceRefusal::UnsupportedGrain {
                grain: grain.clone(),
                tracking_plan: tracking_plan.clone(),
            }),
            smelt_logical::maintenance::Refusal::LocalityNotEstablished { message } => {
                Some(MaintenanceRefusal::LocalityNotEstablished {
                    message: message.clone(),
                })
            }
            smelt_logical::maintenance::Refusal::KeyedRecurrenceDeclarationMismatch { message } => {
                Some(MaintenanceRefusal::KeyedRecurrenceDeclarationMismatch {
                    message: message.clone(),
                })
            }
            smelt_logical::maintenance::Refusal::IdentityNotDerivable { message } => {
                Some(MaintenanceRefusal::IdentityNotDerivable {
                    message: message.clone(),
                })
            }
            // The repair family's two obligation refusals
            // (`MaintenanceRepairKeysNotDiscoverable`/
            // `MaintenanceRepairSliceUnbounded`) — `derive_new_data`
            // (`smelt-logical/src/maintenance/derive.rs`) already pushes
            // both when `repair::admit_per_group_recompute` refuses, but
            // neither has a `DiagnosticCode` variant yet. Left unmapped
            // exactly as `ReachNotDerivable` above, for the same reason: a
            // future phase's own diagnostic lands it.
            smelt_logical::maintenance::Refusal::RepairKeysNotDiscoverable { .. } => None,
            smelt_logical::maintenance::Refusal::RepairSliceUnbounded { .. } => None,
            smelt_logical::maintenance::Refusal::DefinitionChangeNotBackfillable {
                columns,
                why,
            } => Some(MaintenanceRefusal::DefinitionChangeNotBackfillable {
                columns: columns.clone(),
                why: why.clone(),
            }),
            smelt_logical::maintenance::Refusal::KeyedRetractableContribution {
                source,
                columns,
                why,
            } => Some(MaintenanceRefusal::KeyedRetractableContribution {
                source: source.clone(),
                columns: columns.clone(),
                why: why.clone(),
            }),
            // The eleven `Succession*` diagnostic codes land in phase 3a
            // (`docs/outcomes/20260906-scd2-keyed-succession/outcome.md`) —
            // left unmapped exactly as `ReachNotDerivable` above, for the
            // same reason: a future phase's own diagnostic lands it.
            smelt_logical::maintenance::Refusal::SuccessionNotRecognized { .. } => None,
        })
        .collect();
    let cell_column_group_violations = metadata
        .maintenance
        .as_ref()
        .map(|m| cell_column_group_violations(m, &result.column_groups))
        .unwrap_or_default();
    let write_pin_refusals = write_pin_diagnostics(
        metadata,
        &result.plan,
        &result.column_groups,
        active_backends,
        &result.comparability,
    );
    // Availability resolution for the two state-residency diagnostics
    // (`state.md` §Diagnostics `MaintenanceStateDowngraded` /
    // `DeclaredContractRequiresState`). Runs over a CLONE of the derived
    // cells — `result.plan` itself must stay ideal-derivation output, since
    // `smelt-runtime` and `smelt explain` resolve availability themselves
    // against the actual target dialect (plan 05/06's own posture: analysis
    // time has no single declared target). Checked against every declared
    // backend, the same all-declared-backends posture `write_pin_diagnostics`
    // uses; an empty `active_backends` (config unparseable) falls back to
    // `duckdb`, mirroring that function's own fallback.
    let availability_backends: Vec<String> = if active_backends.is_empty() {
        vec!["duckdb".to_string()]
    } else {
        active_backends.to_vec()
    };
    let realisable_for =
        |backend_name: &str| -> Vec<smelt_logical::maintenance::availability::StateStructure> {
            backend_dialect_for(backend_name)
                .map(smelt_logical::maintenance::availability::realisable_state_structures)
                .unwrap_or_default()
        };
    let mut state_downgrades: Vec<StateDowngradeDiagnostic> = Vec::new();
    for backend_name in &availability_backends {
        let realisable = realisable_for(backend_name);
        let availability = smelt_logical::maintenance::availability::StateAvailability::resolve(
            warehouse_tables,
            &realisable,
        );
        let mut cells = result.plan.cells.clone();
        smelt_logical::maintenance::availability::resolve_availability(&mut cells, &availability);
        for cell in &cells {
            let Some(downgrade) = &cell.state_downgrade else {
                continue;
            };
            let cell_label = format!("{:?}", cell.trigger);
            if state_downgrades.iter().any(|d| d.cell == cell_label) {
                continue;
            }
            state_downgrades.push(StateDowngradeDiagnostic {
                cell: cell_label,
                original_technique: format!("{:?}", downgrade.original),
                missing_structure: downgrade.missing.as_str().to_string(),
                backend: backend_name.clone(),
                reason: downgrade.reason.clone(),
            });
        }
    }
    let mut contract_state_refusals: Vec<ContractStateRefusalDiagnostic> = Vec::new();
    if let Some(contract) = metadata.contract.as_ref() {
        let mut declarations: Vec<String> = Vec::new();
        if contract.deferral.is_some() {
            declarations.push("contract.deferral".to_string());
        }
        for cell_cfg in &contract.cells {
            if cell_cfg.deferral.is_some() {
                declarations.push(format!("contract.cells[].deferral (on: {})", cell_cfg.on));
            }
        }
        if !declarations.is_empty() {
            // The concrete `d` value never changes which structure a
            // `Deferral` point requires (`required_state_structure` dispatches
            // on the variant, not its payload) — `0` is a placeholder.
            let point = smelt_logical::contract::ContractPoint::Deferral { d: 0 };
            if let Some(required) = smelt_logical::contract::required_state_structure(&point) {
                for declaration in declarations {
                    for backend_name in &availability_backends {
                        let realisable = realisable_for(backend_name);
                        let availability =
                            smelt_logical::maintenance::availability::StateAvailability::resolve(
                                warehouse_tables,
                                &realisable,
                            );
                        if !availability.contains(required) {
                            contract_state_refusals.push(ContractStateRefusalDiagnostic {
                                declaration: declaration.clone(),
                                missing_structure: required.as_str().to_string(),
                                backend: backend_name.clone(),
                            });
                            break;
                        }
                    }
                }
            }
        }
    }
    MaintenancePlanDiagnostics {
        refusals,
        cell_column_group_violations,
        granularity_mismatch,
        write_pin_refusals,
        scan_bounds_warnings,
        state_downgrades,
        contract_state_refusals,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use smelt_core::config::{MaintenanceCellConfig, PerSourceScanBounds};

    /// The PostgreSQL emission dialect is retired (#181): `Target::backend_type`
    /// already rejects `type: postgres` at the declaration boundary, and these
    /// two name-keyed resolvers must not resurrect it as a second, unguarded
    /// entry point — an unrecognised name resolves conservatively, matching the
    /// fail-loud posture both functions already take for any other unknown name.
    #[test]
    fn retired_backend_names_resolve_to_nothing() {
        for name in ["postgres", "postgresql"] {
            assert_eq!(
                backend_dialect_for(name),
                None,
                "{name} must not resolve to a SqlDialect"
            );
            let caps = backend_write_capabilities_for(name);
            assert_eq!(
                caps,
                smelt_logical::maintenance::BackendWriteCapabilities::default(),
                "{name} must resolve to the conservative default, not a real capability set"
            );
        }
    }

    fn group(columns: &[&str], sensitivity: &[&str]) -> ColumnGroup {
        ColumnGroup {
            columns: columns.iter().map(|s| s.to_string()).collect(),
            mutation_sensitivity: sensitivity.iter().map(|s| s.to_string()).collect(),
            membership_sensitivity: BTreeSet::new(),
        }
    }

    fn source_info_with_mutation(kind: Option<SourceMutationKind>) -> SourceInfo {
        SourceInfo {
            path: std::path::PathBuf::from("/tmp/s.yml"),
            address_segments: vec!["sources".to_string(), "s".to_string()],
            columns: vec![],
            description: None,
            name_override: None,
            tags: vec![],
            timeseries: None,
            mutation_profile: kind.map(smelt_core::sources::SourceMutationProfile::from_kind),
            source_lateness: None,
            watermark: None,
            unique_key: None,
            retention: None,
            referential_integrity: None,
        }
    }

    /// Phase 28c: a declared `mutation_profile: change_feed` source facts to
    /// `PlanMutationProfile::ChangeFeed` — while undeclared and `mutable_snapshot`
    /// both still fail closed to the stricter `MutableSnapshot` posture.
    #[test]
    fn source_facts_maps_declared_change_feed() {
        let feed = source_info_with_mutation(Some(SourceMutationKind::ChangeFeed));
        assert_eq!(
            source_facts("feed", Some(&feed), true).mutation,
            PlanMutationProfile::ChangeFeed
        );

        let mutable = source_info_with_mutation(Some(SourceMutationKind::Mutable));
        assert_eq!(
            source_facts("mutable", Some(&mutable), true).mutation,
            PlanMutationProfile::MutableSnapshot
        );

        assert_eq!(
            source_facts("undeclared", None, true).mutation,
            PlanMutationProfile::MutableSnapshot
        );
    }

    #[test]
    fn keyed_fold_write_pin_matches_on_the_driving_source_address() {
        let metadata = ModelMetadata {
            maintenance: Some(MaintenanceConfig {
                defaults: None,
                cells: vec![MaintenanceCellConfig {
                    columns: vec![],
                    on: "sources.events".to_string(),
                    prefer: None,
                    technique: None,
                    write: Some("staged_candidate".to_string()),
                }],
                scan_bounds: None,
            }),
            ..Default::default()
        };
        assert_eq!(
            keyed_fold_write_pin(&metadata, "sources.events"),
            Some("staged_candidate".to_string())
        );
    }

    #[test]
    fn keyed_fold_write_pin_ignores_a_cell_addressed_at_another_source() {
        let metadata = ModelMetadata {
            maintenance: Some(MaintenanceConfig {
                defaults: None,
                cells: vec![MaintenanceCellConfig {
                    columns: vec![],
                    on: "sources.other".to_string(),
                    prefer: None,
                    technique: None,
                    write: Some("staged_candidate".to_string()),
                }],
                scan_bounds: None,
            }),
            ..Default::default()
        };
        assert_eq!(keyed_fold_write_pin(&metadata, "sources.events"), None);
    }

    #[test]
    fn keyed_fold_effective_override_matches_by_on_address() {
        let metadata = ModelMetadata {
            maintenance: Some(MaintenanceConfig {
                defaults: None,
                cells: vec![MaintenanceCellConfig {
                    columns: vec![],
                    on: "sources.events".to_string(),
                    prefer: None,
                    technique: Some(smelt_core::config::CellTechnique::Unconditional),
                    write: None,
                }],
                scan_bounds: None,
            }),
            ..Default::default()
        };
        let effective = keyed_fold_effective_override(&metadata, "sources.events");
        assert_eq!(
            effective.technique,
            Some(smelt_core::config::CellTechnique::Unconditional)
        );

        let non_matching = keyed_fold_effective_override(&metadata, "sources.other");
        assert_eq!(non_matching.technique, None);
        assert_eq!(non_matching.prefer, None);
    }

    #[test]
    fn cells_columns_spanning_groups_error() {
        let groups = vec![
            group(&["converted"], &["payments"]),
            group(&["shipped"], &["shipments"]),
        ];
        let maintenance = MaintenanceConfig {
            defaults: None,
            cells: vec![MaintenanceCellConfig {
                columns: vec!["converted".to_string(), "shipped".to_string()],
                on: "sources.payments".to_string(),
                prefer: None,
                technique: None,
                write: None,
            }],
            scan_bounds: None,
        };
        let violations = cell_column_group_violations(&maintenance, &groups);
        assert_eq!(
            violations.len(),
            1,
            "expected exactly one violation, got {violations:?}"
        );
        assert!(violations[0].contains("sources.payments"));
    }

    #[test]
    fn cells_columns_within_one_group_ok() {
        let groups = vec![group(&["converted", "converted_at"], &["payments"])];
        let maintenance = MaintenanceConfig {
            defaults: None,
            cells: vec![MaintenanceCellConfig {
                columns: vec!["converted".to_string(), "converted_at".to_string()],
                on: "sources.payments".to_string(),
                prefer: None,
                technique: None,
                write: None,
            }],
            scan_bounds: None,
        };
        assert!(cell_column_group_violations(&maintenance, &groups).is_empty());
    }

    #[test]
    fn allow_full_scan_true_clears_scan_unbounded() {
        let sources = [source_facts("sources.enrichment", None, false)];
        assert!(!sources[0].allow_full_scan);
        let sources = [source_facts("sources.enrichment", None, true)];
        assert!(sources[0].allow_full_scan);
    }

    #[test]
    fn effective_scan_bounds_model_overrides_project() {
        let mut project = ScanBoundsConfig::default();
        project.per_source.insert(
            "sources.enrichment".to_string(),
            PerSourceScanBounds {
                max_lookback: None,
                allow_full_scan: false,
            },
        );
        let mut model = ScanBoundsConfig::default();
        model.per_source.insert(
            "sources.enrichment".to_string(),
            PerSourceScanBounds {
                max_lookback: None,
                allow_full_scan: true,
            },
        );
        let (allow, require, on_violation) =
            effective_scan_bounds("sources.enrichment", Some(&model), Some(&project));
        assert!(allow);
        assert_eq!(require, ScanBoundsRequire::PartitionLocal);
        assert_eq!(on_violation, ScanBoundsViolation::Error);
    }

    #[test]
    fn effective_scan_bounds_on_violation_model_overrides_project() {
        let project = ScanBoundsConfig {
            on_violation: Some(ScanBoundsViolation::Error),
            ..Default::default()
        };
        let model = ScanBoundsConfig {
            on_violation: Some(ScanBoundsViolation::Warn),
            ..Default::default()
        };
        let (_, _, on_violation) =
            effective_scan_bounds("sources.enrichment", Some(&model), Some(&project));
        assert_eq!(on_violation, ScanBoundsViolation::Warn);
    }

    #[test]
    fn grain_mismatch_never_admits_fold_without_aggregate() {
        // `grain: key` with a body that never aggregates has no fold
        // candidate — `derive_fold_spec` must return `None`, not fabricate
        // one, so the derivation's own admission refuses honestly.
        let sql = "SELECT user_id, amount FROM smelt.sources.payments";
        assert!(derive_fold_spec(sql, &[]).is_none());
    }

    #[test]
    fn grain_mismatch_detects_single_aggregate() {
        let sql =
            "SELECT user_id, SUM(amount) AS total FROM smelt.sources.payments GROUP BY user_id";
        let fold =
            derive_fold_spec(sql, &[]).expect("single SUM aggregate should be a fold candidate");
        assert_eq!(
            fold.add_columns,
            vec![("total".to_string(), SqlFunction::Sum)]
        );
    }

    #[test]
    fn grain_mismatch_detects_multiple_aggregates_with_mixed_combiners() {
        let sql = "SELECT user_id, COUNT(*) AS n, MIN(event_ts) AS first_seen, \
                    MAX(event_ts) AS last_seen FROM smelt.sources.events GROUP BY user_id";
        let fold =
            derive_fold_spec(sql, &[]).expect("multi-aggregate SELECT should be a fold candidate");
        assert_eq!(
            fold.add_columns,
            vec![
                ("n".to_string(), SqlFunction::Count),
                ("first_seen".to_string(), SqlFunction::Min),
                ("last_seen".to_string(), SqlFunction::Max),
            ]
        );
    }

    #[test]
    fn grain_mismatch_single_aggregate_shape_unchanged_at_n_equals_1() {
        // Regression: a single-aggregate SELECT derives the exact same
        // `FoldSpec` shape it did before multi-column folds were supported.
        let sql =
            "SELECT user_id, SUM(amount) AS total FROM smelt.sources.payments GROUP BY user_id";
        let fold =
            derive_fold_spec(sql, &[]).expect("single SUM aggregate should be a fold candidate");
        assert_eq!(fold.add_columns.len(), 1);
        assert_eq!(fold.add_columns[0], ("total".to_string(), SqlFunction::Sum));
    }

    #[test]
    fn grain_mismatch_unrecognized_aggregate_among_set_refuses_whole_derivation() {
        // Fail-closed: one recognised aggregate (SUM) alongside one
        // unresolvable projection (an unregistered window function — the
        // classifier's window-item branch admits it into `OtherAggregate`
        // without requiring the function name to resolve, unlike the
        // aggregate branch) must refuse the *whole* derivation, not
        // silently fold only the recognised column.
        let sql = "SELECT user_id, SUM(amount) AS total, \
                    NOT_A_REAL_FUNCTION() OVER (ORDER BY amount) AS weird \
                    FROM smelt.sources.payments GROUP BY user_id";
        assert!(
            derive_fold_spec(sql, &[]).is_none(),
            "an unresolvable aggregate/window item among the set must refuse the whole \
             derivation, not a partial fold"
        );
    }

    /// A `grain: key` model's derived plan carries the classifier's real
    /// `unique_key` (its own GROUP BY columns) — not a hardcoded empty vec.
    #[test]
    fn keyed_plan_carries_real_unique_key() {
        let sql = "SELECT device_id, user_id, COUNT(*) AS n \
                    FROM smelt.sources.events GROUP BY device_id, user_id";
        let metadata = ModelMetadata {
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(ConfigGrain::Key),
            ..Default::default()
        };
        let result = derive_model_maintenance_plan(
            sql,
            "main.device_user",
            &metadata,
            &[],
            &std::collections::HashSet::new(),
            None,
            &[],
            &[],
            &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
            None,
            None,
            &[],
        )
        .expect("grain: key model must derive a plan");
        // `derive_model_maintenance_plan` threads `derive_group_by_unique_key`
        // into `PlanGrain::Key` — assert the same derivation directly (the
        // plan itself does not yet re-expose the grain on a public surface,
        // `MaintenancePlanResult` carries only cells/refusals/column_groups).
        assert_eq!(
            derive_group_by_unique_key(sql),
            vec!["device_id".to_string(), "user_id".to_string()]
        );
        // Sanity: this model has no timeseries: block, so it must NOT hit
        // the locality refusal — it derives ordinary cells/no-cells like
        // any other grain: key model (no admission assertion beyond "no
        // locality refusal" — the fold/aggregate admission is exercised by
        // other tests).
        assert!(
            !result.plan.refusals.iter().any(|r| matches!(
                r,
                smelt_logical::maintenance::Refusal::LocalityNotEstablished { .. }
            )),
            "no timeseries: block declared — must not hit the locality gate: {:?}",
            result.plan.refusals
        );
    }

    /// The W1 flagship repro (`docs/plans/20260715-composed-axes-conditional-maintenance.md`
    /// Blocked-phases entry): six `MIN`-folded payload columns over one key. Before this
    /// phase, `derive_fold_spec` only admitted a single aggregate column, so `inputs.fold`
    /// stayed `None` and the `NewData` cell refused with "keyed grain with no fold
    /// specification" — reproduced here at the unit level (no example workspace staged).
    #[test]
    fn keyed_six_column_extremal_fold_no_longer_refuses_for_missing_fold_spec() {
        let sql = "SELECT event_id, MIN(device_id) AS device_id, MIN(user_id) AS user_id, \
                    MIN(event_time) AS event_ts, MIN(event_date) AS first_seen_date, \
                    MIN(utm_campaign) AS utm_campaign, MIN(payload) AS payload \
                    FROM smelt.sources.raw.events GROUP BY event_id";
        let fold =
            derive_fold_spec(sql, &[]).expect("six-column MIN fold should be a fold candidate");
        assert_eq!(fold.add_columns.len(), 6);
        assert!(fold.add_columns.iter().all(|(_, c)| *c == SqlFunction::Min));

        let metadata = ModelMetadata {
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(ConfigGrain::Key),
            ..Default::default()
        };
        let result = derive_model_maintenance_plan(
            sql,
            "main.events_deduped",
            &metadata,
            &[],
            &std::collections::HashSet::new(),
            None,
            &[],
            &[],
            &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
            None,
            None,
            &[],
        )
        .expect("grain: key model must derive a plan");
        assert!(
            !result.plan.refusals.iter().any(|r| matches!(
                r,
                smelt_logical::maintenance::Refusal::NoAdmissibleTechnique { why, .. }
                    if why.contains("no fold specification")
            )),
            "multi-column fold must be derived — the 'no fold specification' refusal must not \
             recur: {:?}",
            result.plan.refusals
        );
    }

    /// A `grain: key` model that also declares a `timeseries:` block, but
    /// whose `partition_column` is not a `unique_key` column and has no
    /// resolvable driving source, is refused by the key-temporal-locality
    /// gate (`docs/specs/incremental_shapes.md` §"Key temporal locality
    /// (the time-partitioned output)") — no route admits it.
    #[test]
    fn keyed_with_timeseries_refuses_via_locality_gate() {
        use smelt_core::config::{Granularity, TimeseriesConfig};

        let sql = "SELECT device_id, COUNT(*) AS n FROM smelt.sources.events GROUP BY device_id";
        let metadata = ModelMetadata {
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(ConfigGrain::Key),
            timeseries: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            ..Default::default()
        };
        let result = derive_model_maintenance_plan(
            sql,
            "main.device_daily",
            &metadata,
            &[],
            &std::collections::HashSet::new(),
            None,
            &[],
            &[],
            &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
            None,
            None,
            &[],
        )
        .expect("grain: key + timeseries: must still derive a (refused) plan");
        assert!(
            result.plan.cells.is_empty(),
            "a locality-refused model must admit no cells: {:?}",
            result.plan.cells
        );
        assert_eq!(result.plan.refusals.len(), 1, "{:?}", result.plan.refusals);
        match &result.plan.refusals[0] {
            smelt_logical::maintenance::Refusal::LocalityNotEstablished { message } => {
                assert!(
                    message.contains("KeyedForbidsTimeseries"),
                    "message: {message}"
                );
            }
            other => panic!("expected LocalityNotEstablished, got {other:?}"),
        }
    }

    /// Route 1 (key-embedded) admits through the full `smelt-db` plumbing:
    /// `partition_column` (`event_date`) is a `unique_key` column, the
    /// single referenced source is clocked at the same (day) granularity —
    /// the model derives an ordinary plan with no `LocalityNotEstablished`
    /// refusal (`docs/specs/incremental_shapes.md` §"Key temporal
    /// locality").
    #[test]
    fn keyed_with_timeseries_admits_via_route1_key_embedded() {
        use smelt_core::config::{Granularity, TimeseriesConfig};

        let sql = "SELECT device_id, event_date, COUNT(*) AS n \
                    FROM smelt.sources.events GROUP BY device_id, event_date";
        let metadata = ModelMetadata {
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(ConfigGrain::Key),
            timeseries: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            ..Default::default()
        };
        let sources = vec![SourceFacts {
            // `SourceFacts::name` is the *bare* source name (`sources.`
            // breadcrumb stripped) — the real convention `smelt-db::lib`
            // builds (`ref_string.strip_prefix("smelt.").and_then(|s|
            // s.strip_prefix("sources."))`), which `locality::
            // resolve_driving_source` matches against.
            name: "events".to_string(),
            mutation: PlanMutationProfile::AppendOnly,
            partition_col: Some("event_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        }];
        let result = derive_model_maintenance_plan(
            sql,
            "main.device_daily",
            &metadata,
            &sources,
            &std::collections::HashSet::new(),
            Some(Granularity::Day),
            &[],
            &[],
            &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
            None,
            None,
            &[],
        )
        .expect("route 1 must derive a plan");
        assert!(
            !result.plan.refusals.iter().any(|r| matches!(
                r,
                smelt_logical::maintenance::Refusal::LocalityNotEstablished { .. }
            )),
            "route 1 must admit — no locality refusal expected: {:?}",
            result.plan.refusals
        );
    }

    /// A source declaring `referential_integrity` in its `.yml` reaches
    /// `derive_model_maintenance_plan` as a real `SourceReferentialIntegrity`
    /// entry (`docs/outcomes/20260809-probe-backed-facts/phases/03-plan.md`
    /// test 9) — the production Salsa call site's own always-empty map
    /// (before this phase) is replaced by [`build_source_referential_
    /// integrity`], threaded from `source_refs`. A `dim` source declaring
    /// both `unique_key` and `referential_integrity` closes its own
    /// `UpstreamMutation` cell's P1 verdict; the same call with an empty map
    /// (byte-identical to the pre-phase-3 default) leaves it unattempted.
    #[test]
    fn source_declared_referential_integrity_reaches_the_derivation() {
        let sql = "SELECT fact.event_id, fact.event_date, dim.tier \
                    FROM smelt.sources.fact fact \
                    LEFT JOIN smelt.sources.dim dim ON fact.dim_id = dim.id";
        let metadata = ModelMetadata {
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(ConfigGrain::Partition),
            timeseries: Some(smelt_core::config::TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            ..Default::default()
        };
        let sources = vec![
            SourceFacts {
                name: "fact".to_string(),
                mutation: PlanMutationProfile::AppendOnly,
                partition_col: Some("event_date".to_string()),
                unique_key: vec![],
                allow_full_scan: true,
            },
            SourceFacts {
                name: "dim".to_string(),
                mutation: PlanMutationProfile::MutableSnapshot,
                partition_col: None,
                unique_key: vec!["id".to_string()],
                allow_full_scan: true,
            },
        ];
        let explicitly_mutable: std::collections::HashSet<String> =
            std::collections::HashSet::from(["dim".to_string()]);
        let dim_source_info = SourceInfo {
            path: std::path::PathBuf::from("/tmp/dim.yml"),
            address_segments: vec!["sources".to_string(), "dim".to_string()],
            columns: vec![],
            description: None,
            name_override: None,
            tags: vec![],
            timeseries: None,
            mutation_profile: None,
            source_lateness: None,
            watermark: None,
            unique_key: Some(vec!["id".to_string()]),
            retention: None,
            referential_integrity: Some(vec!["id".to_string()]),
        };
        let source_refs: Vec<(String, Option<SourceInfo>)> =
            vec![("dim".to_string(), Some(dim_source_info))];
        let real_ri = build_source_referential_integrity(&source_refs);
        assert_eq!(
            real_ri.get("dim"),
            Some(&vec!["id".to_string()]),
            "build_source_referential_integrity must surface dim's declared \
             referential_integrity, got {real_ri:?}"
        );

        let trigger = smelt_logical::maintenance::Trigger::UpstreamMutation {
            source: "dim".to_string(),
        };
        let with_real_ri = derive_model_maintenance_plan(
            sql,
            "main.t",
            &metadata,
            &sources,
            &explicitly_mutable,
            None,
            &[],
            &[],
            &real_ri,
            None,
            None,
            &[],
        )
        .expect("model must derive a plan");
        let cell = with_real_ri
            .plan
            .cell_for(&trigger)
            .expect("expected an UpstreamMutation cell for dim");
        assert_eq!(
            cell.skeleton_source_closure.as_ref().map(|c| c.is_closed()),
            Some(true),
            "a LEFT JOIN, payload-only, declared-unique_key dimension must close once its \
             declared referential_integrity reaches the derivation, got {:?}",
            cell.skeleton_source_closure
        );

        let with_empty_ri = derive_model_maintenance_plan(
            sql,
            "main.t",
            &metadata,
            &sources,
            &explicitly_mutable,
            None,
            &[],
            &[],
            &SourceReferentialIntegrity::new(),
            None,
            None,
            &[],
        )
        .expect("model must derive a plan");
        let cell = with_empty_ri
            .plan
            .cell_for(&trigger)
            .expect("expected an UpstreamMutation cell for dim");
        assert_eq!(
            cell.skeleton_source_closure, None,
            "an empty referential-integrity map (the pre-phase-3 default) must leave the \
             closure proof unattempted, got {:?}",
            cell.skeleton_source_closure
        );
    }

    /// Multi-source regression for the driving-source resolution this
    /// phase's review fixed: `smelt-db`'s plan-derivation call site and
    /// `smelt-runtime`'s runtime execution path (`classify_cumulative`)
    /// must agree on which source drives a model. A clocked source
    /// referenced only inside a CTE — never joined into the outer
    /// SELECT's FROM/JOIN — must not count as a second driving-source
    /// candidate here, exactly as it would not for the runtime's
    /// alias-scoped `classify_cumulative` resolution
    /// (`smelt_logical::maintenance::locality::resolve_driving_source`).
    /// Before that shared resolution existed, this call site resolved the
    /// driving source over *every* referenced source — seeing two clocked
    /// sources here, it would treat the driving source as unresolved and
    /// refuse route 1 (`KeyedForbidsTimeseries` via `smelt explain`) even
    /// though `smelt build` would actually admit and execute the model.
    #[test]
    fn multi_source_model_agrees_with_runtime_alias_scoped_driving_source() {
        use smelt_core::config::{Granularity, TimeseriesConfig};

        let sql = "WITH other AS ( \
                       SELECT device_id, event_date FROM smelt.sources.other_stream \
                   ) \
                   SELECT device_id, event_date, COUNT(*) AS n \
                   FROM smelt.sources.events \
                   GROUP BY device_id, event_date";
        let metadata = ModelMetadata {
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(ConfigGrain::Key),
            timeseries: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            ..Default::default()
        };
        let sources = vec![
            // `SourceFacts::name` is the bare source name — the real
            // convention `smelt-db::lib` builds and `locality::
            // resolve_driving_source` matches against.
            SourceFacts {
                name: "events".to_string(),
                mutation: PlanMutationProfile::AppendOnly,
                partition_col: Some("event_date".to_string()),
                unique_key: vec![],
                allow_full_scan: false,
            },
            // Clocked, but only ever referenced inside the CTE — never
            // joined into the outer SELECT's FROM/JOIN. Must NOT be
            // treated as a second driving-source candidate.
            SourceFacts {
                name: "other_stream".to_string(),
                mutation: PlanMutationProfile::AppendOnly,
                partition_col: Some("event_date".to_string()),
                unique_key: vec![],
                allow_full_scan: false,
            },
        ];
        let result = derive_model_maintenance_plan(
            sql,
            "main.device_daily",
            &metadata,
            &sources,
            &std::collections::HashSet::new(),
            Some(Granularity::Day),
            &[],
            &[],
            &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
            None,
            None,
            &[],
        )
        .expect("route 1 must derive a plan");
        assert!(
            !result.plan.refusals.iter().any(|r| matches!(
                r,
                smelt_logical::maintenance::Refusal::LocalityNotEstablished { .. }
            )),
            "the CTE-only clocked source must not defeat route 1 admission — the driving \
             source must resolve to the outer FROM/JOIN's alias-scoped `sources.events` alone, \
             matching the runtime's `classify_cumulative` resolution: {:?}",
            result.plan.refusals
        );
    }

    const SUCCESSION_SQL: &str = "SELECT \
         customer_id, \
         changed_at, \
         name, \
         LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_changed_at \
         FROM smelt.sources.customer_changes";

    fn succession_source_info() -> SourceInfo {
        SourceInfo {
            path: std::path::PathBuf::from("/tmp/customer_changes.yml"),
            address_segments: vec!["sources".to_string(), "customer_changes".to_string()],
            columns: vec![
                smelt_core::sources::SourceColumn {
                    name: "customer_id".to_string(),
                    data_type: smelt_types::DataType::Integer,
                    nullable: false,
                    description: None,
                },
                smelt_core::sources::SourceColumn {
                    name: "changed_at".to_string(),
                    data_type: smelt_types::DataType::Timestamp {
                        with_timezone: false,
                    },
                    nullable: false,
                    description: None,
                },
            ],
            description: None,
            name_override: None,
            tags: vec![],
            timeseries: Some(smelt_core::config::TimeseriesConfig {
                event_time_column: "changed_at".to_string(),
                partition_column: "changed_at".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            mutation_profile: Some(smelt_core::sources::SourceMutationProfile::from_kind(
                SourceMutationKind::AppendOnly,
            )),
            source_lateness: None,
            watermark: None,
            unique_key: None,
            retention: None,
            referential_integrity: None,
        }
    }

    fn succession_metadata() -> ModelMetadata {
        ModelMetadata {
            refresh: Some(RefreshStrategy::Incremental),
            ..Default::default()
        }
    }

    #[test]
    fn undeclared_grain_incremental_model_derives_the_succession_plan() {
        let metadata = succession_metadata();
        let source_refs = vec![(
            "customer_changes".to_string(),
            Some(succession_source_info()),
        )];
        let result = derive_model_maintenance_plan(
            SUCCESSION_SQL,
            "main.customer_history",
            &metadata,
            &[],
            &std::collections::HashSet::new(),
            None,
            &[],
            &[],
            &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
            None,
            None,
            &source_refs,
        )
        .expect(
            "undeclared-grain incremental model with a succession-shaped SQL must derive a plan",
        );
        assert!(
            result.plan.refusals.is_empty(),
            "expected the succession cell to admit cleanly: {:?}",
            result.plan.refusals
        );
        assert_eq!(result.plan.cells.len(), 1);
        assert_eq!(
            result.plan.cells[0].technique,
            smelt_logical::maintenance::Technique::SuccessionPatch
        );
        assert_eq!(
            result.plan.cells[0].trigger,
            Trigger::NewData {
                source: "customer_changes".to_string()
            }
        );
    }

    #[test]
    fn undeclared_grain_unrecognised_shape_derives_the_succession_refusal() {
        let metadata = succession_metadata();
        let sql = "SELECT customer_id, COUNT(*) AS n FROM smelt.sources.customer_changes GROUP BY customer_id";
        let result = derive_model_maintenance_plan(
            sql,
            "main.customer_counts",
            &metadata,
            &[],
            &std::collections::HashSet::new(),
            None,
            &[],
            &[],
            &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
            None,
            None,
            &[],
        )
        .expect("undeclared-grain incremental model must still derive a (refused) plan");
        assert!(result.plan.cells.is_empty());
        assert_eq!(result.plan.refusals.len(), 1);
        assert!(matches!(
            result.plan.refusals[0],
            smelt_logical::maintenance::Refusal::SuccessionNotRecognized { .. }
        ));
    }

    #[test]
    fn succession_context_is_built_from_the_source_declarations() {
        let source_refs = vec![(
            "customer_changes".to_string(),
            Some(succession_source_info()),
        )];
        let ctx = build_succession_context(SUCCESSION_SQL, &source_refs);
        assert_eq!(ctx.source_name, "sources.customer_changes");
        assert_eq!(ctx.event_time_column.as_deref(), Some("changed_at"));
        assert!(ctx.not_null_columns.contains("customer_id"));
        assert!(ctx.not_null_columns.contains("changed_at"));

        // Undeclared profile fails closed: no `SourceInfo` for the driving
        // source resolves to an empty/`None`-carrying context, never a panic.
        let ctx_undeclared = build_succession_context(SUCCESSION_SQL, &[]);
        assert_eq!(ctx_undeclared.mutation_profile, None);
        assert_eq!(ctx_undeclared.event_time_column, None);
        assert!(ctx_undeclared.not_null_columns.is_empty());
    }

    #[test]
    fn declared_grain_models_are_unchanged() {
        // A `grain: partition` model.
        let partition_sql =
            "SELECT event_date, COUNT(*) AS n FROM smelt.sources.events GROUP BY event_date";
        let partition_metadata = ModelMetadata {
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(ConfigGrain::Partition),
            timeseries: Some(smelt_core::config::TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            ..Default::default()
        };
        let partition_sources = vec![SourceFacts {
            name: "events".to_string(),
            mutation: PlanMutationProfile::AppendOnly,
            partition_col: Some("event_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        }];
        let with_refs = derive_model_maintenance_plan(
            partition_sql,
            "main.events_daily",
            &partition_metadata,
            &partition_sources,
            &std::collections::HashSet::new(),
            None,
            &[],
            &[],
            &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
            None,
            None,
            &[(
                "customer_changes".to_string(),
                Some(succession_source_info()),
            )],
        )
        .expect("grain: partition model must derive a plan");
        let without_refs = derive_model_maintenance_plan(
            partition_sql,
            "main.events_daily",
            &partition_metadata,
            &partition_sources,
            &std::collections::HashSet::new(),
            None,
            &[],
            &[],
            &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
            None,
            None,
            &[],
        )
        .expect("grain: partition model must derive a plan");
        assert_eq!(with_refs.plan.cells.len(), without_refs.plan.cells.len());
        assert_eq!(
            with_refs.plan.cells[0].technique,
            without_refs.plan.cells[0].technique
        );

        // A `grain: key` model.
        let key_sql = "SELECT user_id, SUM(amount) AS lifetime_spend FROM smelt.sources.payments GROUP BY user_id";
        let key_metadata = ModelMetadata {
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(ConfigGrain::Key),
            ..Default::default()
        };
        let key_sources = vec![SourceFacts {
            name: "payments".to_string(),
            mutation: PlanMutationProfile::AppendOnly,
            partition_col: Some("pay_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        }];
        let key_result = derive_model_maintenance_plan(
            key_sql,
            "main.lifetime_spend",
            &key_metadata,
            &key_sources,
            &std::collections::HashSet::new(),
            None,
            &[],
            &[],
            &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
            None,
            None,
            &[(
                "customer_changes".to_string(),
                Some(succession_source_info()),
            )],
        )
        .expect("grain: key model must derive a plan");
        assert!(!key_result.plan.cells.is_empty());
    }
}
