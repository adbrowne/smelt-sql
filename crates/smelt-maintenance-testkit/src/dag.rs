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
    /// `SELECT id, ANY_VALUE(total) AS total FROM <upstream> GROUP BY id` —
    /// a clockless keyed downstream folding another keyed model's
    /// `KeyedUpsert`-shaped output (`docs/outcomes/20260809-output-delta-
    /// typing/phases/08-plan.md`'s `keyed_chain_dag`).
    KeyedFold,
    /// `SELECT DATE '2024-01-01' AS d, id, ANY_VALUE(total) AS total FROM
    /// <upstream> GROUP BY d, id` — a `grain: partition` downstream reading
    /// a clockless `KeyedUpsert` upstream directly by its key column
    /// (`keyed_partition_sink_dag`): the combination phase 7 flagged as
    /// deriving a key-addressed model-edge cell the run loop never
    /// dispatches (`docs/specs/incremental_models.md` §"Known
    /// Divergences"). The partition column is a constant and the `GROUP
    /// BY` is a structural no-op (the upstream is already one row per
    /// `id`) — this body exists only to PROVE the downstream's own row
    /// identity includes `id` (via the walk, not a declared `unique_key`:
    /// a declared `unique_key: id` alongside `grain: partition` trips the
    /// `GrainAssertionMismatch` diagnostic, since a declared key alone
    /// reads as key-grain shape facts) so `admit_key_addressed_recompute`'s
    /// grain proof resolves through `id`, exercising plan derivation and
    /// the full-refresh fallback correctness rather than pinning a real
    /// time axis.
    PartitionOverKeyedId,
}

/// A node's declared output grain. [`NodeGrain::Partition`] is every node in
/// [`chain_dag`]/[`diamond_dag`]/[`leak_dag`]/[`keyed_partition_sink_dag`]'s
/// `dag_kpart_b`; [`NodeGrain::Key`] is every node in [`keyed_sink_dag`] and
/// [`keyed_chain_dag`].
#[derive(Debug, Clone)]
pub enum NodeGrain {
    /// `refresh: incremental` + `grain: partition`, with the given declared
    /// `batched.unique_key`. Never renders a top-level `unique_key:` line —
    /// see [`render_node_file`]'s own note: a declared `unique_key:`
    /// alongside `grain: partition` trips `GrainAssertionMismatch`, so a
    /// partition-grain node that needs its row identity to include a
    /// non-partition column (`keyed_partition_sink_dag`'s `dag_kpart_b`)
    /// proves it via the body's own `GROUP BY` instead
    /// ([`DagBody::PartitionOverKeyedId`]).
    Partition { unique_key: Vec<String> },
    /// `refresh: incremental` + `grain: key` — no `timeseries:` block
    /// (`incremental_models.md` §Known Divergences "The key grain": "every
    /// `timeseries:` block on a keyed model is refused unconditionally").
    /// `unique_key` is rendered as a top-level `unique_key:` line when
    /// non-empty; every `Key` node this module ships leaves it empty and
    /// relies on the walk to prove grain from the body's own `GROUP BY`.
    Key { unique_key: Vec<String> },
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
                grain: NodeGrain::Key { unique_key: vec![] },
            },
        ],
    }
}

/// The clockless keyed chain: `dag_kchain_a` (`KeyedAgg`, `NodeGrain::Key`)
/// aggregates the source by `id` — a `KeyedUpsert`-shaped output — into
/// `dag_kchain_b` (`KeyedFold`, `NodeGrain::Key`), which folds it
/// (`docs/outcomes/20260809-output-delta-typing/phases/08-plan.md`'s
/// `keyed_chain_dag`, mirroring phase 7's hand-typed `agg` -> `downstream`
/// fixture, `crates/smelt-runtime/tests/key_addressed_model_edge_lowering.rs`).
/// Both nodes are grain-key with no `timeseries:` block — `dag_kchain_a`
/// proves its own `id` grain from its `GROUP BY`.
pub fn keyed_chain_dag() -> DagRecipe {
    DagRecipe {
        source: SourceRecipe::events(crate::recipe::KeyShape::Single),
        nodes: vec![
            DagNode {
                name: "dag_kchain_a".to_string(),
                upstreams: vec![Upstream::Source],
                body: DagBody::KeyedAgg,
                grain: NodeGrain::Key { unique_key: vec![] },
            },
            DagNode {
                name: "dag_kchain_b".to_string(),
                upstreams: vec![Upstream::Node(0)],
                body: DagBody::KeyedFold,
                grain: NodeGrain::Key { unique_key: vec![] },
            },
        ],
    }
}

