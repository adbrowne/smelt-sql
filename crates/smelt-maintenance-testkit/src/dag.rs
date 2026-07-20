//! Generated DAGs (`docs/plans/20260712-generative-maintenance-conformance.md`
//! Phase 10; design doc §9 "generated 2-3 node DAGs"). Extends the single-model
//! recipe/render/verdict machinery ([`crate::recipe`], [`crate::render`],
//! [`crate::verdict`]) to small generated graphs of models wired together via
//! `smelt.ref()`, so `crates/smelt-cli/tests/maintenance_conformance/dags.rs`
//! can assert forward-propagation sufficiency (`smelt_runtime::propagation::
//! plan_since_upstream`) and backward-resolution sufficiency
//! (`smelt_runtime::propagation::resolve_build_plan`) against a real
//! full-refresh oracle, over more than one model.
//!
//! [`DagRecipe`] self-renders (like [`crate::recipe::AdversarialLeafRecipe`]/
//! [`crate::recipe::MutableEnrichedRecipe`]) rather than routing through
//! [`crate::render`]'s exhaustive [`crate::recipe::BodyConstruct`] match: a
//! DAG node's body references an upstream MODEL (`smelt.<node>`) as often as
//! it references a raw source (`smelt.sources.<name>`), which is outside
//! `render.rs`'s single-source contract and outside this phase's edit scope
//! (plan Critical files: `dag.rs` new, `render.rs`/`recipe.rs` untouched).
//!
//! Every node is a `grain: partition` model over the SAME clocked
//! `events(d, id, val)` shape [`crate::recipe::SourceRecipe::events`] renders
//! elsewhere in this crate, except [`NodeGrain::Key`] — reachable only via
//! [`keyed_sink_dag`], which exists solely to pin the keyed-exclusion
//! assertion (Phase 10 review checklist: "keyed-grain nodes excluded from
//! generated graphs").

#![allow(dead_code)]

use std::path::Path;

use duckdb::Connection;

use crate::link_c_harness::LinkCProject;
use crate::recipe::SourceRecipe;
use crate::render::render_smelt_yml;
use crate::verdict::Verdict;

/// Where a [`DagNode`] reads from: the graph's one raw source, or another
/// node by index into [`DagRecipe::nodes`] (always a lower index — nodes are
/// declared in topological order, enforced by construction in every
/// constructor this module ships).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upstream {
    Source,
    Node(usize),
}

/// A DAG node's `SELECT` body shape. Deliberately a small, fixed pool (not a
/// generated construct axis like [`crate::recipe::BodyConstruct`] — Phase
/// 10's generative surface is the SCHEDULE/deltas, not the body shape; see
/// the module doc comment and the plan's Phase 10 "Implementation shape").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DagBody {
    /// `SELECT d, id, val FROM <upstream>` — row-for-row passthrough.
    PassThrough,
    /// Passthrough restricted to one parity of `id` — used to partition the
    /// source's row space into two disjoint branches for a diamond's fan-out
    /// arms (`dag_diamond_a`/`dag_diamond_b`), so the confluence node's
    /// `UNION ALL` never double-counts a row.
    ParityFilter { parity: i64 },
    /// `UNION ALL` of every upstream (≥ 2) — a diamond's confluence node.
    Union,
    /// `SELECT d, SUM(val) AS total FROM <upstream> GROUP BY d` — a
    /// partition-grain additive aggregate sink.
    AdditiveAgg,
    /// `SELECT id, SUM(val) AS total FROM <upstream> GROUP BY id` — the
    /// `grain: key` sink [`keyed_sink_dag`] uses for the keyed-exclusion
    /// assertion.
    KeyedAgg,
    /// The payload-leak family (Phase 10 TDD list
    /// `upstream_payload_in_downstream_skeleton_position`): the upstream's
    /// own payload column (`val`) occupies a skeleton/identity position
    /// (`GROUP BY`) in the downstream body, rather than staying a payload.
    /// `SELECT d, val, COUNT(*) AS cnt FROM <upstream> GROUP BY d, val`.
    GroupByPayload,
}

/// A node's declared output grain. [`NodeGrain::Partition`] is every node in
/// [`chain_dag`]/[`diamond_dag`]/[`leak_dag`]; [`NodeGrain::Key`] exists only
/// in [`keyed_sink_dag`].
#[derive(Debug, Clone)]
pub enum NodeGrain {
    /// `refresh: incremental` + `grain: partition`, with the given declared
    /// `batched.unique_key`.
    Partition { unique_key: Vec<String> },
    /// `refresh: incremental` + `grain: key` — no `timeseries:` block, no
    /// `unique_key` (`incremental_models.md` §Known Divergences "The key grain": "every
    /// `timeseries:` block on a keyed model is refused unconditionally").
    Key,
}

