//! The compile path refuses a construct the registry declares unsupported on
//! the target's dialect, rather than emitting SQL the engine rejects.
//!
//! `smelt-dialect`'s `unsupported_emission` suite proves the pure check; this
//! file proves it is actually wired into every `SqlCompiler` print, so a model
//! never reaches a warehouse carrying a construct that warehouse cannot express.

use smelt_core::config::{Config, Materialization, Target};
use smelt_core::ModelFile;
use smelt_runtime::CompilerRegistry;
use std::collections::HashMap;

fn duckdb_target() -> Target {
    Target {
        target_type: "duckdb".to_string(),
        database: Some("test.duckdb".to_string()),
        schema: "main".to_string(),
        connect_url: None,
        catalog: None,
        warehouse: None,
        format: None,
        settings: None,
        project: None,
        dataset: None,
        location: None,
    }
}

fn bigquery_target() -> Target {
    Target {
        target_type: "bigquery".to_string(),
        database: None,
        schema: "main".to_string(),
        connect_url: None,
        catalog: None,
        warehouse: None,
        format: None,
        settings: None,
        project: Some("p".to_string()),
        dataset: Some("main".to_string()),
        location: Some("US".to_string()),
    }
}

fn spark_target() -> Target {
    Target {
        target_type: "spark".to_string(),
        database: None,
        schema: "main".to_string(),
        connect_url: Some("sc://localhost:15002".to_string()),
        catalog: None,
        warehouse: None,
        format: None,
        settings: None,
        project: None,
        dataset: None,
        location: None,
    }
}

fn registry() -> CompilerRegistry {
    let mut targets = HashMap::new();
    targets.insert("duckdb".to_string(), duckdb_target());
    targets.insert("spark".to_string(), spark_target());
    targets.insert("bigquery".to_string(), bigquery_target());
    let config = Config {
        name: "dialect_seam".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets: targets.clone(),
        default_materialization: Materialization::View,
        models: HashMap::new(),
        python: None,
        target: None,
        state: Default::default(),
        maintenance: None,
        probes: Default::default(),
    };
    CompilerRegistry::new(&config, &targets)
}

fn make_model(name: &str, sql: &str) -> ModelFile {
    let parse = smelt_parser::parse(sql);
    let refs = smelt_parser::ast::File::cast(parse.syntax())
        .map(|f| smelt_core::extract_refs(&f))
        .unwrap_or_default();
    let path = std::path::PathBuf::from(format!("models/{name}.sql"));
    ModelFile {
        name: name.to_string(),
        path: path.clone(),
        content: sql.to_string(),
        refs,
        parse_errors: Vec::new(),
        metadata: None,
        kind: smelt_core::ModelKind::Sql,
        model_id: smelt_core::ModelId::from_path(path),
        address_segments: vec![name.to_string()],
    }
}

const FLOOR_DIVIDE_SQL: &str = "SELECT id, val // 2 AS halved FROM events";

#[test]
fn a_model_using_floor_divide_fails_to_compile_for_bigquery() {
    let model = make_model("q", FLOOR_DIVIDE_SQL);
    let err = registry()
        .get("bigquery")
        .compile(&model, "main")
        .expect_err("BigQuery has no `//`; the compiler must refuse before emitting SQL");
    let msg = format!("{err}");
    assert!(msg.contains("//"), "must name the construct: {msg}");
    assert!(
        msg.contains("BigQuery") || msg.contains("bigquery"),
        "must name the backend: {msg}"
    );
    assert!(
        msg.contains("UnsupportedOnBackend"),
        "must carry its diagnostic code so the CLI output is greppable: {msg}"
    );
}

#[test]
fn the_same_model_compiles_for_duckdb() {
    let model = make_model("q", FLOOR_DIVIDE_SQL);
    registry()
        .get("duckdb")
        .compile(&model, "main")
        .expect("DuckDB has `//`");
}

/// The end-to-end leg of criterion 3, deferred from phase 3 because no
/// production template was call-shaped until phase 4 registered `DATE_SUB` as
/// one (`docs/outcomes/20260904-dialect-emission-vocabulary` phase 4). A
/// `BINARY_EXPR` operator template (`%`, `^`, `**`) can carry none of these
/// modifiers, so this needed a function-call template row to exist at all.
#[test]
fn a_template_call_carrying_distinct_is_refused_for_duckdb() {
    let model = make_model(
        "q",
        "SELECT DATE_SUB(DISTINCT d, INTERVAL 1 DAY) AS x FROM events",
    );
    let err = registry()
        .get("duckdb")
        .compile(&model, "main")
        .expect_err("a template call cannot carry DISTINCT");
    let msg = format!("{err}");
    assert!(msg.contains("DISTINCT"), "must name the modifier: {msg}");
    assert!(
        msg.contains("UnsupportedOnBackend"),
        "must carry its diagnostic code: {msg}"
    );
}