/// The flagged inert-cell combination (phase 7's "for the next planner"
/// note): `dag_kpart_a` (`KeyedAgg`, `NodeGrain::Key`) aggregates the
/// source by `id` into `dag_kpart_b` (`PartitionOverKeyedId`,
/// `NodeGrain::Partition`) — a `grain: partition` downstream whose body
/// `GROUP BY`s its own constant partition column AND the upstream's key
/// column (`id`), so the walk PROVES its row identity includes `id`
/// without a declared `unique_key` (a declared `unique_key: id` alongside
/// `grain: partition` trips `GrainAssertionMismatch` — see
/// `DagBody::PartitionOverKeyedId`'s own doc comment), letting
/// `admit_key_addressed_recompute`'s grain proof resolve through `id` (the
/// edge's own key) even though the model's declared partition axis (`d`,
/// a constant) is a different column entirely. Plan derivation admits a
/// key-addressed `PerGroupRecompute` cell for this edge (the clock-based
/// route never applies — `dag_kpart_a` has no clock to derive), but the
/// run loop's key-addressed dispatch lives entirely inside the `grain:
/// key` branch (`smelt-runtime/src/execute.rs`), so a `grain: partition`
/// downstream like this one never reaches it — the admitted cell is
/// inert, and the model instead maintains via its ordinary (here:
/// always-widens-to-whole, since the upstream carries no clock) window
/// route. `keyed_upstream_partition_downstream_matches_oracle` pins that
/// this divergence is still CORRECT, only non-incremental.
pub fn keyed_partition_sink_dag() -> DagRecipe {
    DagRecipe {
        source: SourceRecipe::events(crate::recipe::KeyShape::Single),
        nodes: vec![
            DagNode {
                name: "dag_kpart_a".to_string(),
                upstreams: vec![Upstream::Source],
                body: DagBody::KeyedAgg,
                grain: NodeGrain::Key { unique_key: vec![] },
            },
            DagNode {
                name: "dag_kpart_b".to_string(),
                upstreams: vec![Upstream::Node(0)],
                body: DagBody::PartitionOverKeyedId,
                grain: NodeGrain::Partition { unique_key: vec![] },
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
pub fn node_output_columns(dag: &DagRecipe, node: &DagNode) -> Vec<String> {
    let d = dag.source.clock_column.clone();
    let id = dag.source.key_column.clone();
    let val = dag.source.payload_column.clone();
    match node.body {
        DagBody::PassThrough | DagBody::ParityFilter { .. } | DagBody::Union => vec![d, id, val],
        DagBody::AdditiveAgg => vec![d, "total".to_string()],
        DagBody::KeyedAgg => vec![id, "total".to_string()],
        DagBody::GroupByPayload => vec![d, val, "cnt".to_string()],
        DagBody::KeyedFold => vec![id, "total".to_string()],
        DagBody::PartitionOverKeyedId => vec![d, id, "total".to_string()],
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
        DagBody::KeyedFold => {
            let src = upstream_ref(dag, node.upstreams[0]);
            format!("SELECT {id}, ANY_VALUE(total) AS total FROM {src} GROUP BY {id}")
        }
        DagBody::PartitionOverKeyedId => {
            let src = upstream_ref(dag, node.upstreams[0]);
            format!(
                "SELECT DATE '2024-01-01' AS {d}, {id}, ANY_VALUE(total) AS total FROM {src} \
                 GROUP BY {d}, {id}"
            )
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
        NodeGrain::Key { unique_key } => {
            let uk = render_unique_key_line(unique_key);
            format!("---\nrefresh: incremental\ngrain: key\n{uk}---\n{body}\n")
        }
    }
}

/// A top-level `unique_key:` frontmatter line for `keys`, empty for no
/// declared key — used by [`NodeGrain::Key`] (`grain: partition` nodes
/// never render one, per [`render_node_file`]'s own note: a declared
/// `unique_key:` alongside `grain: partition` trips
/// `GrainAssertionMismatch`, so a partition-grain node that needs its row
/// identity to include a non-partition column proves it via the body's own
/// `GROUP BY` instead — see `DagBody::PartitionOverKeyedId`).
fn render_unique_key_line(keys: &[String]) -> String {
    if keys.is_empty() {
        String::new()
    } else {
        format!("unique_key: {}\n", keys.join(", "))
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

/// [`stage_dag`] generalised over [`crate::recipe::ConformanceTarget`]
/// (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 5):
/// `DuckDb` delegates straight to [`stage_dag`]; `SparkDelta` stages every
/// node's model file + the driving source's YAML + a Spark-targeted
/// `smelt.yml`, then creates the physical source table (and drops any stale
/// node/source table from a prior run first — drop-before-seed idempotency,
/// mirroring [`crate::render::stage_for_target`]'s convention) through a
/// direct Spark/Delta connection to `crate::recipe::SPARK_CONFORMANCE_SCHEMA`.
/// `BigQuery` mirrors `crate::render::stage_for_target`'s BigQuery arm: no
/// drop-before-seed step, since the case's dataset is fresh.
#[cfg(any(feature = "spark", feature = "bigquery"))]
pub fn stage_dag_for_target(
    dag: &DagRecipe,
    project_dir: &Path,
    db_path: &Path,
    target: crate::recipe::ConformanceTarget,
) -> anyhow::Result<LinkCProject> {
    match &target {
        crate::recipe::ConformanceTarget::DuckDb => stage_dag(dag, project_dir, db_path),
        crate::recipe::ConformanceTarget::SparkDelta => {
            #[cfg(feature = "spark")]
            {
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
                std::fs::write(
                    project_dir.join("smelt.yml"),
                    crate::render::render_smelt_yml_for(target, db_path),
                )?;

                reset_and_create_spark_dag_tables(db_path, dag)?;

                LinkCProject::load(project_dir.to_path_buf(), db_path.to_path_buf())
            }
            #[cfg(not(feature = "spark"))]
            {
                unimplemented!(
                    "ConformanceTarget::SparkDelta requires the `spark` feature on \
                     smelt-maintenance-testkit"
                )
            }
        }
        crate::recipe::ConformanceTarget::BigQuery { dataset } => {
            #[cfg(feature = "bigquery")]
            {
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
                std::fs::write(
                    project_dir.join("smelt.yml"),
                    crate::render::render_smelt_yml_for(target.clone(), db_path),
                )?;

                crate::render::create_bigquery_source_table(
                    dataset,
                    &dag.source.name,
                    &format!(
                        "{d} DATE, {id} INT64, {val} INT64",
                        d = dag.source.clock_column,
                        id = dag.source.key_column,
                        val = dag.source.payload_column,
                    ),
                )?;

                LinkCProject::load(project_dir.to_path_buf(), db_path.to_path_buf())
            }
            #[cfg(not(feature = "bigquery"))]
            {
                let _ = dataset;
                unimplemented!(
                    "ConformanceTarget::BigQuery requires the `bigquery` feature on \
                     smelt-maintenance-testkit"
                )
            }
        }
    }
}

#[cfg(feature = "spark")]
fn reset_and_create_spark_dag_tables(db_path: &Path, dag: &DagRecipe) -> anyhow::Result<()> {
    let db_path = db_path.to_path_buf();
    let schema = crate::recipe::SPARK_CONFORMANCE_SCHEMA;
    let source_table = format!("{schema}.sources_{}", dag.source.name);
    let column_defs = format!(
        "{d} DATE, {id} INT, {val} INT",
        d = dag.source.clock_column,
        id = dag.source.key_column,
        val = dag.source.payload_column,
    );
    let node_tables: Vec<String> = dag
        .nodes
        .iter()
        .map(|n| format!("{schema}.{}", n.name))
        .collect();
    crate::link_c_harness::block_on_isolated(async move {
        let backend = crate::link_c_harness::open_spark_conformance_backend(&db_path).await?;
        for table in &node_tables {
            smelt_backend::Backend::execute_sql(
                backend.as_ref(),
                &format!("DROP TABLE IF EXISTS {table}"),
            )
            .await
            .map_err(|e| anyhow::anyhow!("drop stale Spark DAG node table {table}: {e}"))?;
        }
        smelt_backend::Backend::execute_sql(
            backend.as_ref(),
            &format!("DROP TABLE IF EXISTS {source_table}"),
        )
        .await
        .map_err(|e| anyhow::anyhow!("drop stale Spark DAG source table {source_table}: {e}"))?;
        smelt_backend::Backend::execute_sql(
            backend.as_ref(),
            &format!("CREATE TABLE {source_table} ({column_defs}) USING DELTA"),
        )
        .await
        .map_err(|e| anyhow::anyhow!("create Spark DAG source table {source_table}: {e}"))?;
        Ok::<(), anyhow::Error>(())
    })
}

/// [`insert_rows`] generalised over `&dyn Backend` (plan Phase 5): appends
/// `rows` into `dag`'s physical source table via `Backend::execute_sql`
/// (Delta `INSERT INTO`) rather than a raw `duckdb::Connection` — never a
/// host-filesystem load path.
#[cfg(feature = "spark")]
pub async fn insert_rows_via_backend(
    backend: &dyn smelt_backend::Backend,
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
    backend
        .execute_sql(&format!(
            "INSERT INTO {}.sources_{} VALUES {}",
            crate::recipe::SPARK_CONFORMANCE_SCHEMA,
            dag.source.name,
            values.join(", ")
        ))
        .await
        .map_err(|e| anyhow::anyhow!("insert DAG source rows on Spark: {e}"))?;
    Ok(())
}

/// [`fetch_node_multiset`] generalised over `&dyn Backend` (plan Phase 5):
/// same column-exact, no-runtime-introspection comparison primitive, read
/// back via `Backend::execute_sql` and cast to Spark's unsized `STRING`
/// (DuckDB's unsized `VARCHAR` is refused by Spark's parser —
/// `DATATYPE_MISSING_SIZE`, the same dialect gap
/// `emit_recurrence_bound_probe` had) rather than DuckDB's `VARCHAR`.
#[cfg(feature = "spark")]
pub async fn fetch_node_multiset_via_backend(
    backend: &dyn smelt_backend::Backend,
    dag: &DagRecipe,
    idx: usize,
    where_clause: Option<&str>,
) -> anyhow::Result<Vec<Vec<String>>> {
    let node = &dag.nodes[idx];
    let cols = node_output_columns(dag, node);
    let cast_list = cols
        .iter()
        .map(|c| format!("CAST({c} AS STRING) AS {c}"))
        .collect::<Vec<_>>()
        .join(", ");
    let schema = crate::recipe::SPARK_CONFORMANCE_SCHEMA;
    let sql = match where_clause {
        Some(w) => format!("SELECT {cast_list} FROM {schema}.{} WHERE {w}", node.name),
        None => format!("SELECT {cast_list} FROM {schema}.{}", node.name),
    };
    let batches = backend
        .execute_sql(&sql)
        .await
        .map_err(|e| anyhow::anyhow!("fetch_node_multiset_via_backend {sql:?}: {e}"))?;
    let rows_as_maps = smelt_runtime::check_runner::batches_to_rows(&batches);
    let mut rows: Vec<Vec<String>> = rows_as_maps
        .into_iter()
        .map(|r| {
            cols.iter()
                .map(|c| r.get(c).cloned().unwrap_or_default())
                .collect()
        })
        .collect();
    rows.sort();
    Ok(rows)
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
        for dag in [
            chain_dag(),
            diamond_dag(),
            leak_dag(),
            keyed_sink_dag(),
            keyed_chain_dag(),
            keyed_partition_sink_dag(),
        ] {
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
        for dag in [
            chain_dag(),
            diamond_dag(),
            leak_dag(),
            keyed_sink_dag(),
            keyed_chain_dag(),
            keyed_partition_sink_dag(),
        ] {
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