/// One node in a [`DagRecipe`]: a model name, its upstream ref(s), body
/// shape, and declared grain.
#[derive(Debug, Clone)]
pub struct DagNode {
    pub name: String,
    pub upstreams: Vec<Upstream>,
    pub body: DagBody,
    pub grain: NodeGrain,
}

/// A generated graph: one raw clocked `events(d, id, val)` source
/// ([`crate::recipe::SourceRecipe::events`]) feeding N models
/// ([`DagNode`]s), declared in topological order (`nodes[i]` may only read
/// `nodes[j]` for `j < i`, or the source).
#[derive(Debug, Clone)]
pub struct DagRecipe {
    pub source: SourceRecipe,
    pub nodes: Vec<DagNode>,
}

impl DagRecipe {
    /// The topological execution order (model names) `plan_since_upstream`/
    /// `resolve_build_plan` expect as their `order` parameter.
    pub fn order(&self) -> Vec<String> {
        self.nodes.iter().map(|n| n.name.clone()).collect()
    }
}

/// The two-node chain: `dag_chain_a` (passthrough over the source) ->
/// `dag_chain_b` (additive-agg sink over `dag_chain_a`) — Phase 10 TDD list
/// `chain_since_upstream_dirty_set_suffices`.
pub fn chain_dag() -> DagRecipe {
    DagRecipe {
        source: SourceRecipe::events(crate::recipe::KeyShape::Single),
        nodes: vec![
            DagNode {
                name: "dag_chain_a".to_string(),
                upstreams: vec![Upstream::Source],
                body: DagBody::PassThrough,
                grain: NodeGrain::Partition {
                    unique_key: vec!["id".to_string()],
                },
            },
            DagNode {
                name: "dag_chain_b".to_string(),
                upstreams: vec![Upstream::Node(0)],
                body: DagBody::AdditiveAgg,
                grain: NodeGrain::Partition {
                    unique_key: vec!["d".to_string()],
                },
            },
        ],
    }
}

/// The three-node diamond: `dag_diamond_a`/`dag_diamond_b` (disjoint-parity
/// passthrough branches over the source) confluencing into `dag_diamond_c`
/// (`UNION ALL` of both) — Phase 10 TDD list `diamond_propagation_suffices`.
pub fn diamond_dag() -> DagRecipe {
    DagRecipe {
        source: SourceRecipe::events(crate::recipe::KeyShape::Single),
        nodes: vec![
            DagNode {
                name: "dag_diamond_a".to_string(),
                upstreams: vec![Upstream::Source],
                body: DagBody::ParityFilter { parity: 0 },
                grain: NodeGrain::Partition {
                    unique_key: vec!["id".to_string()],
                },
            },
            DagNode {
                name: "dag_diamond_b".to_string(),
                upstreams: vec![Upstream::Source],
                body: DagBody::ParityFilter { parity: 1 },
                grain: NodeGrain::Partition {
                    unique_key: vec!["id".to_string()],
                },
            },
            DagNode {
                name: "dag_diamond_c".to_string(),
                upstreams: vec![Upstream::Node(0), Upstream::Node(1)],
                body: DagBody::Union,
                grain: NodeGrain::Partition {
                    unique_key: vec!["id".to_string()],
                },
            },
        ],
    }
}

/// The payload-leak pair: `dag_leak_a` (passthrough over the source) ->
/// `dag_leak_b`, which groups by `dag_leak_a`'s own payload column `val` (a
/// skeleton-position leak) — Phase 10 TDD list
/// `upstream_payload_in_downstream_skeleton_position`.
pub fn leak_dag() -> DagRecipe {
    DagRecipe {
        source: SourceRecipe::events(crate::recipe::KeyShape::Single),
        nodes: vec![
            DagNode {
                name: "dag_leak_a".to_string(),
                upstreams: vec![Upstream::Source],
                body: DagBody::PassThrough,
                grain: NodeGrain::Partition {
                    unique_key: vec!["id".to_string()],
                },
            },
            DagNode {
                name: "dag_leak_b".to_string(),
                upstreams: vec![Upstream::Node(0)],
                body: DagBody::GroupByPayload,
                grain: NodeGrain::Partition {
                    unique_key: vec!["d".to_string(), "val".to_string()],
                },
            },
        ],
    }
}

