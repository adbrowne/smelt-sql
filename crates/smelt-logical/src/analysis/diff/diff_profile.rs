use super::*;

/// Diff two [`PropertyProfile`]s, with no `..` rest pattern (mirrors
/// [`diff_property_set`]'s field-coverage guarantee for the top-level
/// profile shape).
pub(super) fn diff_profile(old: &PropertyProfile, new: &PropertyProfile) -> Vec<Change> {
    let PropertyProfile {
        properties: old_properties,
        cell_verdicts: old_cells,
        refusals: old_refusals,
        probes: old_probes,
    } = old;
    let PropertyProfile {
        properties: new_properties,
        cell_verdicts: new_cells,
        refusals: new_refusals,
        probes: new_probes,
    } = new;

    let mut kinds = diff_property_set(old_properties, new_properties);
    kinds.extend(diff_cell_verdicts(old_cells, new_cells));
    // G1 (`docs/outcomes/20260905-property-diff` fix round 1): emitted ONCE
    // here, at the profile level, never derived from the per-cell
    // `cell_removed`/`cell_added` changes above — `cell_removed` stays
    // `Neutral` in this case (see its own doc comment), so this is the only
    // signal that "no longer incrementally maintained" surfaces as. This is
    // the fix for the case a plain `refresh: incremental` -> `refresh: full`
    // edit hit: `derive_model_maintenance_plan` returns `None` before any
    // refusal is constructed, so old cells/new refusals were both empty and
    // nothing downgraded.
    if !old_cells.is_empty() && new_cells.is_empty() {
        kinds.push(ChangeKind::MaintenanceLost);
    } else if old_cells.is_empty() && !new_cells.is_empty() {
        kinds.push(ChangeKind::MaintenanceGained);
    }
    kinds.extend(diff_refusals(old_refusals, new_refusals));
    kinds.extend(diff_probes(old_probes, new_probes));
    kinds.into_iter().map(Change::from_kind).collect()
}

/// Every change for a model that is `added`/`removed` in its entirety: one
/// change per profile field, with `old = null` (added) or `new = null`
/// (removed) — `docs/specs/property_diff.md` §"The diff".
fn whole_model_changes(profile: &PropertyProfile, added: bool) -> Vec<Change> {
    // Reuse the field-by-field diff against an "empty" profile so every
    // field still gets its own dimension, then null out the absent side.
    let empty = PropertyProfile {
        properties: PropertySet {
            columns: Vec::new(),
            grain: Grain::unkeyed(),
            functional_dependencies: Vec::new(),
            determinism: Vec::new(),
            comparability: Vec::new(),
            discriminants: Vec::new(),
            literal_columns: Vec::new(),
            has_set_op_barrier: false,
            has_fan_out_join: false,
            row_identity: RowIdentityVerdict {
                identity: RowIdentity::WholeRow,
                proven_mismatch: None,
            },
            source_bounds: BTreeMap::new(),
        },
        cell_verdicts: Vec::new(),
        refusals: Vec::new(),
        probes: Vec::new(),
    };
    let mut changes = if added {
        diff_profile(&empty, profile)
    } else {
        diff_profile(profile, &empty)
    };
    for c in &mut changes {
        if added {
            c.old = None;
        } else {
            c.new = None;
        }
        // G6 (`docs/outcomes/20260905-property-diff` fix round 1):
        // per-dimension directions are noise for a model that is wholly
        // added or removed — the `cause` (`added`/`removed`) already says
        // so, and grading e.g. a new model's every `refusal_added` as
        // `Downgrade` and a deleted model's every `refusal_removed` as
        // `Upgrade` invents a signal the summary counts should not carry.
        c.direction = Direction::Neutral;
    }
    changes
}

