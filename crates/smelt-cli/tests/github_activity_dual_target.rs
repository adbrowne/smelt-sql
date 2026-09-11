#![cfg(feature = "duckdb")]
//! Dual-target parity for `examples/github_activity/`: does the pipeline
//! compute the same answers on DuckDB and on BigQuery?
//! (`docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md` criterion 6.)
//!
//! **This file is the offline half.** Everything here runs per-PR with no
//! credential and no cloud: the comparator, the relation-set totality check,
//! the divergence registry and the landing seam are all exercised against
//! synthetic local `.duckdb` files and synthetic NDJSON. The live sweep — the
//! schedule, the population the two legs share, and the committed parity
//! report — is deliberately **not** wired up here; it is being re-planned
//! (the comparison basis moved from mirroring BigQuery's population down into
//! DuckDB to expanding the BigQuery population up to the committed fixture),
//! and a comparator that cannot be trusted offline is worth nothing live.
//!
//! # The comparator
//!
//! Whole-row multiset difference — `EXCEPT ALL` in both directions over
//! `SELECT *` — run inside DuckDB with the BigQuery side landed locally,
//! exactly the primitive `github_activity_oracle.rs`'s `relation_diff`
//! already gates on. Deliberately **not**
//! `crates/smelt-cli/tests/common/mod.rs`'s `batches_to_sorted_rows`, which
//! stringifies every cell and so collapses type differences into formatting
//! differences, and gives an unactionable diff on a multi-thousand-row
//! relation.
//!
//! Relation discovery is generic — `information_schema` on the DuckDB side,
//! `INFORMATION_SCHEMA.TABLES` on the BigQuery side — never a hardcoded model
//! list, and a relation present on only one target is a **coverage failure**
//! rather than a quietly smaller comparison. That is what makes the sweep
//! non-vacuous.
//!
//! # The landing seam
//!
//! The BigQuery side reaches the comparator through one pluggable step
//! ([`load_bigquery_snapshot`]): export the rows to typed NDJSON
//! (`scripts/bq_dogfood_export.py`, which decodes BigQuery's all-strings REST
//! encoding using the result schema), then cast each column into the
//! **DuckDB leg's own** declared type for that column, read from
//! `information_schema.columns`.
//!
//! ## Declared normalisation, and nothing else
//!
//! That cast is the **only** admissible normalisation: INT64→BIGINT,
//! FLOAT64→DOUBLE, NUMERIC→DECIMAL, TIMESTAMP→TIMESTAMP at UTC, DATE→DATE,
//! STRING→VARCHAR, BOOL→BOOLEAN. A BigQuery value that will not cast is a
//! **finding**, not a tolerance — the loader raises rather than coercing
//! ([`a_value_that_will_not_cast_is_a_loud_failure`]).
//!
//! Any tolerance beyond exact value equality — float epsilon, timestamp
//! truncation, string trimming — is a registered [`TargetDivergence`] with a
//! root-caused reason, never a quiet comparator setting. There is no
//! comparator knob to loosen.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Relation discovery
// ---------------------------------------------------------------------------

/// Never compared, by prefix:
/// - `sources_` — the raw loaded inputs. The two legs are populated from the
///   same sample by construction, so comparing them would measure the loader
///   rather than the pipeline.
/// - `_smelt_` — run bookkeeping (`_smelt_ledger`, `_smelt_observed_delta`).
///   They record *how* a run happened, not model state, and the two targets
///   legitimately realise different state structures (`docs/specs/state.md`).
const EXCLUDED_PREFIXES: &[&str] = &["sources_", "_smelt_"];

/// Never compared, by suffix: `<model>__tombstones` is a keyed-succession
/// sibling — bookkeeping, for the same reason as `_smelt_*`.
const EXCLUDED_SUFFIXES: &[&str] = &["__tombstones"];

/// Never compared, by exact name:
/// - `github_events` / `github_events_arrival` — the BigQuery leg's physical
///   *source* tables (`models/sources/raw/*.yml` map `raw.github_events` onto
///   them via the target-aware `name:` override). They are the `sources_`
///   exclusion's BigQuery-side spelling.
/// - `_loader_days` — `load_day.sh`'s own idempotence bookkeeping on the
///   DuckDB leg.
const EXCLUDED_EXACT: &[&str] = &["github_events", "github_events_arrival", "_loader_days"];

