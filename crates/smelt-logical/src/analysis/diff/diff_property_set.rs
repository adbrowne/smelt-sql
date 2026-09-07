use super::*;

/// Diff two [`PropertySet`]s, field by field, with no `..` rest pattern —
/// a field added later is a compile error here until it is given a
/// dimension and a direction rule (`docs/specs/property_diff.md`
/// §Constraints item 3, and closing the "shifted with an empty changes
/// array" hole in §"The diff").
pub(super) fn diff_property_set(old: &PropertySet, new: &PropertySet) -> Vec<ChangeKind> {
    let PropertySet {
        columns: old_columns,
        grain: old_grain,
        functional_dependencies: old_fds,
        determinism: old_determinism,
        comparability: old_comparability,
        discriminants: old_discriminants,
        literal_columns: old_literal_columns,
        has_set_op_barrier: old_set_op_barrier,
        has_fan_out_join: old_fan_out_join,
        row_identity: old_row_identity,
        source_bounds: old_source_bounds,
    } = old;
    let PropertySet {
        columns: new_columns,
        grain: new_grain,
        functional_dependencies: new_fds,
        determinism: new_determinism,
        comparability: new_comparability,
        discriminants: new_discriminants,
        literal_columns: new_literal_columns,
        has_set_op_barrier: new_set_op_barrier,
        has_fan_out_join: new_fan_out_join,
        row_identity: new_row_identity,
        source_bounds: new_source_bounds,
    } = new;

    let mut changes = Vec::new();

    if old_grain != new_grain {
        changes.push(ChangeKind::Grain {
            subject: String::new(),
            old: old_grain.clone(),
            new: new_grain.clone(),
        });
    }
    if old_row_identity != new_row_identity {
        changes.push(ChangeKind::RowIdentity {
            old: old_row_identity.clone(),
            new: new_row_identity.clone(),
        });
    }
    if old_set_op_barrier != new_set_op_barrier {
        changes.push(ChangeKind::SetOpBarrier {
            old: *old_set_op_barrier,
            new: *new_set_op_barrier,
        });
    }
    if old_fan_out_join != new_fan_out_join {
        changes.push(ChangeKind::FanOutJoin {
            old: *old_fan_out_join,
            new: *new_fan_out_join,
        });
    }

    // Columns: matched on name (renames are removal + addition, spec
    // "Renames are not detected").
    let old_col_set: BTreeSet<&String> = old_columns.iter().collect();
    let new_col_set: BTreeSet<&String> = new_columns.iter().collect();
    for c in old_col_set.difference(&new_col_set) {
        changes.push(ChangeKind::ColumnRemoved((*c).clone()));
    }
    for c in new_col_set.difference(&old_col_set) {
        changes.push(ChangeKind::ColumnAdded((*c).clone()));
    }

    // Source bounds: matched on source name.
    let old_sources: BTreeSet<&String> = old_source_bounds.keys().collect();
    let new_sources: BTreeSet<&String> = new_source_bounds.keys().collect();
    for s in old_sources.union(&new_sources) {
        let (Some(o), Some(n)) = (old_source_bounds.get(*s), new_source_bounds.get(*s)) else {
            // A source bound present on only one side is a `source_bound`
            // change too (bound derivation appearing/disappearing entirely
            // is itself worth reporting), matched here as widened/
            // narrowed via the same rank comparison against a synthetic
            // absent-side default of `NotDerivable`.
            let default = BoundResult::NotDerivable;
            let old = old_source_bounds
                .get(*s)
                .cloned()
                .unwrap_or(default.clone());
            let new = new_source_bounds.get(*s).cloned().unwrap_or(default);
            if old != new {
                changes.push(ChangeKind::SourceBound {
                    source: (*s).clone(),
                    old,
                    new,
                });
            }
            continue;
        };
        if o != n {
            changes.push(ChangeKind::SourceBound {
                source: (*s).clone(),
                old: o.clone(),
                new: n.clone(),
            });
        }
    }

    // Per-column determinism/comparability/discriminants: matched on
    // column name, over the UNION of both sides' keys (G3,
    // `docs/outcomes/20260905-property-diff` fix round 1) — a column
    // present in one map and absent from the other, with `columns`
    // otherwise unchanged, must still surface as a change, mirroring
    // `literal_columns`'s own union-based diff just below.
    let old_det: BTreeMap<&String, &Det> = old_determinism
        .iter()
        .map(|d| (&d.output, &d.level))
        .collect();
    let new_det: BTreeMap<&String, &Det> = new_determinism
        .iter()
        .map(|d| (&d.output, &d.level))
        .collect();
    let all_det_cols: BTreeSet<&String> = old_det.keys().chain(new_det.keys()).copied().collect();
    for col in all_det_cols {
        let o = old_det.get(col).copied();
        let n = new_det.get(col).copied();
        if o != n {
            changes.push(ChangeKind::Determinism {
                column: col.clone(),
                old: o.copied(),
                new: n.copied(),
            });
        }
    }

    let old_comp: BTreeMap<&String, &Comp> = old_comparability
        .iter()
        .map(|c| (&c.output, &c.comparability))
        .collect();
    let new_comp: BTreeMap<&String, &Comp> = new_comparability
        .iter()
        .map(|c| (&c.output, &c.comparability))
        .collect();
    let all_comp_cols: BTreeSet<&String> =
        old_comp.keys().chain(new_comp.keys()).copied().collect();
    for col in all_comp_cols {
        let o = old_comp.get(col).copied();
        let n = new_comp.get(col).copied();
        if o != n {
            changes.push(ChangeKind::Comparability {
                column: col.clone(),
                old: o.copied(),
                new: n.copied(),
            });
        }
    }

    let old_disc: BTreeMap<&String, &crate::analysis::discriminants::Discriminants> =
        old_discriminants
            .iter()
            .map(|d| (&d.output, &d.discriminants))
            .collect();
    let new_disc: BTreeMap<&String, &crate::analysis::discriminants::Discriminants> =
        new_discriminants
            .iter()
            .map(|d| (&d.output, &d.discriminants))
            .collect();
    let all_disc_cols: BTreeSet<&String> =
        old_disc.keys().chain(new_disc.keys()).copied().collect();
    for col in all_disc_cols {
        let o = old_disc.get(col).copied();
        let n = new_disc.get(col).copied();
        if o != n {
            changes.push(ChangeKind::Discriminant {
                column: col.clone(),
                old: o.copied(),
                new: n.copied(),
            });
        }
    }

    // Functional dependencies: matched on (key, determines) as a whole
    // tuple (an FD is identified by its full shape, not a separate name).
    // `multiset_excess` (G7, `docs/outcomes/20260905-property-diff` fix
    // round 1) counts occurrences rather than membership — plain `.contains`
    // would silently drop a duplicate FD's removal (two copies of the same
    // FD on the old side, one on the new side, is a real removal `.contains`
    // cannot see since the value is still present).
    for fd in multiset_excess(old_fds, new_fds) {
        changes.push(ChangeKind::FdRemoved(fd));
    }
    for fd in multiset_excess(new_fds, old_fds) {
        changes.push(ChangeKind::FdAdded(fd));
    }

    // Literal columns: matched on column name.
    let old_lit: BTreeMap<&String, &String> =
        old_literal_columns.iter().map(|(k, v)| (k, v)).collect();
    let new_lit: BTreeMap<&String, &String> =
        new_literal_columns.iter().map(|(k, v)| (k, v)).collect();
    let all_lit_cols: BTreeSet<&String> = old_lit.keys().chain(new_lit.keys()).copied().collect();
    for col in all_lit_cols {
        let o = old_lit.get(col).map(|s| (*s).clone());
        let n = new_lit.get(col).map(|s| (*s).clone());
        if o != n {
            changes.push(ChangeKind::LiteralColumn {
                column: col.clone(),
                old: o,
                new: n,
            });
        }
    }

    changes
}
