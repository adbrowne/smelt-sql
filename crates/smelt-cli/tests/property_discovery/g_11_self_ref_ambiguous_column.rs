//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `G-11` (`docs/research/20260705-property-discovery-loop.md` §4,
//! appended from `G-08`).
//!
//! Construct: the SAME self-referential running-balance model as `G-08`, but
//! using the DIRECT self-join form `docs/specs/batched_models.md`'s own
//! "Window independence and self-referential models" section documents and
//! `window_independence`'s own unit tests exercise — a JOIN of the driving
//! source to `smelt.<self>`, both exposing the model's own output/partition
//! column under its OWN bare name (`t.d` / `bal.d`) — rather than G-08's
//! subquery-wrapped form (which the loop discovered was *necessary*, not
//! merely stylistic, while building G-08's test).
//!
//! This cell is execution-layer, not maintenance-correctness: it tests
//! whether `crates/smelt-runtime/src/transformer.rs::inject_time_filter` —
//! which injects the outer output clamp as a BARE, unqualified
//! `{event_time_column} >= .. AND {event_time_column} < ..` whenever
//! `is_transparent_single_source` is false (true for any self-referential
//! model, since the self-edge is itself a second bounded source) — can even
//! produce SQL DuckDB will execute, when the FROM scope exposes the bare
//! column name from more than one input.
//!
//! Hypothesis: DuckDB rejects the compiled SQL with `Binder Error: Ambiguous
//! reference to column name "d"` on EVERY run of this model shape, because
//! both `t` and `bal` expose a `d` column and the injected clamp does not
//! qualify which one it means. This reproduces on the FIRST run — no
//! adversarial schedule needed; it is a hard compile-time-of-SQL failure, not
//! a data-dependent divergence.

use crate::link_c_harness::{base_request, LinkCProject};
use crate::model_shapes::{running_balance_self_ref_direct_join, ModelShape};

fn stage_project(shape: &ModelShape, project_dir: &std::path::Path, db_path: &std::path::Path) {
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();
    std::fs::write(
        project_dir.join(format!("models/{}.sql", shape.name)),
        shape.sql,
    )
    .unwrap();

    let cols: String = shape
        .source_columns
        .iter()
        .map(|c| format!("  - name: {}\n    type: {}\n", c.name, c.ty))
        .collect();
    let source_yml = format!(
        "description: property-discovery source.\nmutation_profile: append_only\ncolumns:\n{cols}"
    );
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", shape.source)),
        source_yml,
    )
    .unwrap();

    let smelt_yml = format!(
        "name: property_discovery\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

fn seed_sources(db_path: &std::path::Path) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE TABLE main.sources_transactions (d DATE, amt DOUBLE);
        INSERT INTO main.sources_transactions VALUES
            (DATE '2024-01-01', 10.0),
            (DATE '2024-01-02', 5.0);
        CREATE TABLE main.running_balance (d DATE, balance DOUBLE);
        "#,
    )
    .expect("seed sources");
}

/// RED (pre-fix): the direct self-join form — the exact shape
/// `docs/specs/batched_models.md` documents and `window_independence`'s own
/// unit tests use — fails on its very first run because the injected outer
/// clamp is an unqualified bare column reference into a FROM scope that
/// exposes that column name from two inputs (`t.d`, `bal.d`).
#[tokio::test]
async fn direct_self_join_output_clamp_is_ambiguous_without_subquery_wrap() {
    let shape = running_balance_self_ref_direct_join();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&shape, &project_dir, &db_path);
    seed_sources(&db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    let mut request = base_request("dev");
    request.start = Some("2024-01-01".to_string());
    request.end = Some("2024-01-02".to_string());

    let result = project.run_quiet("run-1", request).await;

    let err = result.expect_err(
        "the direct self-join form (no subquery wrap) is expected to fail: the outer \
         output-clamp injection is an unqualified bare column reference into a FROM \
         scope where both the driving source and the self-reference expose that same \
         column name",
    );
    let message = format!("{err:#}");
    assert!(
        message.contains("Ambiguous") || message.contains("ambiguous"),
        "expected a DuckDB ambiguous-column binder error, got: {message}"
    );
}
