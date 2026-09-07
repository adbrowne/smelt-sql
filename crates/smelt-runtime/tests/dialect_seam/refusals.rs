//! Compile-time refusal of `Emission::Unsupported` constructs, on the target's
//! dialect, before any SQL reaches the warehouse.

use crate::fixtures::{make_model, registry, FLOOR_DIVIDE_SQL};

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

/// `//` over an operand type inference cannot resolve — here, both operands
/// are columns of an upstream model with no schema information, so type
/// inference reports `Unknown` and `OperandClass::of` classifies it
/// `Unresolved` — is refused on Spark exactly like `//` over any operand
/// today (`docs/specs/multi_backend.md` §"Operand-conditional verdicts": an
/// unresolved operand must never compute a silently wrong number). `//` is
/// wholesale `Unsupported` on Spark until phase 7 makes it conditional per
/// operand class; this test is green today for that reason and stays green
/// once phase 7 lands, for the `otherwise`-arm reason instead.
#[test]
fn integer_division_with_an_unresolvable_operand_is_refused_on_spark() {
    let model = make_model("q", FLOOR_DIVIDE_SQL);
    let err = registry()
        .get("spark")
        .compile(&model, "main")
        .expect_err("Spark has no safe `//` lowering over an unresolved operand");
    let msg = format!("{err}");
    assert!(msg.contains("//"), "must name the construct: {msg}");
    assert!(
        msg.contains("UnsupportedOnBackend"),
        "must carry its diagnostic code: {msg}"
    );
}

/// The other side of the same axis (phase 7): once both operands' classes
/// *are* resolvable — here, forced to `INTEGER` by `CAST` — `//` compiles
/// for Spark via the `Integral, Integral` arm's `{0} DIV {1}` template,
/// where it previously refused wholesale.
#[test]
fn intdiv_over_typed_integer_columns_compiles_on_spark() {
    let model = make_model(
        "q",
        "SELECT CAST(id AS INTEGER) // CAST(2 AS INTEGER) AS halved FROM events",
    );
    let compiled = registry()
        .get("spark")
        .compile(&model, "main")
        .expect("both operands are known INTEGER, so `//` has a safe Spark lowering");
    assert!(
        compiled.sql.contains("DIV"),
        "expected the integral-class `DIV` template: {}",
        compiled.sql
    );
}
