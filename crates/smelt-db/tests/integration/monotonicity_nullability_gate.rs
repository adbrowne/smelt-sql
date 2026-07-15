//! Real-fixture tests for the nullability gate composing
//! `smelt_logical::trace_event_time` with smelt-db's inferred schema
//! (docs/plans/20260702-monotonicity-primitive-tested.md Phase 3).
//!
//! Spec: `docs/specs/model_properties.md` §"Event-time monotonicity trace"
//! and the column-nullability-gate row — a `Traceable` verdict whose leaf source column can be
//! `NULL` is unsound to push down (a full refresh keeps NULL-event-time
//! rows; a pushed window filter silently drops them).

use std::fs;

use smelt_db::{trace_event_time_checked, Database, Workspace};
use smelt_logical::{BoundContext, EventTimeTrace};
use tempfile::TempDir;

const SMELT_YML: &str = "name: monotonicity_nullability_fixture\n\
version: 1\n\
paths:\n  - models\n\
targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
default_materialization: view\n";

fn events_source_yaml(nullable: bool) -> String {
    format!(
        "description: Events source for monotonicity nullability gate test\n\
columns:\n\
- name: event_id\n  type: INTEGER\n  nullable: false\n\
- name: event_ts\n  type: TIMESTAMP\n  nullable: {nullable}\n"
    )
}

fn stage_files(files: &[(&str, &str)]) -> TempDir {
    let tmp = TempDir::new().expect("create tempdir");
    for (rel, contents) in files {
        let path = tmp.path().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create_dir_all");
        }
        fs::write(&path, contents).expect("write file");
    }
    tmp
}

/// Ingest a project rooted at `tmp.path()` with no SQL model files — the
/// gate is exercised purely against the declared source YAML.
fn ingest(tmp: &TempDir) -> (Database, Workspace) {
    let project_root = tmp.path().to_path_buf();
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());
    db.set_workspace(Vec::new(), vec![project]);
    let ws = db.workspace();
    (db, ws)
}

fn bare_column_expr(name: &str) -> smelt_parser::Expr {
    let sql = format!("SELECT {name} AS event_time FROM t");
    let parse = smelt_parser::parse(&sql);
    let file = smelt_parser::File::cast(parse.syntax()).expect("file cast");
    let select = file.select_stmt().expect("select stmt");
    let select_list = select.select_list().expect("select list");
    let item = select_list.items().next().expect("first select item");
    item.expression().expect("item expression")
}

fn events_ctx() -> BoundContext {
    BoundContext::new().with_source("sources.raw.events", "event_ts")
}

#[test]
fn nullable_leaf_downgraded_to_not_traceable() {
    let tmp = stage_files(&[
        ("smelt.yml", SMELT_YML),
        ("models/sources/raw/events.yml", &events_source_yaml(true)),
    ]);
    let (db, ws) = ingest(&tmp);

    let expr = bare_column_expr("event_ts");
    let trace = trace_event_time_checked(&db, ws, &expr, &events_ctx());

    assert!(
        matches!(trace, EventTimeTrace::NotTraceable { .. }),
        "nullable leaf must downgrade Traceable -> NotTraceable, got {trace:?}"
    );
}

#[test]
fn non_null_leaf_stays_traceable() {
    let tmp = stage_files(&[
        ("smelt.yml", SMELT_YML),
        ("models/sources/raw/events.yml", &events_source_yaml(false)),
    ]);
    let (db, ws) = ingest(&tmp);

    let expr = bare_column_expr("event_ts");
    let trace = trace_event_time_checked(&db, ws, &expr, &events_ctx());

    match trace {
        EventTimeTrace::Traceable {
            ref source,
            ref source_column,
            ..
        } => {
            assert_eq!(source, "sources.raw.events");
            assert_eq!(source_column, "event_ts");
        }
        other => panic!("NOT NULL leaf must stay Traceable, got {other:?}"),
    }
}

/// Phase B1 (`model_properties.md` §"Event-time monotonicity trace") wires
/// `trace_event_time`/`trace_event_time_checked` into join-input
/// driving-fact resolution (`smelt_logical::analysis::source_bounds::resolve_join_driving_fact`)
/// and UNION-branch/subquery-body classification — all of which can trace a
/// *qualified* column reference (e.g. `f.event_ts`, an aliased FROM/JOIN
/// input) rather than only a bare one. The nullability gate must downgrade
/// a `Traceable` verdict on a nullable leaf exactly the same way regardless
/// of whether the traced expression carried a qualifier — the gate reads
/// off the resolved `(source, source_column)`, not the expression's surface
/// shape.
#[test]
fn qualified_join_style_leaf_nullable_downgraded_to_not_traceable() {
    let tmp = stage_files(&[
        ("smelt.yml", SMELT_YML),
        ("models/sources/raw/events.yml", &events_source_yaml(true)),
    ]);
    let (db, ws) = ingest(&tmp);

    // Shape a join/UNION consumer would trace: a column qualified by its
    // FROM/JOIN alias (the alias itself is irrelevant to nullability
    // resolution — only the resolved source_column name is).
    let expr = bare_column_expr("f.event_ts");
    let trace = trace_event_time_checked(&db, ws, &expr, &events_ctx());

    assert!(
        matches!(trace, EventTimeTrace::NotTraceable { .. }),
        "nullable leaf traced through a qualified (join-style) reference must downgrade, got {trace:?}"
    );
}

#[test]
fn unresolvable_leaf_fails_closed() {
    // No source YAML declares `sources.raw.events` at all: the leaf column's
    // nullability is unresolvable, so the gate must fail closed.
    let tmp = stage_files(&[("smelt.yml", SMELT_YML)]);
    let (db, ws) = ingest(&tmp);

    let expr = bare_column_expr("event_ts");
    let trace = trace_event_time_checked(&db, ws, &expr, &events_ctx());

    assert!(
        matches!(trace, EventTimeTrace::NotTraceable { .. }),
        "unresolvable leaf nullability must fail closed to NotTraceable, got {trace:?}"
    );
}
