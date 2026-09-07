use super::*;

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