/// An `Emission::Unsupported` verdict reaches the user as a compile-time
/// diagnostic rather than an engine error — `INITCAP` is the first DuckDB row
/// closed this way (phase 4: DuckDB 1.5.4 has no such scalar).
#[test]
fn a_model_using_initcap_is_refused_for_duckdb() {
    let model = make_model("q", "SELECT INITCAP(name) AS x FROM events");
    let err = registry()
        .get("duckdb")
        .compile(&model, "main")
        .expect_err("DuckDB has no `initcap`");
    let msg = format!("{err}");
    assert!(
        msg.contains("INITCAP") || msg.contains("initcap"),
        "must name the construct: {msg}"
    );
    assert!(
        msg.contains("UnsupportedOnBackend"),
        "must carry its diagnostic code: {msg}"
    );
}

/// The refusal is not specific to `compile`: an ephemeral model is inlined as a
/// CTE into its consumer, so it never passes through a consumer's own check.
#[test]
fn an_ephemeral_model_is_refused_too() {
    let err = registry()
        .get("bigquery")
        .build_ephemeral_resolver(
            &[("staged".to_string(), FLOOR_DIVIDE_SQL.to_string())],
            "main",
        )
        .expect_err("an inlined ephemeral CTE reaches the same warehouse");
    assert!(format!("{err}").contains("//"));
}

/// Every occurrence is listed, so a user is not walked through one compile
/// round trip per site.
#[test]
fn all_occurrences_are_named_in_one_error() {
    let model = make_model("q", "SELECT a // b AS x, c // d AS y FROM t");
    let err = registry()
        .get("bigquery")
        .compile(&model, "main")
        .expect_err("two refusals");
    let msg = format!("{err}");
    assert!(msg.contains("2 constructs"), "{msg}");
}

/// Structural: no compile entry point may reach the printer directly, or it
/// would skip the refusal. `print_checked_for` is the sole permitted caller.
#[test]
fn every_compile_path_is_emission_checked() {
    const COMPILE_SRC: &str = include_str!("../src/compile.rs");
    // The two hardwired-DuckDB helpers (`resolve_refs_in_sql` and the
    // function-body expander) are exempt: they take no dialect, return no
    // `Result`, and sit on no path that produces an executed `CompiledModel`.
    // Not because DuckDB is free of unsupported constructs — it declares
    // `PERCENTILE_CONT`/`PERCENTILE_DISC` unsupported in running-window
    // position.
    const EXEMPT: usize = 2;
    let direct = COMPILE_SRC.matches("smelt_dialect::print(").count();
    assert_eq!(
        direct,
        EXEMPT + 1,
        "compile.rs calls `smelt_dialect::print` {direct} times; only \
         `print_checked_for` plus the {EXEMPT} hardwired-DuckDB helpers may. A new \
         compile path must print through `print_checked`, or it skips the \
         `UnsupportedOnBackend` refusal."
    );
}

/// A running window over a built-in with no analytic form on the target has
/// no correct CTE form (`docs/specs/multi_backend.md` §"Statement-level
/// lowering": "A running-frame window ... has no correct CTE form and is
/// refused with `UnsupportedOnBackend`"). This refusal is registry data that
/// already ships as registry data (`Position::Window` →
/// `Emission::Unsupported` for the ordered-set `PERCENTILE_CONT`/
/// `PERCENTILE_DISC` family on DuckDB and Spark) — this test pins that it
/// reaches the user as a compile-time `UnsupportedOnBackend` diagnostic
/// rather than a warehouse-side parse error, and that it keeps doing so as
/// the restructure planner is wired into the same compile path.
#[test]
fn running_window_refused_at_compile_time() {
    let model = make_model(
        "q",
        "SELECT id, g, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) \
         OVER (PARTITION BY g ORDER BY t) AS med FROM tbl",
    );
    let err = registry().get("duckdb").compile(&model, "main").expect_err(
        "DuckDB has the ordered-set aggregate but no running-window form; \
             this must refuse at compile time",
    );
    let msg = format!("{err}");
    assert!(
        msg.contains("UnsupportedOnBackend"),
        "must carry its diagnostic code so the CLI output is greppable: {msg}"
    );
    assert!(
        msg.contains("PERCENTILE_CONT"),
        "must name the construct: {msg}"
    );
    assert!(
        !msg.to_lowercase().contains("syntax error")
            && !msg.to_lowercase().contains("binder error"),
        "must be smelt's own diagnostic, not a warehouse-shaped error: {msg}"
    );
}