/// A chain whose SINK is `grain: key` (no `timeseries:`) — exists solely for
/// the keyed-exclusion assertion (Phase 10 review checklist): a generated
/// graph containing a keyed node must never derive a propagation edge into
/// it (`smelt_runtime::propagation::build_forward_graph`'s own contract,
/// pinned over a hand-typed fixture by
/// `crates/smelt-runtime/tests/since_upstream_propagation.rs::
/// keyed_grain_model_never_derives_an_edge`; this pins the SAME contract
/// over a *generated* graph).
pub fn keyed_sink_dag() -> DagRecipe {
    DagRecipe {
        source: SourceRecipe::events(crate::recipe::KeyShape::Single),
        nodes: vec![
            DagNode {
                name: "dag_keyed_a".to_string(),
                upstreams: vec![Upstream::Source],
                body: DagBody::PassThrough,
                grain: NodeGrain::Partition {
                    unique_key: vec!["id".to_string()],
                },
            },
            DagNode {
                name: "dag_keyed_sink".to_string(),
                upstreams: vec![Upstream::Node(0)],
                body: DagBody::KeyedAgg,
                grain: NodeGrain::Key,
            },
        ],
    }
}

fn upstream_ref(dag: &DagRecipe, up: Upstream) -> String {
    match up {
        Upstream::Source => format!("smelt.sources.{}", dag.source.name),
        Upstream::Node(i) => format!("smelt.{}", dag.nodes[i].name),
    }
}

/// The declared output columns for `node`'s body — known by construction
/// (this module wrote the SQL), used both by [`render_node_body`] and by
/// [`fetch_node_multiset`] to build a column-exact comparison query without
/// any runtime schema introspection.
fn node_output_columns(dag: &DagRecipe, node: &DagNode) -> Vec<String> {
    let d = dag.source.clock_column.clone();
    let id = dag.source.key_column.clone();
    let val = dag.source.payload_column.clone();
    match node.body {
        DagBody::PassThrough | DagBody::ParityFilter { .. } | DagBody::Union => vec![d, id, val],
        DagBody::AdditiveAgg => vec![d, "total".to_string()],
        DagBody::KeyedAgg => vec![id, "total".to_string()],
        DagBody::GroupByPayload => vec![d, val, "cnt".to_string()],
    }
}

/// The model's `SELECT` body for `dag.nodes[idx]` — "renders once, serves
/// the model file" (this module has no separate oracle-query path: a DAG's
/// oracle IS a from-scratch full-refresh build over an independently-staged
/// twin of the same [`DagRecipe`], never a source-substituted query, since a
/// node's own upstream may be another model rather than a raw source).
pub fn render_node_body(dag: &DagRecipe, idx: usize) -> String {
    let node = &dag.nodes[idx];
    let d = &dag.source.clock_column;
    let id = &dag.source.key_column;
    let val = &dag.source.payload_column;
    match node.body {
        DagBody::PassThrough => {
            let src = upstream_ref(dag, node.upstreams[0]);
            format!("SELECT {d}, {id}, {val} FROM {src}")
        }
        DagBody::ParityFilter { parity } => {
            let src = upstream_ref(dag, node.upstreams[0]);
            format!("SELECT {d}, {id}, {val} FROM {src} WHERE {id} % 2 = {parity}")
        }
        DagBody::Union => {
            let parts: Vec<String> = node
                .upstreams
                .iter()
                .map(|&u| format!("SELECT {d}, {id}, {val} FROM {}", upstream_ref(dag, u)))
                .collect();
            parts.join(" UNION ALL ")
        }
        DagBody::AdditiveAgg => {
            let src = upstream_ref(dag, node.upstreams[0]);
            format!("SELECT {d}, SUM({val}) AS total FROM {src} GROUP BY {d}")
        }
        DagBody::KeyedAgg => {
            let src = upstream_ref(dag, node.upstreams[0]);
            format!("SELECT {id}, SUM({val}) AS total FROM {src} GROUP BY {id}")
        }
        DagBody::GroupByPayload => {
            let src = upstream_ref(dag, node.upstreams[0]);
            format!("SELECT {d}, {val}, COUNT(*) AS cnt FROM {src} GROUP BY {d}, {val}")
        }
    }
}

