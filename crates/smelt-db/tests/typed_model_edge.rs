//! `smelt-db` derives typed model edges (`docs/outcomes/20260809-output-
//! delta-typing/outcome.md` phase 9): `ModelEdge.output_shape` is no longer
//! hard-coded `None` — the Salsa layer behind `smelt explain`/diagnostics
//! folds the same cross-model output-delta verdicts the run loop derives, so
//! a `KeyedUpsert` upstream is visible to reporting and
//! `append_model_edge_cells`' key-addressed loop stops silently skipping it.

use std::fs;

use smelt_core::workspace::load_workspace;
use smelt_db::workspace_ingest::ingest_loaded_workspace;
use smelt_logical::maintenance::derive::ModelEdge;
use smelt_logical::maintenance::Technique;
use smelt_logical::OutputDelta;

const SMELT_YML: &str = r#"
name: typed_model_edge_fixture
version: 1

paths:
  - models

targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main

default_materialization: view
"#;

/// Build a real on-disk workspace under a fresh tempdir, ingest it into a
/// Salsa `Database`, and return the model edges `smelt-db` derives for
/// `model_file` (relative to `models/`, without extension) — the same edges
/// `smelt_db::maintenance_plan_report` folds into its plan.
fn model_edges_for(files: &[(&str, &str)], model_file: &str) -> Vec<ModelEdge> {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().to_path_buf();
    for (rel, content) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
    }

    let loaded = load_workspace(&root);
    let mut db = smelt_db::Database::default();
    let ingested = ingest_loaded_workspace(&mut db, &loaded);
    db.set_workspace(ingested.source_files.clone(), vec![ingested.project]);
    let ws = db.workspace();

    let target_path = root.join("models").join(format!("{model_file}.sql"));
    let file = ingested
        .source_files
        .iter()
        .zip(ingested.paths.iter())
        .find(|(_, p)| **p == target_path)
        .map(|(f, _)| *f)
        .unwrap_or_else(|| panic!("model file {target_path:?} not ingested"));

    smelt_db::model_edges_for(&db, ws, file)
}

const EVENTS_SOURCE: &str = r#"
description: Raw events, append-only, clocked on event_date.
mutation_profile: append_only
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
columns:
  - { name: event_id, type: INTEGER, nullable: false }
  - { name: user_id, type: INTEGER, nullable: false }
  - { name: event_date, type: DATE, nullable: false }
"#;

/// A clockless `grain: key` upstream (a `GROUP BY`-keyed aggregate over an
/// append-only clocked source) carries `output_shape: Some(KeyedUpsert)` on
/// its derived `ModelEdge` — the transfer-rule table's "keyed aggregation
/// over append-only emits keyed upsert" row, now reachable from the Salsa
/// path.
#[test]
fn keyed_upstream_edge_carries_keyed_upsert_output_shape() {
    let keyed_upstream = r#"---
materialization: table
refresh: incremental
grain: key
---
SELECT user_id, SUM(event_id) AS n FROM smelt.sources.events GROUP BY user_id
"#;
    let downstream = r#"---
materialization: table
refresh: incremental
grain: key
unique_key: [user_id]
---
SELECT user_id, n FROM smelt.keyed_upstream
"#;

    let edges = model_edges_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/events.yml", EVENTS_SOURCE),
            ("models/keyed_upstream.sql", keyed_upstream),
            ("models/gold_events.sql", downstream),
        ],
        "gold_events",
    );

    let edge = edges
        .iter()
        .find(|e| e.name == "keyed_upstream")
        .unwrap_or_else(|| panic!("expected a ModelEdge for 'keyed_upstream'; edges: {edges:?}"));
    assert!(
        matches!(edge.output_shape, Some(OutputDelta::KeyedUpsert { .. })),
        "expected output_shape KeyedUpsert, got {:?}",
        edge.output_shape
    );
}

