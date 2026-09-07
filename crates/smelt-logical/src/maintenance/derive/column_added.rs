use super::*;

/// Extract one named column's [`ColumnDef`] (name + defining expression)
/// from `sql`'s own outermost `SELECT` scope — a `ColumnAdded` trigger fires
/// because `name` already exists in the model's current `sql`, so this
/// resolves the [`crate::analysis::definition_change::classify_definition_
/// change`] proof's `added_column` argument straight from the same source
/// the rest of this derivation already reads. `None` when `sql` has no
/// classifiable top-level `SELECT`, or `name` isn't one of its projected
/// aliases — the caller fails closed rather than guessing an expression.
pub fn column_def_from_sql(sql: &str, name: &str) -> Option<ColumnDef> {
    let stripped = crate::types::Frontmatter::strip(sql);
    let parse = smelt_parser::parse(stripped);
    let file = smelt_parser::File::cast(parse.syntax())?;
    let select = file.select_stmt()?;
    let items = select_stmt_items(&select)?;
    items
        .iter()
        .find(|item| item_alias(item) == name)
        .map(|item| ColumnDef {
            name: name.to_string(),
            expr: item_expr(item).clone(),
        })
}

/// Diff `sql`'s own currently-projected output columns against
/// `deployed_column_names` (a prior deployed-schema snapshot's column
/// names, world-fact supplied by the caller — this function does no I/O of
/// its own, per the Salsa-purity/plan-purity rule) to derive the two
/// ingredients a production `Trigger::ColumnAdded` needs:
///
/// - `old_columns` — every currently-projected column whose name is ALSO
///   in `deployed_column_names` (i.e. still-present, pre-existing output
///   columns), with its defining expression read from `sql` itself. This
///   is [`ModelInputs::old_columns`] — [`classify_definition_change`]'s
///   `ctx.old_columns` (leg 2's collision check, leg 3's "already stored"
///   set).
/// - `added` — every currently-projected column name that is NOT in
///   `deployed_column_names`: the genuinely new columns a `Trigger::
///   ColumnAdded { columns: added }` should carry.
///
/// `None` when `sql` has no classifiable top-level `SELECT` — the caller
/// fails closed (no trigger derived) rather than guessing, exactly like
/// [`column_def_from_sql`] above.
pub fn diff_deployed_columns(
    sql: &str,
    deployed_column_names: &[String],
) -> Option<(Vec<ColumnDef>, Vec<String>)> {
    let stripped = crate::types::Frontmatter::strip(sql);
    let parse = smelt_parser::parse(stripped);
    let file = smelt_parser::File::cast(parse.syntax())?;
    let select = file.select_stmt()?;
    let items = select_stmt_items(&select)?;
    let deployed: std::collections::HashSet<&str> =
        deployed_column_names.iter().map(|s| s.as_str()).collect();
    let mut old_columns = Vec::new();
    let mut added = Vec::new();
    for item in &items {
        let name = item_alias(item);
        if deployed.contains(name) {
            old_columns.push(ColumnDef {
                name: name.to_string(),
                expr: item_expr(item).clone(),
            });
        } else {
            added.push(name.to_string());
        }
    }
    Some((old_columns, added))
}

/// Prove (or refuse to prove) that `inputs.sql`'s skeleton clause — the
/// FROM/JOIN tree, GROUP BY/DISTINCT, and the row-set/ordering/row-count
/// post-processing clauses — is unchanged against `inputs.old_sql`, using
/// the same clause-level factoring `smelt migrate` uses
/// ([`crate::backbuild::diff::definition_diff`]). Returns `Some(reason)`
/// when [`crate::backbuild::SkeletonDiff::Changed`] proves a clause
/// difference — the `Refusal::SkeletonClauseChanged` this function's caller
/// pushes. Returns `None` when there is no deployed SQL to compare
/// (`inputs.old_sql` is `None`), when either version fails to parse as a
/// plain `SELECT`, or when the diff proves the skeleton
/// [`crate::backbuild::SkeletonDiff::Unchanged`] or only gained `LEFT
/// JOIN`s (`AddedLeftJoins`, admissible — research §4's G0 class). A
/// `DefinitionDiff::Opaque` diff (unparseable, or a changed `WITH`-prefix)
/// is deliberately NOT treated as a skeleton change here — this check only
/// ever *adds* a refusal on a positive proof, never on an inability to
/// prove either way, matching every other `deployed_*`-gated derivation in
/// this module.
pub(super) fn skeleton_clause_changed(inputs: &ModelInputs) -> Option<String> {
    let old_sql = inputs.old_sql?;
    let old_stripped = crate::types::Frontmatter::strip(old_sql);
    let new_stripped = crate::types::Frontmatter::strip(inputs.sql);
    let old_parse = smelt_parser::parse(old_stripped);
    let new_parse = smelt_parser::parse(new_stripped);
    let old_file = smelt_parser::File::cast(old_parse.syntax())?;
    let new_file = smelt_parser::File::cast(new_parse.syntax())?;
    match crate::backbuild::diff::definition_diff(&old_file, &new_file) {
        crate::backbuild::DefinitionDiff::Comparable(diff) => match diff.skeleton {
            crate::backbuild::SkeletonDiff::Changed { reason, .. } => Some(reason),
            crate::backbuild::SkeletonDiff::Unchanged
            | crate::backbuild::SkeletonDiff::AddedLeftJoins(_) => None,
        },
        crate::backbuild::DefinitionDiff::Opaque { .. } => None,
    }
}