/// The full model file for `dag.nodes[idx]`: frontmatter (per [`NodeGrain`])
/// followed by [`render_node_body`].
pub fn render_node_file(dag: &DagRecipe, idx: usize) -> String {
    let node = &dag.nodes[idx];
    let body = render_node_body(dag, idx);
    match &node.grain {
        NodeGrain::Partition { unique_key: _ } => {
            // The retired `batched.unique_key` sub-block this used to carry
            // `unique_key` under is gone — it never fed row-identity
            // derivation for a `Grain::Partition` output anyway
            // (`derive::ModelInputs::declared_unique_key` is empty for
            // every `Grain::Partition`), so dropping it changes no derived
            // maintenance plan.
            let d = &dag.source.clock_column;
            format!(
                "---\ntimeseries:\n  event_time_column: {d}\n  partition_column: {d}\n  granularity: day\nrefresh: incremental\ngrain: partition\n---\n{body}\n"
            )
        }
        NodeGrain::Key => {
            format!("---\nrefresh: incremental\ngrain: key\n---\n{body}\n")
        }
    }
}

/// Stage `dag` into a fresh project dir + DuckDB file: writes every node's
/// model file, the driving source's YAML sidecar, `smelt.yml`, and creates
/// the empty physical source table — the DAG-pool counterpart of
/// [`crate::render::stage`].
pub fn stage_dag(
    dag: &DagRecipe,
    project_dir: &Path,
    db_path: &Path,
) -> anyhow::Result<LinkCProject> {
    std::fs::create_dir_all(project_dir.join("models/sources"))?;
    for idx in 0..dag.nodes.len() {
        std::fs::write(
            project_dir.join(format!("models/{}.sql", dag.nodes[idx].name)),
            render_node_file(dag, idx),
        )?;
    }
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", dag.source.name)),
        dag.source.source_yaml(),
    )?;
    std::fs::write(project_dir.join("smelt.yml"), render_smelt_yml(db_path))?;

    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch(&format!(
        "CREATE SCHEMA IF NOT EXISTS main; \
         CREATE TABLE main.sources_{name} ({d} DATE, {id} INTEGER, {val} INTEGER);",
        name = dag.source.name,
        d = dag.source.clock_column,
        id = dag.source.key_column,
        val = dag.source.payload_column,
    ))?;
    drop(conn);

    LinkCProject::load(project_dir.to_path_buf(), db_path.to_path_buf())
}

/// Insert `(d, id, val)` rows into `dag`'s physical source table — one
/// batched `INSERT`, literal-embedded (mirrors `gate.rs::insert_row`'s
/// per-row convention, batched since DAG cases insert whole generated
/// windows at once). A no-op for an empty `rows` (a pure catch-up
/// propagation with no new data).
pub fn insert_rows(
    conn: &Connection,
    dag: &DagRecipe,
    rows: &[(chrono::NaiveDate, i64, i64)],
) -> anyhow::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let values: Vec<String> = rows
        .iter()
        .map(|(d, id, val)| format!("(DATE '{}', {id}, {val})", d.format("%Y-%m-%d")))
        .collect();
    conn.execute_batch(&format!(
        "INSERT INTO main.sources_{} VALUES {}",
        dag.source.name,
        values.join(", ")
    ))?;
    Ok(())
}