/// The pure diff (`docs/specs/property_diff.md` §"The diff", §Constraints
/// item 2 "Diff purity"): no I/O, no ledger, no backend, no git.
pub fn diff_profiles(
    old: &BTreeMap<String, PropertyProfile>,
    new: &BTreeMap<String, PropertyProfile>,
    graph: &DiffGraph,
) -> PropertyDiff {
    let mut model_diffs: Vec<ModelDiff> = Vec::new();
    let all_names: BTreeSet<&String> = old.keys().chain(new.keys()).collect();

    for name in &all_names {
        let (changes, cause_kind) = match (old.get(*name), new.get(*name)) {
            (None, Some(new_profile)) => (whole_model_changes(new_profile, true), CauseKind::Added),
            (Some(old_profile), None) => {
                (whole_model_changes(old_profile, false), CauseKind::Removed)
            }
            (Some(old_profile), Some(new_profile)) => {
                if old_profile == new_profile {
                    continue;
                }
                let attributed = graph.attribute(name);
                let mut model_diff = ModelDiff {
                    model: (*name).clone(),
                    cause: attributed,
                    changes: diff_profile(old_profile, new_profile),
                    stories: Vec::new(),
                };
                model_diff.stories = crate::analysis::diff_stories::narrate(&model_diff);
                model_diffs.push(model_diff);
                continue;
            }
            (None, None) => continue,
        };
        let mut model_diff = ModelDiff {
            model: (*name).clone(),
            cause: Cause {
                kind: cause_kind,
                of: vec![],
                reason: None,
            },
            changes,
            stories: Vec::new(),
        };
        model_diff.stories = crate::analysis::diff_stories::narrate(&model_diff);
        model_diffs.push(model_diff);
    }

    // Order: the graph's topological order (upstream first), then name for
    // ties (`docs/specs/property_diff.md` §Surface "Text").
    let mut order_index: BTreeMap<String, usize> = BTreeMap::new();
    if let Ok(topo) = topological_order(graph) {
        for (i, n) in topo.into_iter().enumerate() {
            order_index.insert(n, i);
        }
    }
    model_diffs.sort_by(|a, b| {
        let ai = order_index.get(&a.model);
        let bi = order_index.get(&b.model);
        match (ai, bi) {
            (Some(ai), Some(bi)) => ai.cmp(bi).then_with(|| a.model.cmp(&b.model)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.model.cmp(&b.model),
        }
    });

    let mut summary = DiffSummary {
        shifted_models: model_diffs.len(),
        ..Default::default()
    };
    for m in &model_diffs {
        for c in &m.changes {
            match c.direction {
                Direction::Downgrade => summary.downgrades += 1,
                Direction::Upgrade => summary.upgrades += 1,
                Direction::Neutral => summary.neutral += 1,
            }
        }
    }

    PropertyDiff {
        models: model_diffs,
        summary,
    }
}

/// A simple upstream-first topological order over `graph.upstream`,
/// falling back to name order alone if the graph is cyclic (a cyclic graph
/// is already a `GraphError` upstream of this module — this just must not
/// hang, `docs/outcomes/20260905-property-diff/phases/03-plan.md` "Risks").
fn topological_order(graph: &DiffGraph) -> Result<Vec<String>, ()> {
    let mut all_names: BTreeSet<String> = graph.upstream.keys().cloned().collect();
    for ups in graph.upstream.values() {
        for u in ups {
            all_names.insert(u.clone());
        }
    }
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut in_progress: BTreeSet<String> = BTreeSet::new();
    let mut order: Vec<String> = Vec::new();

    fn visit(
        node: &str,
        graph: &DiffGraph,
        visited: &mut BTreeSet<String>,
        in_progress: &mut BTreeSet<String>,
        order: &mut Vec<String>,
    ) -> Result<(), ()> {
        if visited.contains(node) {
            return Ok(());
        }
        if !in_progress.insert(node.to_string()) {
            return Err(());
        }
        if let Some(ups) = graph.upstream.get(node) {
            for u in ups {
                visit(u, graph, visited, in_progress, order)?;
            }
        }
        in_progress.remove(node);
        visited.insert(node.to_string());
        order.push(node.to_string());
        Ok(())
    }

    for name in &all_names {
        visit(name, graph, &mut visited, &mut in_progress, &mut order)?;
    }
    Ok(order)
}