/// The two models excluded from **both** legs, so the relation sets are equal
/// by construction rather than by tolerance. Their absence from BigQuery is a
/// compile-time `UnsupportedOnBackend` refusal on GoogleSQL — the `RANGE
/// BETWEEN INTERVAL '2 days' PRECEDING` lookback frame, which GoogleSQL
/// allows only with numeric offsets — not a value divergence:
/// `silver.actor_sessions` carries the frame and
/// `marts.daily_active_contributors` is its only downstream consumer.
const EXCLUDED_MODELS: &[&str] = &["silver.actor_sessions", "marts.daily_active_contributors"];

fn is_compared(name: &str) -> bool {
    !EXCLUDED_PREFIXES.iter().any(|p| name.starts_with(p))
        && !EXCLUDED_SUFFIXES.iter().any(|s| name.ends_with(s))
        && !EXCLUDED_EXACT.contains(&name)
}

fn discover_relations(db: &Path) -> Vec<String> {
    let conn = duckdb::Connection::open(db).unwrap_or_else(|e| panic!("open {db:?}: {e}"));
    let mut stmt = conn
        .prepare(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = 'main' ORDER BY 1",
        )
        .expect("prepare relation discovery");
    let names: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query relation discovery")
        .collect::<Result<_, _>>()
        .expect("collect relation names");
    names.into_iter().filter(|n| is_compared(n)).collect()
}

// ---------------------------------------------------------------------------
// The comparator
// ---------------------------------------------------------------------------

fn attached_conn(duck_db: &Path, bq_db: &Path) -> duckdb::Connection {
    let conn = duckdb::Connection::open_in_memory().expect("open in-memory duckdb");
    conn.execute_batch(&format!(
        "ATTACH '{}' AS duck_db (READ_ONLY); ATTACH '{}' AS bq_db (READ_ONLY);",
        duck_db.display(),
        bq_db.display()
    ))
    .expect("attach both databases");
    conn
}

fn scalar_on(conn: &duckdb::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0))
        .unwrap_or_else(|e| panic!("query failed: {e}\nSQL:\n{sql}"))
}

/// Up to 5 offending rows rendered as JSON, for a failure message.
fn sample_rows(conn: &duckdb::Connection, diff_sql: &str) -> Vec<String> {
    let sql = format!("SELECT to_json(d) FROM ({diff_sql}) AS d LIMIT 5");
    let mut stmt = conn
        .prepare(&sql)
        .unwrap_or_else(|e| panic!("prepare failed: {e}\nSQL:\n{sql}"));
    stmt.query_map([], |row| row.get::<_, String>(0))
        .unwrap_or_else(|e| panic!("query failed: {e}\nSQL:\n{sql}"))
        .collect::<Result<_, _>>()
        .unwrap_or_else(|e| panic!("collect sample rows failed: {e}\nSQL:\n{sql}"))
}

#[derive(Debug)]
struct RelationDiff {
    relation: String,
    duck_rows: i64,
    bq_rows: i64,
    duck_only: i64,
    bq_only: i64,
    duck_only_sample: Vec<String>,
    bq_only_sample: Vec<String>,
}

fn relation_diff(conn: &duckdb::Connection, relation: &str) -> RelationDiff {
    let duck_only_sql = format!(
        "SELECT * FROM duck_db.main.{relation} EXCEPT ALL SELECT * FROM bq_db.main.{relation}"
    );
    let bq_only_sql = format!(
        "SELECT * FROM bq_db.main.{relation} EXCEPT ALL SELECT * FROM duck_db.main.{relation}"
    );
    RelationDiff {
        relation: relation.to_string(),
        duck_rows: scalar_on(
            conn,
            &format!("SELECT count(*) FROM duck_db.main.{relation}"),
        ),
        bq_rows: scalar_on(conn, &format!("SELECT count(*) FROM bq_db.main.{relation}")),
        duck_only: scalar_on(conn, &format!("SELECT count(*) FROM ({duck_only_sql})")),
        bq_only: scalar_on(conn, &format!("SELECT count(*) FROM ({bq_only_sql})")),
        duck_only_sample: sample_rows(conn, &duck_only_sql),
        bq_only_sample: sample_rows(conn, &bq_only_sql),
    }
}

/// Relation-set totality first, then a whole-row multiset difference per
/// relation. `Err` names a relation present on only one target — coverage
/// totality, never a silently smaller comparison.
fn compare_databases(duck_db: &Path, bq_db: &Path) -> Result<Vec<RelationDiff>, String> {
    let duck_relations = discover_relations(duck_db);
    let bq_relations = discover_relations(bq_db);
    let mut all: BTreeSet<String> = BTreeSet::new();
    all.extend(duck_relations.iter().cloned());
    all.extend(bq_relations.iter().cloned());

    for rel in &all {
        let on_duck = duck_relations.contains(rel);
        let on_bq = bq_relations.contains(rel);
        if !on_duck || !on_bq {
            let missing_from = if on_duck { "bigquery" } else { "duckdb" };
            return Err(format!(
                "relation `{rel}` present on only one target (duckdb={on_duck}, \
                 bigquery={on_bq}) — missing from {missing_from}"
            ));
        }
    }

    let conn = attached_conn(duck_db, bq_db);
    Ok(all.iter().map(|rel| relation_diff(&conn, rel)).collect())
}