/// Read back `dag.nodes[idx]`'s materialized table as a sorted multiset of
/// stringified rows, using [`node_output_columns`]'s known-by-construction
/// column list (no runtime schema introspection) — the comparison primitive
/// every Phase 10 test diffs an incrementally-propagated project against an
/// independently-staged full-refresh twin with, restricted to `where_clause`
/// when the caller only cares about one period (`Some("d = DATE '...'")`),
/// unrestricted (`None`) when comparing the node's entire materialized
/// state.
pub fn fetch_node_multiset(
    conn: &Connection,
    dag: &DagRecipe,
    idx: usize,
    where_clause: Option<&str>,
) -> Vec<Vec<String>> {
    let node = &dag.nodes[idx];
    let cols = node_output_columns(dag, node);
    let cast_list = cols
        .iter()
        .map(|c| format!("CAST({c} AS VARCHAR)"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = match where_clause {
        Some(w) => format!("SELECT {cast_list} FROM main.{} WHERE {w}", node.name),
        None => format!("SELECT {cast_list} FROM main.{}", node.name),
    };
    let mut stmt = conn.prepare(&sql).expect("prepare fetch_node_multiset");
    let n = cols.len();
    let mut rows: Vec<Vec<String>> = stmt
        .query_map([], move |row| {
            let mut v = Vec::with_capacity(n);
            for i in 0..n {
                let cell: Option<String> = row.get(i)?;
                v.push(cell.unwrap_or_else(|| "NULL".to_string()));
            }
            Ok(v)
        })
        .expect("query_map fetch_node_multiset")
        .map(|r| r.expect("row fetch_node_multiset"))
        .collect();
    rows.sort();
    rows
}

/// Classify `dag`'s node `model_name` through the REAL maintenance
/// derivation, mirroring [`crate::verdict::classify`]'s contract exactly
/// (same `smelt_db::maintenance_plan_report` + `smelt_db::file_diagnostics`
/// production entry points, same fail-loud "refusal needs a named
/// diagnostic" check) — duplicated here rather than imported because
/// `verdict::classify` takes a [`crate::recipe::ModelRecipe`] specifically,
/// and a DAG node has no such value (`verdict.rs` is outside this phase's
/// edit scope per the plan's Critical files, the same reason
/// [`crate::recipe::AdversarialLeafRecipe`] self-renders instead of reusing
/// `render.rs`).
pub fn classify_node(project: &LinkCProject, model_name: &str) -> anyhow::Result<Verdict> {
    let config = smelt_core::config::Config::load(&project.project_dir)?;
    let discovery =
        smelt_core::ModelDiscovery::new(project.project_dir.clone(), config.paths.clone());
    let sql_models = discovery.discover_models()?;
    let target_path = project.project_dir.join(format!("models/{model_name}.sql"));

    let mut db = smelt_db::Database::default();
    let proj_input = db.set_project_input(project.project_dir.clone(), String::new());
    let mut target: Option<smelt_db::SourceFile> = None;
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| {
            let file = db.set_source_file(
                m.path.clone(),
                m.content.clone(),
                project.project_dir.clone(),
            );
            if m.path == target_path {
                target = Some(file);
            }
            file
        })
        .collect();
    db.set_workspace(source_files, vec![proj_input]);
    let workspace = db.workspace();

    let target = target.ok_or_else(|| {
        anyhow::anyhow!(
            "staged DAG node {model_name:?} (expected at {}) not found among discovered models",
            target_path.display()
        )
    })?;

    let diagnostics = smelt_db::file_diagnostics(&db, workspace, target);
    let plan_result = smelt_db::maintenance_plan_report(&db, workspace, target);

    let named: Vec<smelt_db::Diagnostic> = diagnostics
        .iter()
        .filter(|d| {
            d.severity == smelt_db::DiagnosticSeverity::Error
                && matches!(
                    d.code,
                    Some(
                        smelt_db::DiagnosticCode::MaintenanceNoAdmissibleTechnique
                            | smelt_db::DiagnosticCode::MaintenanceScanUnbounded
                            | smelt_db::DiagnosticCode::MaintenanceGranularityMismatch
                    )
                )
        })
        .cloned()
        .collect();

    match plan_result {
        Some(r) if !r.plan.cells.is_empty() => Ok(Verdict::Admitted(r.plan)),
        _ => {
            if named.is_empty() {
                anyhow::bail!(
                    "DAG node {model_name:?} was refused (no admitted maintenance cell) but \
                     carries no named Maintenance*/admission diagnostic — fail-loud discipline \
                     violation (architecture.md §\"Fail-loud discipline\")"
                );
            }
            Ok(Verdict::Refused(named))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sanity: every constructor's nodes only ever reference the source or a
    /// STRICTLY earlier node — the topological-order invariant every test in
    /// `dags.rs` relies on (`DagRecipe::order()` is fed straight to
    /// `plan_since_upstream`/`resolve_build_plan` as the caller's
    /// topological execution order).
    #[test]
    fn every_shipped_dag_is_topologically_ordered() {
        for dag in [chain_dag(), diamond_dag(), leak_dag(), keyed_sink_dag()] {
            for (i, node) in dag.nodes.iter().enumerate() {
                for &up in &node.upstreams {
                    if let Upstream::Node(j) = up {
                        assert!(
                            j < i,
                            "node {i} ({:?}) references node {j}, not a strictly earlier node",
                            node.name
                        );
                    }
                }
            }
        }
    }

    /// `render_node_body`'s upstream substitution is exhaustive over every
    /// declared [`Upstream`] variant — a smoke check that every shipped DAG
    /// actually renders without panicking.
    #[test]
    fn every_shipped_dag_renders_every_node() {
        for dag in [chain_dag(), diamond_dag(), leak_dag(), keyed_sink_dag()] {
            for idx in 0..dag.nodes.len() {
                let file = render_node_file(&dag, idx);
                assert!(
                    file.contains("---\n"),
                    "node {idx} of {dag:?} rendered no frontmatter: {file}"
                );
            }
        }
    }
}
