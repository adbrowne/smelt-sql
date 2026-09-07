use smelt_core::metadata::{extract_file_metadata, FileMetadata};

use crate::*;

use super::edges::resolved_model_sql_and_meta;

/// Per-source clamp observability (`docs/specs/incremental_shapes.md`
/// §"Observing the per-source clamp"): `file`'s own [`BoundResult`] per
/// `smelt.<path>` source it references, for editor hover. Thin Salsa
/// wrapper (Salsa purity rule) over the pure
/// `smelt_logical::analysis::source_bounds::derive_model_bounds`: resolves
/// each of `file`'s own refs to the upstream's declared
/// `timeseries.partition_column` (+ sibling spellings), mirroring
/// `ref_model_edge`'s pattern, builds the `BoundContext`, and calls the
/// pure derivation over `file`'s own SQL. Returns an empty map when `file`'s
/// own model is not itself partition-grain (no `timeseries:` declared) or
/// references no bounded sources — hover has nothing to show either way.
pub fn model_source_clamps(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> std::collections::BTreeMap<String, smelt_logical::BoundResult> {
    let text = file.text(db);
    let Ok(FileMetadata::Single {
        metadata,
        sql_offset,
    }) = extract_file_metadata(text)
    else {
        return Default::default();
    };
    if metadata.timeseries.is_none() {
        return Default::default();
    }
    let sql = &text[sql_offset..];
    let mut ctx = smelt_logical::BoundContext::new();
    for r in smelt_logical::collect_path_refs(sql) {
        let Some(stripped) = r.strip_prefix("smelt.") else {
            continue;
        };
        let segments: Vec<String> = stripped.split('.').map(|s| s.to_string()).collect();
        let Some(leaf) = segments.last().cloned() else {
            continue;
        };
        let Some(resolved) = resolve_ref_path(db, workspace, segments.clone()) else {
            continue;
        };
        if resolved.kind != RefKind::Model {
            continue;
        }
        let Some(upstream_file) = resolved.source_file else {
            continue;
        };
        let upstream_text = upstream_file.text(db);
        let Some((upstream_sql, upstream_meta)) = resolved_model_sql_and_meta(upstream_text, &leaf)
        else {
            continue;
        };
        let Some(ts) = upstream_meta.timeseries.as_ref() else {
            continue;
        };
        ctx.add_source(stripped, &ts.partition_column);
        let aliases = smelt_logical::analysis::source_bounds::defining_expr_siblings(
            &upstream_sql,
            &ts.partition_column,
        );
        ctx.add_source_partition_col_aliases(stripped, aliases);
    }
    if ctx.source_partition_cols.is_empty() {
        return Default::default();
    }
    smelt_logical::analysis::source_bounds::derive_model_bounds(sql, &ctx)
        .into_iter()
        .collect()
}