/// An existing-shaped clocked upstream still types as `AppendOnlyWindow` and
/// its clock-based creation cell is unchanged (no regression of the
/// pre-existing clock-only route this phase's plumbing runs alongside).
#[test]
fn clocked_append_only_upstream_edge_carries_append_only_window() {
    let upstream = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_id, event_date FROM smelt.sources.events
"#;
    let downstream = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_id, event_date FROM smelt.silver_events
"#;

    let edges = model_edges_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/events.yml", EVENTS_SOURCE),
            ("models/silver_events.sql", upstream),
            ("models/gold_events.sql", downstream),
        ],
        "gold_events",
    );

    let edge = edges
        .iter()
        .find(|e| e.name == "silver_events")
        .unwrap_or_else(|| panic!("expected a ModelEdge for 'silver_events'; edges: {edges:?}"));
    assert!(
        matches!(
            edge.output_shape,
            Some(OutputDelta::AppendOnlyWindow { .. })
        ),
        "expected output_shape AppendOnlyWindow, got {:?}",
        edge.output_shape
    );
    assert_eq!(
        edge.clock_col.as_deref(),
        Some("event_date"),
        "the clock-based route must still see the upstream's own clock column"
    );
}

/// Over a fixture shaped like phase 8's `keyed_chain_dag` (a clockless keyed
/// upstream feeding a `grain: key` downstream), the plan report now admits a
/// key-addressed `Technique::PerGroupRecompute` cell instead of a
/// `ReachNotDerivable` refusal — the phase-8 summary's directly-verified gap
/// (`smelt-db::lib.rs`'s `ref_model_edge` hard-coded `output_shape: None`,
/// so `append_model_edge_cells`' key-addressed loop always skipped here).
#[test]
fn keyed_chain_plan_report_admits_a_key_addressed_cell() {
    let keyed_upstream = r#"---
materialization: table
refresh: incremental
grain: key
---
SELECT user_id, SUM(event_id) AS n FROM smelt.sources.events GROUP BY user_id
"#;
    let downstream = r#"---
materialization: table
refresh: incremental
grain: key
unique_key: [user_id]
---
SELECT user_id, n FROM smelt.keyed_upstream
"#;

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().to_path_buf();
    for (rel, content) in [
        ("smelt.yml", SMELT_YML),
        ("models/sources/events.yml", EVENTS_SOURCE),
        ("models/keyed_upstream.sql", keyed_upstream),
        ("models/gold_events.sql", downstream),
    ] {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
    }

    let loaded = load_workspace(&root);
    let mut db = smelt_db::Database::default();
    let ingested = ingest_loaded_workspace(&mut db, &loaded);
    db.set_workspace(ingested.source_files.clone(), vec![ingested.project]);
    let ws = db.workspace();

    let target_path = root.join("models").join("gold_events.sql");
    let file = ingested
        .source_files
        .iter()
        .zip(ingested.paths.iter())
        .find(|(_, p)| **p == target_path)
        .map(|(f, _)| *f)
        .unwrap_or_else(|| panic!("model file {target_path:?} not ingested"));

    let plan = smelt_db::maintenance_plan_report(&db, ws, file)
        .unwrap_or_else(|| panic!("gold_events has no maintenance plan"));

    assert!(
        !plan.plan.refusals.iter().any(|r| matches!(
            r,
            smelt_logical::maintenance::Refusal::ReachNotDerivable { edge, .. }
                if edge == "keyed_upstream"
        )),
        "the keyed edge must no longer be refused: {:?}",
        plan.plan.refusals
    );
    let cell = plan
        .plan
        .cells
        .iter()
        .find(|c| c.technique == Technique::PerGroupRecompute)
        .unwrap_or_else(|| {
            panic!(
                "expected a key-addressed PerGroupRecompute cell; cells: {:?}",
                plan.plan.cells
            )
        });
    let key_scope = cell
        .key_scope
        .as_ref()
        .expect("key-addressed cell must carry a key_scope");
    assert_eq!(key_scope.from, "keyed_upstream");
}

