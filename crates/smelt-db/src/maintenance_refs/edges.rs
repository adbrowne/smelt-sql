use smelt_core::metadata::{extract_file_metadata, FileMetadata};

use crate::*;

use super::refs::ref_source_info;

/// Extract the addressed section's own SQL body (frontmatter stripped) and
/// [`smelt_core::metadata::ModelMetadata`] from a model file's full `text`,
/// for either a single-model file (`leaf` unused) or a multi-model file
/// (matched by declared `name:`). `None` for a generator file — its
/// maintenance metadata lives on the emitted model, not the generator
/// file's own frontmatter (not exercised by any current maintained-upstream
/// fixture; resolving it is deferred), or a file with no frontmatter.
pub(super) fn resolved_model_sql_and_meta(
    text: &str,
    leaf: &str,
) -> Option<(String, smelt_core::metadata::ModelMetadata)> {
    match extract_file_metadata(text) {
        Ok(FileMetadata::Single {
            metadata,
            sql_offset,
        }) => Some((text[sql_offset..].to_string(), *metadata)),
        Ok(FileMetadata::Multi { models }) => {
            let section = models
                .into_iter()
                .find(|s| s.metadata.name.as_deref() == Some(leaf))?;
            Some((
                text[section.sql_range.clone()].to_string(),
                section.metadata,
            ))
        }
        _ => None,
    }
}

/// This model's own `smelt.sources.*` refs as [`output_delta::SourceFacts`]
/// — the per-model input the output-delta walk reads. Mirrors
/// `smelt-runtime::propagation::model_output_delta_sources`'s declared-source
/// collection over a `ModelFile`'s own `refs`, rebuilt here from `sql` text
/// via [`smelt_logical::collect_path_refs`] since the Salsa side has no
/// eagerly-loaded `ModelFile::refs` at this call site.
fn model_own_source_facts(
    db: &dyn salsa::Database,
    workspace: Workspace,
    project: Option<ProjectInput>,
    sql: &str,
) -> Vec<smelt_logical::analysis::output_delta::SourceFacts> {
    let mut sources = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for r in smelt_logical::collect_path_refs(sql) {
        let Some(stripped) = r.strip_prefix("smelt.") else {
            continue;
        };
        let Some(bare) = stripped.strip_prefix("sources.") else {
            continue;
        };
        if !seen.insert(bare.to_string()) {
            continue;
        }
        if let Some(info) = ref_source_info(db, workspace, project, &r) {
            sources.push(
                smelt_logical::analysis::output_delta::SourceFacts::from_source_info(bare, &info),
            );
        }
    }
    sources
}

