//! The printer → cast-wrap → projection seam. The enumerating legs in
//! `smelt-db`'s `dialect_audit` test the printer; this one guards the seam
//! where the `MEDIAN` re-parse bug actually lived: BigQuery's `MEDIAN`
//! lowering prints an `ARRAY_AGG`-indexing `CASE` that does not read back as
//! smelt SQL, so any consumer recovering the projection from the *printed*
//! SQL silently lost the alias.
//!
//! Five models, one per shape — not one per registry entry. This leg
//! deliberately does not scale with the registry; `dialect_audit` is what
//! covers breadth.

use crate::fixtures::{make_model, registry};

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