/// An upstream whose own SQL is an unclassifiable `SELECT *` projection
/// contributes no derived column groups at all (`grouping::
/// derive_column_groups`'s own wildcard fail-closed default) — the edge's
/// `output_shape` stays `None`, never a fabricated or optimistic shape.
#[test]
fn unresolvable_upstream_yields_no_output_shape() {
    let upstream = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT * FROM smelt.sources.events
"#;
    let downstream = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_id, event_date FROM smelt.silver_events
"#;

    let edges = model_edges_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/events.yml", EVENTS_SOURCE),
            ("models/silver_events.sql", upstream),
            ("models/gold_events.sql", downstream),
        ],
        "gold_events",
    );

    let edge = edges
        .iter()
        .find(|e| e.name == "silver_events")
        .unwrap_or_else(|| panic!("expected a ModelEdge for 'silver_events'; edges: {edges:?}"));
    assert_eq!(
        edge.output_shape, None,
        "an upstream whose output cannot be classified into column groups must not fabricate \
         a shape: {:?}",
        edge.output_shape
    );
}

/// A 3-model chain (`a` keyed leaf -> `b` pass-through -> `gold_events`,
/// the query file) written entirely with bare `smelt.<addr>` refs (the form
/// every fixture and the dag generator emit) composes end-to-end: the
/// direct `gold_events -> b` edge is `KeyedUpsert`, not `General`
/// (`docs/outcomes/20260809-output-delta-typing/phases/11-plan.md`). Before
/// the model-reference-leaf fix, `b`'s own SQL resolving its bare `smelt.a`
/// ref never consulted `model_verdicts` at all, so this edge fell closed to
/// `General`/`None`.
#[test]
fn three_hop_bare_ref_chain_edge_is_keyed() {
    let a = r#"---
materialization: table
refresh: incremental
grain: key
---
SELECT user_id, SUM(event_id) AS n FROM smelt.sources.events GROUP BY user_id
"#;
    let b = r#"---
materialization: table
refresh: incremental
grain: key
unique_key: [user_id]
---
SELECT user_id, n FROM smelt.a
"#;
    let downstream = r#"---
materialization: table
refresh: incremental
grain: key
unique_key: [user_id]
---
SELECT user_id, n FROM smelt.b
"#;

    let edges = model_edges_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/events.yml", EVENTS_SOURCE),
            ("models/a.sql", a),
            ("models/b.sql", b),
            ("models/gold_events.sql", downstream),
        ],
        "gold_events",
    );

    let edge = edges
        .iter()
        .find(|e| e.name == "b")
        .unwrap_or_else(|| panic!("expected a ModelEdge for 'b'; edges: {edges:?}"));
    assert!(
        matches!(edge.output_shape, Some(OutputDelta::KeyedUpsert { .. })),
        "expected output_shape KeyedUpsert composed through the bare smelt.a ref, got {:?}",
        edge.output_shape
    );
}

/// Two models referencing each other terminate — no hang, no panic — pinning
/// the bounded-pass property of `model_delta_inputs`' address-deduplicated
/// assembly (never a per-model-reference recursive Salsa query, which could
/// not terminate over a cycle).
#[test]
fn cyclic_model_refs_terminate_fail_closed() {
    let model_a = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_id, event_date FROM smelt.model_b
"#;
    let model_b = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_id, event_date FROM smelt.model_a
"#;
    let downstream = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_id, event_date FROM smelt.model_a
"#;

    let edges = model_edges_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/events.yml", EVENTS_SOURCE),
            ("models/model_a.sql", model_a),
            ("models/model_b.sql", model_b),
            ("models/gold_events.sql", downstream),
        ],
        "gold_events",
    );

    let edge = edges
        .iter()
        .find(|e| e.name == "model_a")
        .unwrap_or_else(|| panic!("expected a ModelEdge for 'model_a'; edges: {edges:?}"));
    match &edge.output_shape {
        None => {}
        Some(OutputDelta::General { .. }) => {}
        other => panic!("expected a General-or-None shape for the cyclic pair, got {other:?}"),
    }
}