// ---------------------------------------------------------------------------
// The divergence vocabulary
// ---------------------------------------------------------------------------

/// Which target a [`DivergenceBound::MonotoneDivergence`] licenses to be
/// behind. Neither variant has a registry entry today
/// ([`TARGET_DIVERGENCE_REGISTRY`] is empty); both are kept live by
/// [`a_monotone_bound_holds_and_rejects_the_leading_side`].
#[derive(PartialEq, Eq)]
enum BehindSide {
    Duckdb,
    Bigquery,
}

/// A registry entry's checkable bound. A bound never licenses a missing or
/// extra row — only a differing value, and only in columns it names
/// explicitly.
enum DivergenceBound {
    /// One target is always at or behind the other on `monotone_columns`
    /// (`behind_side` names the one never allowed to lead), and every row
    /// present on both targets matches exactly on every column in
    /// `exact_columns`.
    MonotoneDivergence {
        key_col: &'static str,
        exact_columns: &'static [&'static str],
        monotone_columns: &'static [&'static str],
        behind_side: BehindSide,
    },
}

/// A registered difference between the two targets, carrying a root-caused
/// reason and a checkable bound. An unregistered non-zero diff fails the
/// sweep ([`check_targets_agree`]).
struct TargetDivergence {
    relation: &'static str,
    reason: &'static str,
    bound: DivergenceBound,
}

/// **Empty**, and empty is a claim rather than an omission.
///
/// No cross-target difference has been measured and root-caused yet, so there
/// is nothing to register. An empty registry plus a vacuous sweep would be
/// indistinguishable from success, so it is not shipped as one:
/// [`the_sweep_fails_closed_on_an_empty_registry`] drives the
/// registry-consulting comparator itself over a perturbed pair and requires
/// it to report the mismatch, and
/// [`an_unregistered_target_divergence_fails`] does the same for
/// [`compare_databases`] underneath it. Both are the precedent at
/// `github_activity_oracle.rs::assert_matches_oracle_fails_closed_on_an_empty_registry`.
///
/// The two-sided liveness ratchet an entry needs — "an entry naming a
/// relation that no longer diverges is an error telling you to delete it" —
/// lands with the live sweep, against the measured parity report it will
/// check entries against.
const TARGET_DIVERGENCE_REGISTRY: &[TargetDivergence] = &[];

fn check_bound(duck_db: &Path, bq_db: &Path, entry: &TargetDivergence) -> Result<(), String> {
    let conn = attached_conn(duck_db, bq_db);
    match &entry.bound {
        DivergenceBound::MonotoneDivergence {
            key_col,
            exact_columns,
            monotone_columns,
            behind_side,
        } => {
            let r = entry.relation;
            let key_only_duck = scalar_on(
                &conn,
                &format!(
                    "SELECT count(*) FROM duck_db.main.{r} d WHERE NOT EXISTS \
                     (SELECT 1 FROM bq_db.main.{r} b WHERE b.{key_col} = d.{key_col})"
                ),
            );
            let key_only_bq = scalar_on(
                &conn,
                &format!(
                    "SELECT count(*) FROM bq_db.main.{r} b WHERE NOT EXISTS \
                     (SELECT 1 FROM duck_db.main.{r} d WHERE d.{key_col} = b.{key_col})"
                ),
            );
            if key_only_duck != 0 || key_only_bq != 0 {
                return Err(format!(
                    "row-key-set mismatch on `{key_col}`: {key_only_duck} duckdb-only, \
                     {key_only_bq} bigquery-only — a divergence bound never licenses a \
                     missing or extra row"
                ));
            }
            for col in *exact_columns {
                let n = scalar_on(
                    &conn,
                    &format!(
                        "SELECT count(*) FROM duck_db.main.{r} d \
                         JOIN bq_db.main.{r} b USING ({key_col}) \
                         WHERE d.{col} IS DISTINCT FROM b.{col}"
                    ),
                );
                if n != 0 {
                    return Err(format!(
                        "{n} row(s) differ on `{col}`, which this bound does not license \
                         to diverge"
                    ));
                }
            }
            // `behind_side` names the target never allowed to *lead*, so a
            // violation is a row where that target's value is strictly ahead.
            let (behind, other, predicate) = match behind_side {
                BehindSide::Bigquery => ("bigquery", "duckdb", "b.{col} > d.{col}"),
                BehindSide::Duckdb => ("duckdb", "bigquery", "d.{col} > b.{col}"),
            };
            for col in *monotone_columns {
                let predicate = predicate.replace("{col}", col);
                let n = scalar_on(
                    &conn,
                    &format!(
                        "SELECT count(*) FROM duck_db.main.{r} d \
                         JOIN bq_db.main.{r} b USING ({key_col}) \
                         WHERE {predicate}"
                    ),
                );
                if n != 0 {
                    return Err(format!(
                        "{n} row(s) have the {behind} leg's `{col}` ahead of the {other} \
                         leg's — expected {behind} to never lead on a monotone column"
                    ));
                }
            }
            Ok(())
        }
    }
}

