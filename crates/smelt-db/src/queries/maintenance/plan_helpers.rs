use super::*;

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