/// Ingest `files` and return the `(Database, Workspace)` pair plus a lookup
/// from a model file's path (relative to `models/`, without extension) to
/// its `SourceFile` — the same ingest `model_edges_for` performs, but
/// exposing the database so a test can call a Salsa entry point directly on
/// an arbitrary model file rather than only on the one `model_edges_for`
/// walks from.
fn ingest(
    files: &[(&str, &str)],
) -> (
    tempfile::TempDir,
    smelt_db::Database,
    smelt_db::Workspace,
    std::collections::BTreeMap<String, smelt_db::SourceFile>,
) {
    // The `TempDir` guard is returned (not just its path) and must be held
    // alive by the caller for as long as `db`/`ws` are used: some Salsa
    // queries (`project_sources`, reached via `model_output_delta_for`) read
    // source declarations lazily from disk rather than from an eagerly
    // ingested structure, so dropping the tempdir here would delete the
    // fixture files out from under a later query.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().to_path_buf();
    for (rel, content) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
    }

    let loaded = load_workspace(&root);
    let mut db = smelt_db::Database::default();
    let ingested = ingest_loaded_workspace(&mut db, &loaded);
    db.set_workspace(ingested.source_files.clone(), vec![ingested.project]);
    let ws = db.workspace();

    let mut by_name = std::collections::BTreeMap::new();
    for (file, path) in ingested.source_files.iter().zip(ingested.paths.iter()) {
        if let Ok(rel) = path.strip_prefix(root.join("models")) {
            let name = rel.with_extension("");
            by_name.insert(name.to_string_lossy().replace('\\', "/"), *file);
        }
    }
    (tmp, db, ws, by_name)
}

/// A model's own `smelt_db::model_output_delta_for` is exactly the
/// `OutputDelta` a downstream's `model_edges_for` reports for that same
/// model as an upstream edge (single-owner assertion, this outcome's phase
/// 1): the headline `smelt explain` prints for a model must read the SAME
/// derivation a downstream's edge view already uses, never a second one
/// that could silently drift from it.
#[test]
fn own_output_delta_matches_downstream_edge_view() {
    let keyed_upstream = r#"---
materialization: table
refresh: incremental
grain: key
---
SELECT user_id, SUM(event_id) AS n FROM smelt.sources.events GROUP BY user_id
"#;
    let downstream = r#"---
materialization: table
refresh: incremental
grain: key
unique_key: [user_id]
---
SELECT user_id, n FROM smelt.keyed_upstream
"#;

    let files: &[(&str, &str)] = &[
        ("smelt.yml", SMELT_YML),
        ("models/sources/events.yml", EVENTS_SOURCE),
        ("models/keyed_upstream.sql", keyed_upstream),
        ("models/gold_events.sql", downstream),
    ];

    let edges = model_edges_for(files, "gold_events");
    let edge_shape = edges
        .iter()
        .find(|e| e.name == "keyed_upstream")
        .unwrap_or_else(|| panic!("expected a ModelEdge for 'keyed_upstream'; edges: {edges:?}"))
        .output_shape
        .clone();

    let (_tmp, db, ws, by_name) = ingest(files);
    let keyed_upstream_file = *by_name
        .get("keyed_upstream")
        .expect("keyed_upstream ingested");
    let own_shape = smelt_db::model_output_delta_for(&db, ws, keyed_upstream_file);

    assert!(
        matches!(own_shape, Some(OutputDelta::KeyedUpsert { .. })),
        "expected own output_shape KeyedUpsert, got {own_shape:?}"
    );
    assert_eq!(
        own_shape, edge_shape,
        "a model's own output_delta must match a downstream's edge view of the same model"
    );
}