/// Structural, mirroring `every_compile_path_is_emission_checked`: no compile
/// entry point may plan a statement-level restructure and then print without
/// going through `print_checked_for` — the same seam that refuses an
/// `Unsupported` construct is where a `Restructure` verdict gets planned, so
/// a new print call bypassing it would silently skip both the refusal and
/// the planning.
#[test]
fn no_compile_entry_point_prints_without_planning() {
    const COMPILE_SRC: &str = include_str!("../src/compile.rs");
    let plan_calls = COMPILE_SRC
        .matches("smelt_dialect::plan_restructure(")
        .count();
    assert_eq!(
        plan_calls, 1,
        "compile.rs calls `smelt_dialect::plan_restructure` {plan_calls} times; only \
         `print_checked_for` may call it. A new compile path constructing its own plan \
         (or none) would drift from the single planning site."
    );
}

/// BigQuery's `APPROX_COUNT_DISTINCT` has no analytic form at all — GoogleSQL's
/// own dry run accepts `APPROX_COUNT_DISTINCT(x) OVER (PARTITION BY g ORDER BY
/// …)`, but execution refuses it (measured live 2026-08-27). This pins the
/// compile-time behaviour: a running window still refuses with
/// `UnsupportedOnBackend`.
#[test]
fn approx_count_distinct_refused_in_running_window_position_on_bigquery() {
    let model = make_model(
        "q",
        "SELECT id, g, APPROX_COUNT_DISTINCT(id) \
         OVER (PARTITION BY g ORDER BY id) AS approx_distinct FROM tbl",
    );
    let err = registry()
        .get("bigquery")
        .compile(&model, "main")
        .expect_err(
            "BigQuery's APPROX_COUNT_DISTINCT has no analytic form; a running \
             window must refuse, not compile",
        );
    assert!(
        format!("{err}").contains("APPROX_COUNT_DISTINCT"),
        "the refusal must name the built-in: {err}"
    );
}