/// The registry-consulting sweep: for every compared relation, either a
/// registered bound holds or the multiset difference is zero in both
/// directions.
fn check_targets_agree(
    duck_db: &Path,
    bq_db: &Path,
    window_label: &str,
) -> Result<Vec<RelationDiff>, String> {
    let diffs = compare_databases(duck_db, bq_db)
        .map_err(|e| format!("window {window_label}: coverage failure: {e}"))?;
    for diff in &diffs {
        if let Some(entry) = TARGET_DIVERGENCE_REGISTRY
            .iter()
            .find(|e| e.relation == diff.relation)
        {
            if let Err(msg) = check_bound(duck_db, bq_db, entry) {
                return Err(format!(
                    "window {window_label}: registered divergence bound violated for `{}` \
                     (registered reason: {}): {msg}",
                    entry.relation, entry.reason
                ));
            }
        } else if diff.duck_only != 0 || diff.bq_only != 0 {
            return Err(format!(
                "window {window_label}: unregistered divergence in `{}` \
                 (duck_only={}, bq_only={})\nduckdb-only sample: {:?}\n\
                 bigquery-only sample: {:?}",
                diff.relation,
                diff.duck_only,
                diff.bq_only,
                diff.duck_only_sample,
                diff.bq_only_sample
            ));
        }
    }
    Ok(diffs)
}

// ---------------------------------------------------------------------------
// The landing seam: exported BigQuery rows -> a scratch DuckDB database
// ---------------------------------------------------------------------------

/// Build `out_db` holding one table per compared relation of `duck_db`,
/// populated from `<ndjson_dir>/<relation>.ndjson`.
///
/// Each table is created as `SELECT * FROM duck.<relation> WHERE false`, so it
/// carries **the DuckDB leg's own declared column names and types** — that is
/// the whole of the declared normalisation (see the module doc comment). Each
/// JSON field is then cast into that column: a TIMESTAMP column from the
/// exporter's epoch-seconds float via `make_timestamp` over microseconds (so
/// the conversion is exact and free of any session timezone — `to_timestamp`
/// would produce a TIMESTAMPTZ and re-interpret it locally), everything else
/// by a plain `CAST` of the extracted text.
///
/// A value that will not cast raises here rather than being coerced: a
/// BigQuery value the DuckDB-side type cannot hold is a finding about the two
/// targets, not something for the comparator to absorb.
fn load_bigquery_snapshot(duck_db: &Path, ndjson_dir: &Path, out_db: &Path) {
    if out_db.exists() {
        std::fs::remove_file(out_db).unwrap_or_else(|e| panic!("remove {out_db:?}: {e}"));
    }
    let conn = duckdb::Connection::open(out_db).unwrap_or_else(|e| panic!("open {out_db:?}: {e}"));
    conn.execute_batch(&format!(
        "ATTACH '{}' AS duck_db (READ_ONLY);",
        duck_db.display()
    ))
    .expect("attach the duckdb leg");

    for relation in discover_relations(duck_db) {
        let columns = relation_columns(&conn, &relation);
        conn.execute_batch(&format!(
            "CREATE TABLE main.{relation} AS \
             SELECT * FROM duck_db.main.{relation} WHERE false;"
        ))
        .unwrap_or_else(|e| panic!("create scratch table {relation}: {e}"));

        let file = ndjson_dir.join(format!("{relation}.ndjson"));
        let len = std::fs::metadata(&file)
            .unwrap_or_else(|e| panic!("stat {file:?}: {e}"))
            .len();
        if len == 0 {
            continue;
        }
        let projection: Vec<String> = columns
            .iter()
            .map(|(name, ty)| {
                let extracted = format!("json_extract_string(j.json, '$.\"{name}\"')");
                if ty.starts_with("TIMESTAMP") {
                    format!(
                        "make_timestamp(CAST(round(CAST({extracted} AS DOUBLE) * 1000000) \
                         AS BIGINT)) AS {name}"
                    )
                } else {
                    format!("CAST({extracted} AS {ty}) AS {name}")
                }
            })
            .collect();
        let sql = format!(
            "INSERT INTO main.{relation} SELECT {} FROM read_json_objects('{}') AS j",
            projection.join(", "),
            file.display()
        );
        conn.execute_batch(&sql)
            .unwrap_or_else(|e| panic!("load {relation} from {file:?}: {e}\nSQL:\n{sql}"));
    }
    conn.execute_batch("DETACH duck_db;").expect("detach");
}

