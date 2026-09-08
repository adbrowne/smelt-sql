//! Proves the testkit's own rendered recipe bodies print clean GoogleSQL
//! offline — the seam that let `WHERE id % 2 = 0` reach a live BigQuery
//! warehouse unlowered in the first place
//! (`dags_bigquery::diamond_propagation_suffices_on_bigquery`, `400 Syntax
//! error: Expected ")" but got "%"`, measured live 2026-08-19; fixed by
//! `7a2eb89d0`/`af972abe0`). The diamond mechanism is gated at the printer
//! (`modulo_lowering.rs`, `power_lowering.rs`), but nothing tied the
//! testkit's own `DagBody`/composed-pool render output to those lowerings —
//! this file is that tie
//! (`docs/outcomes/20260906-bigquery-correctness/phases/09-plan.md`).

use std::collections::{HashMap, HashSet};

use smelt_dialect::{print, BackendCapabilities, PrintContext, SqlDialect};
use smelt_maintenance_testkit::dag::{
    chain_dag, diamond_dag, keyed_chain_dag, keyed_partition_sink_dag, keyed_sink_dag, leak_dag,
    render_node_body, DagRecipe,
};
use smelt_maintenance_testkit::recipe::{ComposedKeyedRecipe, ComposedRoute};
use smelt_maintenance_testkit::render::{render_composed_model_body, render_composed_oracle_sql};
use smelt_parser::parse;

/// Every construct the shared scanner refuses — GoogleSQL either lacks the
/// spelling entirely (infix `%`/`^`, `VARCHAR`, `DOUBLE`, `EXCEPT ALL`, a
/// `FROM (VALUES ...)` table-value constructor) or lacks the function
/// (`MEDIAN`). Returns the subset present in `sql`, by name, so a failure
/// message names exactly what survived rather than just "something did".
fn refused_constructs(sql: &str) -> Vec<&'static str> {
    let upper = sql.to_uppercase();
    let mut found = Vec::new();
    if sql.contains('%') {
        found.push("infix `%`");
    }
    if sql.contains('^') {
        found.push("infix `^`");
    }
    if upper.contains("MEDIAN(") {
        found.push("MEDIAN(");
    }
    if upper.contains("VARCHAR") {
        found.push("VARCHAR");
    }
    if upper.contains("DOUBLE") {
        found.push("DOUBLE");
    }
    if upper.contains("EXCEPT ALL") {
        found.push("EXCEPT ALL");
    }
    if upper.contains("FROM (VALUES") {
        found.push("FROM (VALUES");
    }
    found
}

fn bigquery_print(sql: &str) -> String {
    let parsed = parse(sql);
    assert!(
        parsed.errors.is_empty(),
        "body failed to parse: {:?}\nSQL: {sql}",
        parsed.errors
    );
    let ctx = PrintContext {
        dialect: &SqlDialect::BigQuery,
        capabilities: &BackendCapabilities::bigquery(),
        schema: "main",
        ephemeral_models: HashSet::new(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: None,
        smelt_path_call: None,
        restructure_plans: &[],
        settled_emissions: &[],
    };
    print(&parsed.syntax(), &ctx)
}

/// Parses and prints `sql` under the BigQuery dialect and asserts no
/// refused construct survived — fails loud (via `assert!`/`bigquery_print`'s
/// own parse-error assertion) rather than silently skipping an unparseable
/// or non-clean body.
fn assert_clean_bigquery_render(sql: &str, context: &str) {
    let out = bigquery_print(sql);
    let hits = refused_constructs(&out);
    assert!(
        hits.is_empty(),
        "{context}: refused construct(s) {hits:?} survived BigQuery printing: {out}\nsource: {sql}"
    );
}

fn all_dag_recipes() -> Vec<DagRecipe> {
    vec![
        chain_dag(),
        diamond_dag(),
        leak_dag(),
        keyed_sink_dag(),
        keyed_chain_dag(),
        keyed_partition_sink_dag(),
    ]
}

/// Test 1 — the gate that would have caught `diamond_propagation_suffices`
/// offline: every `DagBody` variant's rendered body prints clean GoogleSQL.
/// `all_dag_recipes()`'s six recipes cover all eight `DagBody` variants
/// (`PassThrough`, `ParityFilter`, `Union`, `AdditiveAgg`, `KeyedAgg`,
/// `GroupByPayload`, `KeyedFold`, `PartitionOverKeyedId`).
#[test]
fn every_dag_body_prints_clean_googlesql() {
    for dag in all_dag_recipes() {
        for idx in 0..dag.nodes.len() {
            let sql = render_node_body(&dag, idx);
            assert_clean_bigquery_render(&sql, &format!("dag node {:?}", dag.nodes[idx].name));
        }
    }
}

/// Test 2 — the composed keyed pool's rendered bodies (the family
/// `composed_keyed_pool_upholds_equivalence_on_bigquery` reaches) over a
/// deterministic sample: all four `ComposedRoute` variants.
#[test]
fn every_composed_pool_body_prints_clean_googlesql() {
    for route in [
        ComposedRoute::KeyEmbedded,
        ComposedRoute::KeyDetermined,
        ComposedRoute::KeyDerived,
        ComposedRoute::RecurrenceBounded,
    ] {
        let recipe = ComposedKeyedRecipe::new(route);
        let model_sql = render_composed_model_body(&recipe);
        assert_clean_bigquery_render(&model_sql, &format!("composed route {route:?} model body"));

        // `render_composed_oracle_sql` is only a valid oracle for
        // `KeyEmbedded` (see its own doc comment), but it is still rendered
        // SQL text this pool ships — every route's text-substitution result
        // is checked the same way, cheaply, rather than special-cased out.
        let oracle_sql = render_composed_oracle_sql(&recipe);
        assert_clean_bigquery_render(&oracle_sql, &format!("composed route {route:?} oracle sql"));
    }
}

/// Test 3 — negative control: the scanner must actually flag every needle
/// it claims to refuse, the same fail-closed shape phase 8's
/// `an_unregistered_divergence_fails` used for the divergence registry. A
/// scan that never matches anything would let tests 1/2 pass vacuously.
#[test]
fn the_refused_construct_scan_is_not_vacuous() {
    let cases: &[(&str, &str)] = &[
        ("SELECT id % 2 FROM t", "infix `%`"),
        ("SELECT id ^ 2 FROM t", "infix `^`"),
        ("SELECT MEDIAN(val) FROM t", "MEDIAN("),
        ("SELECT CAST(x AS VARCHAR) FROM t", "VARCHAR"),
        ("SELECT CAST(x AS DOUBLE) FROM t", "DOUBLE"),
        ("SELECT * FROM a EXCEPT ALL SELECT * FROM b", "EXCEPT ALL"),
        ("SELECT * FROM (VALUES (1), (2)) AS t(x)", "FROM (VALUES"),
    ];
    for (text, needle) in cases {
        let hits = refused_constructs(text);
        assert!(
            hits.contains(needle),
            "scanner failed to flag {needle:?} in {text:?}, found: {hits:?}"
        );
    }
}

/// Test 4 — an unparseable body must fail the gate loud, not be silently
/// skipped as "nothing to check".
#[test]
#[should_panic(expected = "failed to parse")]
fn a_body_that_does_not_parse_fails_loud() {
    assert_clean_bigquery_render("SELECT FROM WHERE (((", "deliberately malformed body");
}