/// Prove (or refuse to prove) that `inputs`'s declared
/// `timeseries.partition_column` is unchanged against `inputs.old_partition_
/// col` — the recorded address from the deployed-schema snapshot. Returns
/// `Some((from, to))` naming the recorded and current column when they
/// differ (ASCII-case-insensitive compare), the `Refusal::
/// PartitionColumnChanged` this function's caller pushes. Returns `None`
/// when there is no recorded address (`inputs.old_partition_col` is `None`)
/// or the output has no partition axis at all (a `Grain::Key` output) — a
/// keyed model's identity is its `unique_key`, not a partition column, so
/// there is no address to compare.
pub(super) fn partition_column_changed(inputs: &ModelInputs) -> Option<(String, String)> {
    let old = inputs.old_partition_col?;
    let current = inputs.output_partition_col()?;
    if old.eq_ignore_ascii_case(current) {
        None
    } else {
        Some((old.to_string(), current.to_string()))
    }
}

/// Definition change: the model gained fields. Skeleton adds are grain
/// changes and refuse (EX-39); payload adds land in the 2×2's left column by
/// what they read (EX-36/37/40), instantiating their ledger entries at
/// `S = ∅` (the catch-up flag).
pub(super) fn derive_column_added(
    inputs: &ModelInputs,
    loc: &LocalityInputs<'_>,
    columns: &[String],
    identity: &RowIdentityVerdict,
    plan: &mut MaintenancePlan,
) {
    let trigger = Trigger::ColumnAdded {
        columns: columns.to_vec(),
    };
    // Boundary first: a skeleton-position add changes which rows exist.
    // Two independent proofs must both clear every added column before
    // *either* branch below (empty-sensitivity in-place update, or the
    // mutation-sensitive column-scoped merge) is allowed to dispatch it:
    // the hand-declared `output.skeleton_columns` set, and — when the
    // model's current SQL is classifiable — the derived skeleton-role
    // extraction (`maintenance::skeleton::skeleton_roles`). Only the
    // declared-set check used to run here; the derived check ran solely
    // inside `classify_definition_change`, which the non-empty-sensitivity
    // branch never calls, so a `GROUP BY` key absent from the declared set
    // could reach `ColumnScopedMerge` in that branch undetected. Computing
    // `skeleton_roles` once here, ahead of both branches, closes that gap.
    // An unclassifiable shape (`skeleton_roles` returns `None`) does not
    // newly refuse a model that relied on the declared set alone —
    // fail-closed without over-refusing.
    let derived_roles = skeleton_roles(
        inputs.sql,
        inputs.declared_unique_key(),
        inputs.output_partition_col(),
    );
    for col in columns {
        if inputs.output.skeleton_columns.contains(col) {
            plan.refusals.push(Refusal::SkeletonChanged {
                column: col.clone(),
            });
            return;
        }
        if let Some(role) = derived_roles
            .as_ref()
            .and_then(|roles| roles.iter().find(|(name, _)| name == col))
            .map(|(_, role)| *role)
        {
            if role.is_skeleton() {
                plan.refusals.push(Refusal::SkeletonChanged {
                    column: col.clone(),
                });
                return;
            }
        }
    }

    // The added fields factor by shared mutation-sensitivity exactly as the
    // base plan does; each added group gets its own catch-up op.
    for group in inputs
        .column_groups
        .iter()
        .filter(|g| g.columns.iter().any(|c| columns.contains(c)))
    {
        if group.mutation_sensitivity.is_empty() {
            // Empty mutation-sensitivity is *eligible* for the cheap
            // in-place update, but is not on its own proof of purity — it
            // can also mean "an append-only source read that is never
            // re-mutated after creation, yet was never stored before" (the
            // misclassification `classify_definition_change` exists to
            // correct, `model_properties.md` §"Definition-change column
            // classification"). Every added column in this group is run
            // through the composed proof; a `PureBackfill` verdict admits
            // the in-place `UPDATE`, a `SkeletonAdd` verdict this group's
            // hand-declared `output.skeleton_columns` didn't already catch
            // still refuses as a grain change, and an `UpstreamRederive`
            // verdict — or any refusal — fails closed here: this group
            // carries no source name to scan (that is exactly what "empty
            // mutation-sensitivity" means), so a column-scoped merge cannot
            // be constructed from the inputs this v0 derivation has
            // (production `ColumnAdded` trigger derivation, which could
            // supply one, stays out of this phase's scope).
            let added_in_group: Vec<&String> = group
                .columns
                .iter()
                .filter(|c| columns.contains(c))
                .collect();
            let mut verdict: Option<DefinitionChangeClass> = None;
            let mut refused: Option<String> = None;
            'columns: for c in &added_in_group {
                let Some(def) = column_def_from_sql(inputs.sql, c) else {
                    refused = Some(format!(
                        "could not resolve '{c}''s expression in the model's own SQL"
                    ));
                    break 'columns;
                };
                let ctx = DefinitionChangeCtx {
                    old_columns: &inputs.old_columns,
                    declared_unique_key: inputs.declared_unique_key(),
                    partition_col: inputs.output_partition_col(),
                    declared_skeleton_columns: &inputs.output.skeleton_columns,
                    monotone_dims: &[],
                };
                match classify_definition_change(&def, inputs.sql, &ctx) {
                    Ok(DefinitionChangeClass::SkeletonAdd { .. }) => {
                        plan.refusals.push(Refusal::SkeletonChanged {
                            column: (*c).clone(),
                        });
                        return;
                    }
                    Ok(v) => match &verdict {
                        None => verdict = Some(v),
                        Some(prev) if *prev == v => {}
                        Some(_) => {
                            refused = Some(
                                "group columns disagree on definition-change classification"
                                    .to_string(),
                            );
                            break 'columns;
                        }
                    },
                    Err(e) => {
                        refused = Some(format!("{e:?}"));
                        break 'columns;
                    }
                }
            }
            match (verdict, refused) {
                (Some(DefinitionChangeClass::PureBackfill), None) => {
                    plan.cells.push(PlanCell {
                        group: group.name(),
                        trigger: trigger.clone(),
                        corner: Corner::FoldDelta,
                        technique: Technique::InPlaceUpdate,
                        partition_local: PartitionLocal::Yes,
                        scans: vec![],
                        ledger_catch_up: true,
                        row_identity: identity.clone(),
                        skeleton_source_closure: None,
                        fingerprint_projections: BTreeMap::new(),
                        key_scope: None,
                        state_downgrade: None,
                    });
                }
                (Some(DefinitionChangeClass::UpstreamRederive), None) => {
                    plan.refusals
                        .push(Refusal::DefinitionChangeNotBackfillable {
                            columns: added_in_group.iter().map(|c| (*c).clone()).collect(),
                            why: "re-derives from upstream, but this group's mutation-sensitivity \
                              names no source to scan — a column-scoped merge cannot be \
                              constructed"
                                .to_string(),
                        });
                }
                (_, Some(why)) => {
                    plan.refusals
                        .push(Refusal::DefinitionChangeNotBackfillable {
                            columns: added_in_group.iter().map(|c| (*c).clone()).collect(),
                            why,
                        });
                }
                (_, None) => {
                    plan.refusals
                        .push(Refusal::DefinitionChangeNotBackfillable {
                            columns: added_in_group.iter().map(|c| (*c).clone()).collect(),
                            why: "in-place update not proven additive-only".to_string(),
                        });
                }
            }
            continue;
        }

        // Re-derives from upstream: column-scoped MERGE. Every read source
        // must be linked to the output partition axis or explicitly accepted
        // as a full read (EX-38: the field-add inherits its source's
        // partition-locality verdict unchanged).
        let mut scans = Vec::new();
        let mut locality = PartitionLocal::Yes;
        let mut refused = false;
        for source_name in &group.mutation_sensitivity {
            let Some(facts) = inputs.source(source_name) else {
                plan.refusals.push(Refusal::NoAdmissibleTechnique {
                    trigger: format!("{trigger:?}"),
                    why: format!("unknown source '{source_name}'"),
                });
                refused = true;
                break;
            };
            match project_source_link(
                inputs.output_partition_col(),
                inputs.keyed_time_axis,
                loc,
                facts,
            ) {
                SourceLink::Clamp(clamp) => scans.push(clamp),
                SourceLink::Unclocked | SourceLink::Unlinked { .. } if !facts.allow_full_scan => {
                    plan.refusals
                        .push(Refusal::DefinitionChangeNotBackfillable {
                            columns: group
                                .columns
                                .iter()
                                .filter(|c| columns.contains(c))
                                .cloned()
                                .collect(),
                            why: format!(
                                "backfill of {} reads '{}' with no partition bound",
                                group.name(),
                                facts.name
                            ),
                        });
                    refused = true;
                    break;
                }
                SourceLink::Unclocked => {
                    locality = PartitionLocal::No {
                        source: facts.name.clone(),
                        why: "unclocked source read in full (declared)".to_string(),
                    };
                }
                SourceLink::Unlinked { why } => {
                    locality = PartitionLocal::No {
                        source: facts.name.clone(),
                        why: format!("{why} (declared full scan)"),
                    };
                }
            }
        }
        if refused {
            continue;
        }
        plan.cells.push(PlanCell {
            group: group.name(),
            trigger: trigger.clone(),
            corner: Corner::ColumnMerge,
            technique: Technique::ColumnScopedMerge,
            partition_local: locality,
            scans,
            ledger_catch_up: true,
            row_identity: identity.clone(),
            skeleton_source_closure: None,
            fingerprint_projections: BTreeMap::new(),
            key_scope: None,
            state_downgrade: None,
        });
    }
}