fn relation_columns(conn: &duckdb::Connection, relation: &str) -> Vec<(String, String)> {
    let mut stmt = conn
        .prepare(
            "SELECT column_name, data_type FROM information_schema.columns \
             WHERE table_catalog = 'duck_db' AND table_schema = 'main' AND table_name = ? \
             ORDER BY ordinal_position",
        )
        .expect("prepare column discovery");
    stmt.query_map([relation], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })
    .expect("query column discovery")
    .collect::<Result<_, _>>()
    .expect("collect columns")
}

// ---------------------------------------------------------------------------
// Offline tests — synthetic pairs, no cloud, no credential
// ---------------------------------------------------------------------------

fn synth_db(path: &Path, sql: &str) {
    let conn = duckdb::Connection::open(path).unwrap_or_else(|e| panic!("open {path:?}: {e}"));
    conn.execute_batch(sql)
        .unwrap_or_else(|e| panic!("exec failed: {e}\nSQL:\n{sql}"));
}

/// A synthetic pair agreeing on `gold_repo_dim` except for one row's
/// `current_repo_name`, which the BigQuery leg carries perturbed.
fn perturbed_pair(tmp: &Path) -> (PathBuf, PathBuf) {
    let duck = tmp.join("duck.duckdb");
    let bq = tmp.join("bq.duckdb");
    synth_db(
        &duck,
        "CREATE TABLE main.gold_repo_dim AS SELECT * FROM (VALUES \
           (1, 'a/one', 10), (2, 'a/two', 20), (3, 'a/three', 30)) \
           AS t(repo_id, current_repo_name, event_count);",
    );
    synth_db(
        &bq,
        "CREATE TABLE main.gold_repo_dim AS SELECT * FROM (VALUES \
           (1, 'a/one', 10), (2, 'a/two_renamed', 20), (3, 'a/three', 30)) \
           AS t(repo_id, current_repo_name, event_count);",
    );
    (duck, bq)
}

/// Relation-set totality. A relation present on only one target is a coverage
/// failure naming the relation and the side — never a silently smaller
/// comparison, which is the failure mode a hardcoded model list would have.
#[test]
fn relation_set_mismatch_fails() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let duck = tmp.path().join("duck.duckdb");
    let bq = tmp.path().join("bq.duckdb");
    synth_db(
        &duck,
        "CREATE TABLE main.bronze_events AS SELECT 1 AS id; \
         CREATE TABLE main.marts_star_growth AS SELECT 1 AS id;",
    );
    synth_db(&bq, "CREATE TABLE main.bronze_events AS SELECT 1 AS id;");

    let err = compare_databases(&duck, &bq)
        .expect_err("a relation missing from the BigQuery leg must be a coverage failure");
    assert!(
        err.contains("marts_star_growth"),
        "expected the missing relation to be named: {err}"
    );
    assert!(
        err.contains("bigquery"),
        "expected the side it is missing from to be named: {err}"
    );
}

/// The exclusions are applied on both sides, so bookkeeping asymmetry — which
/// is expected, the two targets realising different state structures — is not
/// mistaken for a coverage failure.
#[test]
fn bookkeeping_relations_are_excluded_on_both_sides() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let duck = tmp.path().join("duck.duckdb");
    let bq = tmp.path().join("bq.duckdb");
    synth_db(
        &duck,
        "CREATE TABLE main.bronze_events AS SELECT 1 AS id; \
         CREATE TABLE main.sources_raw_github_events AS SELECT 1 AS id; \
         CREATE TABLE main._loader_days AS SELECT DATE '2026-08-05' AS day;",
    );
    synth_db(
        &bq,
        "CREATE TABLE main.bronze_events AS SELECT 1 AS id; \
         CREATE TABLE main.github_events AS SELECT 1 AS id; \
         CREATE TABLE main.github_events_arrival AS SELECT 1 AS id; \
         CREATE TABLE main._smelt_ledger AS SELECT 1 AS id; \
         CREATE TABLE main.silver_repo_naming__tombstones AS SELECT 1 AS id;",
    );

    let diffs = compare_databases(&duck, &bq).expect("bookkeeping tables are excluded");
    let names: Vec<&str> = diffs.iter().map(|d| d.relation.as_str()).collect();
    assert_eq!(names, vec!["bronze_events"]);
}