/// The user docs quote the exact `UnsupportedOnBackend` refusal text so a
/// reader can recognise it verbatim; this pins that quote against what the
/// compile path actually emits, so the guide cannot drift from the
/// diagnostic (`docs/plans/20260827-statement-level-lowering.md` Phase 7).
///
/// The doc's quoted block (`docs-site/docs/reference/diagnostics.md`, marked
/// by the `<!-- unsupported-on-backend-refusal-text -->` comment) is
/// extracted from the markdown rather than hand-copied into this test, and
/// compared against the live error text from the same running-window model
/// used above — so editing either the reason string in `signatures.rs` or
/// the quoted text in the docs, without updating the other, fails this test.
#[test]
fn docs_quoted_refusal_text_matches_the_live_diagnostic() {
    const DOC_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs-site/docs/reference/diagnostics.md"
    );
    let doc = std::fs::read_to_string(DOC_PATH)
        .unwrap_or_else(|e| panic!("failed to read {DOC_PATH}: {e}"));

    const MARKER: &str = "<!-- unsupported-on-backend-refusal-text -->";
    let after_marker = doc
        .split_once(MARKER)
        .unwrap_or_else(|| panic!("docs no longer carry the {MARKER} marker"))
        .1;
    let fence_start = after_marker
        .find("```text")
        .expect("marker must be immediately followed by a ```text fenced block")
        + "```text".len();
    let fenced = &after_marker[fence_start..];
    let fence_end = fenced
        .find("```")
        .expect("the ```text block quoting the refusal must be closed");
    let quoted = fenced[..fence_end].trim_matches('\n');

    // The same running-window model as `running_window_refused_at_compile_time`.
    // Its single `PERCENTILE_CONT(...) WITHIN GROUP (...) OVER (...)` call is
    // flagged twice — once for the ordered-set aggregate, once for the window
    // it sits under — so the message reads "2 constructs" with two identical
    // detail lines. That is the live shape the doc's quote must match.
    let model = make_model(
        "q",
        "SELECT id, g, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) \
         OVER (PARTITION BY g ORDER BY t) AS med FROM tbl",
    );
    let err = registry()
        .get("duckdb")
        .compile(&model, "main")
        .expect_err("running-window PERCENTILE_CONT must refuse on DuckDB");
    let live = format!("{err}");

    assert_eq!(
        live, quoted,
        "the docs' quoted UnsupportedOnBackend text has drifted from what the \
         compile path actually emits. Live:\n{live}\n\nDocs (from {DOC_PATH}):\n{quoted}"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// The seam leg.
//
// The enumerating legs in `smelt-db`'s `dialect_audit` test the printer. This
// one guards the printer → cast-wrap → projection seam, which is where the
// `MEDIAN` re-parse bug actually lived: BigQuery's `MEDIAN` lowering prints an
// `ARRAY_AGG`-indexing `CASE` that does not read back as smelt SQL, so any
// consumer recovering the projection from the *printed* SQL silently lost the
// alias.
//
// Five models, one per shape — not one per registry entry. This leg
// deliberately does not scale with the registry; `dialect_audit` is what
// covers breadth.
// ─────────────────────────────────────────────────────────────────────────

/// `(model name, SQL, expected output columns)`, one per emission shape.
const SEAM_MODELS: &[(&str, &str, &[&str])] = &[
    (
        "seam_scalar",
        "SELECT id, UPPER(name) AS u FROM events",
        &["id", "u"],
    ),
    (
        "seam_aggregate",
        "SELECT id, MEDIAN(val) AS med FROM events GROUP BY id",
        &["id", "med"],
    ),
    (
        "seam_window",
        "SELECT id, MEDIAN(val) OVER (PARTITION BY id) AS med FROM events",
        &["id", "med"],
    ),
    (
        "seam_operator",
        "SELECT id, val % 3 AS r, val ** 2 AS s FROM events",
        &["id", "r", "s"],
    ),
    (
        "seam_tablefn",
        "SELECT u FROM events, UNNEST(tags) AS u",
        &["u"],
    ),
];

/// Every backend must derive the same projection from the same model, whatever
/// its own lowering prints. A dialect's `MEDIAN`, `%` or `**` rewrite leaking
/// into `output_columns` is the failure this catches.
#[test]
fn the_projection_survives_every_backends_lowering() {
    let registry = registry();
    for (name, sql, expected) in SEAM_MODELS {
        let model = make_model(name, sql);
        let expected: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
        for backend in ["duckdb", "spark", "bigquery"] {
            let compiled = registry
                .get(backend)
                .compile(&model, "main")
                .unwrap_or_else(|e| panic!("{name} must compile for {backend}: {e}"));
            assert_eq!(
                compiled.output_columns, expected,
                "{name} on {backend}: the projection was recovered from the \
                 dialect-printed SQL rather than derived from the source select \
                 list.\n  sql = {}",
                compiled.sql
            );
        }
    }
}

/// The lowerings actually fire — a leg that only compared projections would
/// pass just as well if no backend lowered anything at all.
#[test]
fn each_backend_actually_lowers_what_it_must() {
    let registry = registry();
    let operator = make_model("seam_operator", SEAM_MODELS[3].1);
    let bigquery = registry
        .get("bigquery")
        .compile(&operator, "main")
        .expect("compile");
    assert!(
        bigquery.sql.contains("MOD(") && bigquery.sql.contains("POWER("),
        "GoogleSQL has no infix `%` and reads `^` as XOR — both must lower: {}",
        bigquery.sql
    );
    assert!(
        !bigquery.sql.contains(" % ") && !bigquery.sql.contains("**"),
        "an unlowered operator survived into the BigQuery SQL: {}",
        bigquery.sql
    );

    let duckdb = registry
        .get("duckdb")
        .compile(&operator, "main")
        .expect("compile");
    assert!(
        duckdb.sql.contains(" % ") && duckdb.sql.contains("**"),
        "DuckDB owns both operators natively; lowering them would be a \
         needless rewrite: {}",
        duckdb.sql
    );

    let aggregate = make_model("seam_aggregate", SEAM_MODELS[1].1);
    let bq_median = registry
        .get("bigquery")
        .compile(&aggregate, "main")
        .expect("compile");
    assert!(
        !bq_median.sql.contains("MEDIAN("),
        "GoogleSQL has no MEDIAN; it must be rewritten: {}",
        bq_median.sql
    );
}