/// Assemble the per-model [`smelt_logical::analysis::output_delta::
/// ModelDeltaInput`] records for the cross-model output-delta fold
/// (`derive_workspace_output_deltas`), scoped to every model transitively
/// reachable from `file`'s own refs — mirrors `smelt-runtime::propagation::
/// workspace_output_delta_verdicts`'s per-model input shape, but built by
/// walking refs rather than over an eagerly-loaded `&[ModelFile]` (`smelt-db`
/// has no such list at this call site). `address` is the ref's own
/// `smelt.`-stripped path, lowercased — the SAME key
/// [`smelt_logical::analysis::output_delta::derive_workspace_output_deltas`]
/// inserts into its verdict map, so a model-reference leaf inside any
/// reached model's own SQL resolves against it. Deduplicated by that address
/// (not by `SourceFile`), which is what makes a cyclic model-ref graph
/// terminate: each distinct address is queued at most once, so the walk is
/// bounded by the number of distinct reachable addresses regardless of how
/// many cycles connect them — never a per-model-reference recursive Salsa
/// query (`CLAUDE.md` §"Salsa purity rule"), which could not terminate over
/// a cycle.
fn model_delta_inputs(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<smelt_logical::analysis::output_delta::ModelDeltaInput> {
    let mut inputs = Vec::new();
    let mut visited: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut frontier: Vec<SourceFile> = vec![file];
    while let Some(f) = frontier.pop() {
        let text = f.text(db);
        for r in smelt_logical::collect_path_refs(text) {
            let Some(stripped) = r.strip_prefix("smelt.") else {
                continue;
            };
            let address = stripped.to_ascii_lowercase();
            if !visited.insert(address.clone()) {
                continue;
            }
            let segments: Vec<String> = stripped.split('.').map(|s| s.to_string()).collect();
            let Some(leaf) = segments.last().cloned() else {
                continue;
            };
            let Some(resolved) = resolve_ref_path(db, workspace, segments) else {
                continue;
            };
            if resolved.kind != RefKind::Model {
                continue;
            }
            let Some(model_file) = resolved.source_file else {
                continue;
            };
            let model_text = model_file.text(db);
            let Some((sql, _meta)) = resolved_model_sql_and_meta(model_text, &leaf) else {
                continue;
            };
            let project = find_project(db, workspace, model_file.project_root(db));
            let sources = model_own_source_facts(db, workspace, project, &sql);
            inputs.push(smelt_logical::analysis::output_delta::ModelDeltaInput {
                address,
                sql,
                ctx: smelt_logical::analysis::join_shape::JoinContext::new(),
                sources,
            });
            frontier.push(model_file);
        }
    }
    inputs
}

/// Every upstream maintained-model edge for `file` (`incremental_models.md`
/// §"Upstream model edges"): the model refs `file`'s own SQL makes that
/// resolve to another maintained model in this project, each carrying that
/// upstream's own validated clock and derived output-delta shape
/// (`ModelEdge::output_shape`). Its own entry point (not only inlined within
/// [`crate::maintenance_refs::maintenance_plan_report`]) so `smelt explain`'s
/// plan report and a direct caller — a test pinning the Salsa-side derivation
/// itself — read the SAME edges rather than two independently-assembled
/// lists. `file` with no frontmatter or no `Single`-model metadata
/// contributes no edges.
pub fn model_edges_for(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Vec<smelt_logical::maintenance::derive::ModelEdge> {
    let text = file.text(db);
    let Ok(FileMetadata::Single { sql_offset, .. }) = extract_file_metadata(text) else {
        return Vec::new();
    };
    let sql_body = &text[sql_offset..];
    let refs = smelt_logical::collect_path_refs(sql_body);
    // The cross-model output-delta verdict map is folded ONCE per call (not
    // once per ref) over every model transitively reachable from `file`'s
    // own refs, then threaded into every `ref_model_edge` call so a
    // model-reference leaf inside any upstream's own SQL resolves against
    // it (`docs/outcomes/20260809-output-delta-typing/outcome.md` phase 9).
    let model_verdicts = smelt_logical::analysis::output_delta::derive_workspace_output_deltas(
        &model_delta_inputs(db, workspace, file),
    );
    refs.iter()
        .filter_map(|r| ref_model_edge(db, workspace, r, &model_verdicts))
        .collect()
}

fn ref_model_edge(
    db: &dyn salsa::Database,
    workspace: Workspace,
    ref_str: &str,
    model_verdicts: &std::collections::BTreeMap<
        String,
        smelt_logical::analysis::output_delta::OutputDeltaFacts,
    >,
) -> Option<smelt_logical::maintenance::derive::ModelEdge> {
    let stripped = ref_str.strip_prefix("smelt.")?;
    let segments: Vec<String> = stripped.split('.').map(|s| s.to_string()).collect();
    let leaf = segments.last()?.clone();
    let resolved = resolve_ref_path(db, workspace, segments.clone())?;
    if resolved.kind != RefKind::Model {
        return None;
    }
    let file = resolved.source_file?;
    let text = file.text(db);
    // Extract the addressed model's own `refresh:`/`timeseries:` plus its
    // own SQL body — the latter feeds `output_shape` below.
    let (sql, meta) = resolved_model_sql_and_meta(text, &leaf)?;
    // Only a maintained (`refresh: incremental`) upstream delivers an
    // incremental delta to receive; a `full`-mode or view upstream is
    // excluded (no creation cell, no refusal).
    if meta.refresh != Some(smelt_core::config::RefreshStrategy::Incremental) {
        return None;
    }
    let clock_col = meta.timeseries.as_ref().map(|t| t.partition_column.clone());
    // Sibling spellings of `clock_col` within the upstream's own SQL
    // (`ModelEdge::clock_col_aliases`'s doc comment) — derived from the same
    // `text` the metadata above was extracted from.
    let clock_col_aliases = clock_col
        .as_deref()
        .map(|c| smelt_logical::analysis::source_bounds::defining_expr_siblings(text, c))
        .unwrap_or_default();
    // The upstream's own declared top-level `unique_key:` (`models.md`
    // §"The Relation Contract"), threaded through so a downstream's P1
    // skeleton-source-closure proof over this edge can prove the join
    // one-to-one (T3, `docs/plans/20260715-composed-axes-conditional-
    // maintenance.md` Phase E3) — `ModelEdge::unique_key`'s doc comment.
    let unique_key = meta.unique_key.clone().unwrap_or_default();
    // The upstream's own derived output-delta shape (`ModelEdge::
    // output_shape`'s doc comment): the meet across whatever per-column-group
    // verdicts this upstream's own SQL derives — the SAME per-workspace fold
    // `smelt-runtime::propagation::upstream_output_delta_groups` computes,
    // never re-implemented differently here. `None` when the upstream
    // contributes no groups at all (e.g. an unclassifiable `SELECT *`
    // projection) rather than an optimistic guess.
    let project = find_project(db, workspace, file.project_root(db));
    let sources = model_own_source_facts(db, workspace, project, &sql);
    let declared_unique_key = meta.unique_key.clone().unwrap_or_default();
    let partition_col = meta.timeseries.as_ref().map(|t| t.partition_column.clone());
    let output_shape = own_output_delta_shape(
        &sql,
        &declared_unique_key,
        partition_col.as_deref(),
        &sources,
        model_verdicts,
    );
    Some(smelt_logical::maintenance::derive::ModelEdge {
        name: stripped.to_string(),
        clock_col,
        clock_col_aliases,
        unique_key,
        output_shape,
    })
}

/// A model's own derived output-delta shape: the meet across whatever
/// per-column-group verdicts its own SQL derives, given the cross-model
/// verdict map its own model-references should resolve against. Pure
/// (Salsa purity rule) — extracted out of [`ref_model_edge`] so
/// [`model_output_delta_for`] computes a model's own shape through the SAME
/// derivation a downstream's edge view of that model already uses; the two
/// call sites differ only in which model's SQL/sources/verdict-map they
/// pass in, never in what this function does with them.
fn own_output_delta_shape(
    sql: &str,
    unique_key: &[String],
    partition_col: Option<&str>,
    sources: &[smelt_logical::analysis::output_delta::SourceFacts],
    model_verdicts: &std::collections::BTreeMap<
        String,
        smelt_logical::analysis::output_delta::OutputDeltaFacts,
    >,
) -> Option<smelt_logical::analysis::output_delta::OutputDelta> {
    let skeleton =
        smelt_logical::maintenance::skeleton::skeleton_columns(sql, unique_key, partition_col);
    smelt_logical::analysis::output_delta::derive_output_delta_with_model_verdicts(
        sql,
        &smelt_logical::analysis::join_shape::JoinContext::new(),
        sources,
        &skeleton,
        model_verdicts,
    )
    .into_iter()
    .map(|(_, shape)| shape)
    .reduce(smelt_logical::analysis::output_delta::OutputDelta::meet)
}

/// This model's own emitted output-delta shape (`incremental_models.md`
/// §Surface "CLI" headline — the delta signature `smelt explain` prints
/// first): the SAME derivation [`ref_model_edge`] applies when some
/// downstream reports this model as an upstream edge, single-owned via
/// [`own_output_delta_shape`] so `smelt explain`'s own-model headline and a
/// downstream's edge view of this same model can never disagree
/// (`docs/outcomes/20260904-delta-signature-front-door/outcome.md` phase
/// 1). `None` for a generator/multi-model file (only a `Single`-model file
/// has one address to report a shape for), a file with no frontmatter, or a
/// model whose own SQL yields no output column groups.
pub fn model_output_delta_for(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Option<smelt_logical::analysis::output_delta::OutputDelta> {
    let text = file.text(db);
    let Ok(FileMetadata::Single {
        metadata,
        sql_offset,
    }) = extract_file_metadata(text)
    else {
        return None;
    };
    let sql = &text[sql_offset..];
    let unique_key = metadata.unique_key.clone().unwrap_or_default();
    let partition_col = metadata
        .timeseries
        .as_ref()
        .map(|t| t.partition_column.clone());
    let project = find_project(db, workspace, file.project_root(db));
    let sources = model_own_source_facts(db, workspace, project, sql);
    let model_verdicts = smelt_logical::analysis::output_delta::derive_workspace_output_deltas(
        &model_delta_inputs(db, workspace, file),
    );
    own_output_delta_shape(
        sql,
        &unique_key,
        partition_col.as_deref(),
        &sources,
        &model_verdicts,
    )
}