/// Negative control on the comparator: a genuine value difference is reported
/// in both directions, and fails the sweep naming the relation and both
/// counts.
#[test]
fn an_unregistered_target_divergence_fails() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (duck, bq) = perturbed_pair(tmp.path());

    let diffs = compare_databases(&duck, &bq).expect("coverage matches");
    let d = diffs
        .iter()
        .find(|d| d.relation == "gold_repo_dim")
        .expect("gold_repo_dim present in the diff set");
    assert_eq!(
        (d.duck_only, d.bq_only),
        (1, 1),
        "expected the perturbed row on both sides of the multiset difference"
    );

    let err = check_targets_agree(&duck, &bq, "synthetic")
        .expect_err("an unregistered divergence must fail the sweep");
    assert!(
        err.contains("gold_repo_dim"),
        "expected the relation to be named: {err}"
    );
    assert!(
        err.contains("duck_only=1") && err.contains("bq_only=1"),
        "expected both counts to be named: {err}"
    );
}

/// The sweep fails closed on an empty registry. An empty
/// [`TARGET_DIVERGENCE_REGISTRY`] plus a vacuous sweep is indistinguishable
/// from success, so the registry-consulting path itself is driven over a real
/// mismatch rather than left to be inferred from the registry's silence.
#[test]
fn the_sweep_fails_closed_on_an_empty_registry() {
    assert!(
        TARGET_DIVERGENCE_REGISTRY.is_empty(),
        "this test's assertions assume an empty registry — see the \
         TARGET_DIVERGENCE_REGISTRY doc comment for what changes once an entry is added"
    );
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (duck, bq) = perturbed_pair(tmp.path());

    let err = check_targets_agree(&duck, &bq, "synthetic")
        .expect_err("an empty registry must not make the sweep vacuous");
    assert!(
        err.contains("unregistered divergence"),
        "expected the unregistered-divergence branch, not a registered bound: {err}"
    );
}

/// A registered bound is checkable in both directions: it holds when the
/// target it licenses to be behind is the one that is in fact behind, and it
/// is rejected — naming the column — when that target leads, or when the
/// diverging column is only licensed by exact match. Keeps both
/// [`BehindSide`] variants and every leg of [`check_bound`] live while the
/// registry is empty.
#[test]
fn a_monotone_bound_holds_and_rejects_the_leading_side() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let duck = tmp.path().join("duck.duckdb");
    let bq = tmp.path().join("bq.duckdb");
    synth_db(
        &duck,
        "CREATE TABLE main.gold_repo_activity_daily AS SELECT * FROM (VALUES \
           (1, 10), (2, 20)) AS t(repo_id, event_count);",
    );
    synth_db(
        &bq,
        "CREATE TABLE main.gold_repo_activity_daily AS SELECT * FROM (VALUES \
           (1, 9), (2, 20)) AS t(repo_id, event_count);",
    );

    let holding = TargetDivergence {
        relation: "gold_repo_activity_daily",
        reason: "test-local: the bigquery leg is genuinely behind on event_count",
        bound: DivergenceBound::MonotoneDivergence {
            key_col: "repo_id",
            exact_columns: &[],
            monotone_columns: &["event_count"],
            behind_side: BehindSide::Bigquery,
        },
    };
    check_bound(&duck, &bq, &holding)
        .expect("the bigquery leg never leads on event_count; the bound should hold");

    let flipped = TargetDivergence {
        relation: "gold_repo_activity_daily",
        reason: "test-local: duckdb wrongly licensed as the behind side",
        bound: DivergenceBound::MonotoneDivergence {
            key_col: "repo_id",
            exact_columns: &[],
            monotone_columns: &["event_count"],
            behind_side: BehindSide::Duckdb,
        },
    };
    let err = check_bound(&duck, &bq, &flipped)
        .expect_err("duckdb leads on event_count; behind_side: Duckdb forbids that");
    assert!(
        err.contains("event_count"),
        "expected the leading column to be named: {err}"
    );

    let unlicensed = TargetDivergence {
        relation: "gold_repo_activity_daily",
        reason: "test-local: event_count wrongly licensed via exact match",
        bound: DivergenceBound::MonotoneDivergence {
            key_col: "repo_id",
            exact_columns: &["event_count"],
            monotone_columns: &[],
            behind_side: BehindSide::Bigquery,
        },
    };
    let err = check_bound(&duck, &bq, &unlicensed)
        .expect_err("exact_columns licenses no divergence at all");
    assert!(
        err.contains("event_count"),
        "expected the unlicensed column to be named: {err}"
    );
}

/// A bound never licenses a missing or extra row, whatever columns it names.
#[test]
fn a_bound_never_licenses_a_missing_row() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let duck = tmp.path().join("duck.duckdb");
    let bq = tmp.path().join("bq.duckdb");
    synth_db(
        &duck,
        "CREATE TABLE main.gold_repo_dim AS SELECT * FROM (VALUES (1, 10), (2, 20)) \
         AS t(repo_id, event_count);",
    );
    synth_db(
        &bq,
        "CREATE TABLE main.gold_repo_dim AS SELECT * FROM (VALUES (1, 10)) \
         AS t(repo_id, event_count);",
    );

    let entry = TargetDivergence {
        relation: "gold_repo_dim",
        reason: "test-local: a bound that names every column must still reject a lost row",
        bound: DivergenceBound::MonotoneDivergence {
            key_col: "repo_id",
            exact_columns: &[],
            monotone_columns: &["event_count"],
            behind_side: BehindSide::Bigquery,
        },
    };
    let err = check_bound(&duck, &bq, &entry)
        .expect_err("a row present on only one target is never inside a bound");
    assert!(
        err.contains("row-key-set mismatch"),
        "expected the missing row to be reported as a key-set mismatch: {err}"
    );
}

/// The exclusion set is exactly the pair GoogleSQL refuses at compile time. A
/// future silent widening — which would shrink the comparison without
/// shrinking any claim made about it — fails here rather than passing
/// quietly.
#[test]
fn excluded_models_are_exactly_the_compile_refused_pair() {
    let actual: BTreeSet<&str> = EXCLUDED_MODELS.iter().copied().collect();
    assert_eq!(
        actual,
        BTreeSet::from(["silver.actor_sessions", "marts.daily_active_contributors"]),
        "the exclusion set must stay exactly the pair GoogleSQL refuses at compile time on \
         the INTERVAL RANGE lookback frame — widening it shrinks criterion 6's comparison"
    );
    assert_eq!(
        EXCLUDED_MODELS.len(),
        actual.len(),
        "EXCLUDED_MODELS lists a model twice"
    );

    // The pair is exactly `silver.actor_sessions` plus its downstream, read
    // from the example project rather than asserted from memory: no other
    // model may reference it, or the exclusion set is incomplete.
    let models_dir = repo_root().join("examples/github_activity/models");
    let mut referencing: BTreeSet<String> = BTreeSet::new();
    for entry in walkdir::WalkDir::new(&models_dir)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("sql") {
            continue;
        }
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
        // `FROM smelt.silver.actor_sessions` — a reference, as opposed to the
        // prose mentions that appear in model comments.
        if text.contains("smelt.silver.actor_sessions") {
            let name = path
                .strip_prefix(&models_dir)
                .expect("under models/")
                .with_extension("")
                .to_string_lossy()
                .replace('/', ".");
            referencing.insert(name);
        }
    }
    assert_eq!(
        referencing,
        BTreeSet::from(["marts.daily_active_contributors".to_string()]),
        "`silver.actor_sessions` has a different downstream set than the exclusion list \
         assumes — excluding it would tear the working set or leave a model uncompared"
    );
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root")
        .to_owned()
}

// ---------------------------------------------------------------------------
// The landing seam, proved offline
// ---------------------------------------------------------------------------

/// A DuckDB-side relation covering every type family the cast rule names, and
/// the exporter's own JSON encoding of the same rows: integers and floats as
/// JSON numbers, booleans as JSON booleans, DATE as a string, TIMESTAMP as
/// epoch seconds with a microsecond fraction (`scripts/bq_dogfood_export.py`).
fn typed_pair(tmp: &Path) -> (PathBuf, PathBuf) {
    let duck = tmp.join("duck.duckdb");
    synth_db(
        &duck,
        "CREATE TABLE main.gold_events_enriched ( \
           id VARCHAR, actor_id BIGINT, score DOUBLE, amount DECIMAL(10, 2), \
           public BOOLEAN, created_at TIMESTAMP, event_date DATE); \
         INSERT INTO main.gold_events_enriched VALUES \
           ('e1', 7, 1.5, 12.34, true,  TIMESTAMP '2026-08-05 01:02:03.000004', DATE '2026-08-05'), \
           ('e2', 8, 2.5, 0.05, false, TIMESTAMP '2026-08-06 23:59:59.999999', DATE '2026-08-06'), \
           ('e3', NULL, NULL, NULL, NULL, NULL, NULL);",
    );
    let ndjson_dir = tmp.join("bq");
    std::fs::create_dir_all(&ndjson_dir).expect("mkdir ndjson dir");
    std::fs::write(
        ndjson_dir.join("gold_events_enriched.ndjson"),
        concat!(
            r#"{"id": "e1", "actor_id": 7, "score": 1.5, "amount": 12.34, "public": true, "created_at": 1785891723.000004, "event_date": "2026-08-05"}"#,
            "\n",
            r#"{"id": "e2", "actor_id": 8, "score": 2.5, "amount": 0.05, "public": false, "created_at": 1786060799.999999, "event_date": "2026-08-06"}"#,
            "\n",
            r#"{"id": "e3", "actor_id": null, "score": null, "amount": null, "public": null, "created_at": null, "event_date": null}"#,
            "\n",
        ),
    )
    .expect("write ndjson");
    (duck, ndjson_dir)
}

/// The landing seam round-trips: exported BigQuery rows land under the DuckDB
/// leg's own declared types, and the comparator then sees zero difference on
/// every type family — including the microsecond edge of a TIMESTAMP and an
/// all-NULL row.
#[test]
fn landing_a_bigquery_snapshot_casts_to_the_duckdb_legs_own_types() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (duck, ndjson_dir) = typed_pair(tmp.path());
    let bq = tmp.path().join("bq.duckdb");
    load_bigquery_snapshot(&duck, &ndjson_dir, &bq);

    let types: Vec<(String, String)> = {
        let conn = attached_conn(&duck, &bq);
        let mut stmt = conn
            .prepare(
                "SELECT column_name, data_type FROM information_schema.columns \
                 WHERE table_catalog = 'bq_db' AND table_name = 'gold_events_enriched' \
                 ORDER BY ordinal_position",
            )
            .expect("prepare");
        stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect")
    };
    assert_eq!(
        types
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join(","),
        "VARCHAR,BIGINT,DOUBLE,DECIMAL(10,2),BOOLEAN,TIMESTAMP,DATE",
        "the landed side must carry the DuckDB leg's own declared types, not JSON's"
    );

    let diffs = check_targets_agree(&duck, &bq, "synthetic").expect("the landed rows must match");
    let d = diffs
        .iter()
        .find(|d| d.relation == "gold_events_enriched")
        .expect("relation compared");
    assert_eq!((d.duck_rows, d.bq_rows), (3, 3));
    assert_eq!((d.duck_only, d.bq_only), (0, 0));
}

/// The seam does not hide a difference either: perturb one landed cell and
/// the sweep reports it. Without this, a broken cast that silently produced
/// NULLs everywhere would look exactly like agreement.
#[test]
fn a_perturbed_landed_cell_is_still_reported() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (duck, ndjson_dir) = typed_pair(tmp.path());
    let file = ndjson_dir.join("gold_events_enriched.ndjson");
    let text = std::fs::read_to_string(&file).expect("read ndjson");
    std::fs::write(
        &file,
        text.replace("1785891723.000004", "1785891723.000005"),
    )
    .expect("write perturbed ndjson");

    let bq = tmp.path().join("bq.duckdb");
    load_bigquery_snapshot(&duck, &ndjson_dir, &bq);
    let err = check_targets_agree(&duck, &bq, "synthetic")
        .expect_err("a one-microsecond difference must not be absorbed by the seam");
    assert!(
        err.contains("gold_events_enriched"),
        "expected the relation to be named: {err}"
    );
}

/// A BigQuery value the DuckDB-side type cannot hold is a loud failure, never
/// a silent NULL or a tolerance: it is a finding about the two targets.
#[test]
#[should_panic(expected = "not-a-number")]
fn a_value_that_will_not_cast_is_a_loud_failure() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (duck, ndjson_dir) = typed_pair(tmp.path());
    let file = ndjson_dir.join("gold_events_enriched.ndjson");
    let text = std::fs::read_to_string(&file).expect("read ndjson");
    std::fs::write(
        &file,
        text.replace("\"actor_id\": 7", "\"actor_id\": \"not-a-number\""),
    )
    .expect("write uncastable ndjson");

    let bq = tmp.path().join("bq.duckdb");
    load_bigquery_snapshot(&duck, &ndjson_dir, &bq);
}
