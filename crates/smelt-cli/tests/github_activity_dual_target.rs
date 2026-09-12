#![cfg(feature = "duckdb")]
//! Dual-target parity for `examples/github_activity/`: does the pipeline
//! compute the same answers on DuckDB and on BigQuery?
//! (`docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md` criterion 6.)
//!
//! **Almost all of this runs per-PR with no credential and no cloud**: the
//! comparator, the relation-set totality check, the divergence registry, the
//! landing seam and the two gates over the committed parity report are all
//! exercised against synthetic local `.duckdb` files, synthetic NDJSON, and
//! the checked-in report artifact. Exactly one test reaches for real
//! snapshots — [`duckdb_and_bigquery_agree_on_every_model`], driven by
//! `scripts/bq-dogfood-parity.sh`, which runs the fixture's thirty windows on
//! both targets and leaves a manifest of per-checkpoint snapshots behind.
//!
//! # The measured result
//!
//! `docs/outcomes/20260906-bigquery-dogfood-spine/phases/13-parity.json` is
//! that sweep's committed output: for every compared checkpoint, every
//! compared relation's row counts on both targets and the multiset difference
//! in both directions. [`TARGET_DIVERGENCE_REGISTRY`] is checked against it in
//! both directions by [`registry_entries_are_all_live`], so a divergence
//! cannot be registered without evidence and evidence cannot be left
//! unregistered.
//!
//! # The comparator and the landing seam
//!
//! Both are shared, not restated: `bq_parity_support` holds the whole-row
//! multiset difference, the generic relation discovery and exclusions, and the
//! typed NDJSON landing step, so this sweep and the BigQuery equivalence sweep
//! (`github_activity_bq_oracle.rs`) make their two different claims with the
//! *same* primitive. Read that module's header for what the comparator does
//! and what the one declared normalisation is.
//!
//! Any tolerance beyond exact value equality — float epsilon, timestamp
//! truncation, string trimming — is a registered [`RegisteredDivergence`] with a
//! root-caused reason, never a quiet comparator setting. There is no
//! comparator knob to loosen.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[path = "bq_parity_support/mod.rs"]
mod bq_parity_support;
use bq_parity_support::{
    attached_conn, check_agreement_against, check_bound, compare_databases, load_bigquery_snapshot,
    repo_root, DivergenceBound, RegisteredDivergence, RelationDiff, Side, SideLabels,
    EXCLUDED_MODELS,
};

/// The two sides of *this* sweep, for failure messages.
const SIDES: SideLabels = SideLabels {
    left: "duckdb",
    right: "bigquery",
};

// ---------------------------------------------------------------------------
// The divergence registry for THIS sweep
// ---------------------------------------------------------------------------
const TARGET_DIVERGENCE_REGISTRY: &[RegisteredDivergence] = &[RegisteredDivergence {
    relation: "bronze_events",
    // Root cause, not a shrug. `bronze.events` is a whole-source passthrough
    // (`materialization: table`, no incremental strategy), so each window
    // rebuilds it from whatever the source holds *at that moment*. The two legs'
    // sources do not arrive the same way: BigQuery's `smelt_dogfood.github_events`
    // is fully populated before the first window, while the DuckDB leg's
    // `load_day.sh` appends day D at window D. So at window w the BigQuery side
    // holds all thirty days and the DuckDB side holds w of them — identical
    // behaviour over different input, and the gap closes monotonically
    // (62,382 -> 59,605 -> 57,217 -> 50,406 -> 32,251 -> 11,536 -> 0) to exact
    // agreement at the final window.
    //
    // Attributed, not assumed: replaying the DuckDB leg with the source staged
    // up front (`run_incremental.py --preload-source`) removes the difference at
    // every checkpoint including the first, which is
    // `13-parity-attribution.json` and is gated by
    // `controlling_arrival_order_removes_every_divergence`. No other relation
    // diverges on either run — every window-addressed and keyed model is
    // window-limited on both targets.
    reason: "arrival order: `bronze.events` is a whole-source rebuild, and the BigQuery              leg's source is fully populated before window 1 while the DuckDB leg's grows              a day per window. Removed entirely by the --preload-source attribution run              (13-parity-attribution.json); zero at the final window on both runs.",
    bound: DivergenceBound::ArrivalLag {
        event_time_column: "created_at",
        behind_side: Side::Left,
    },
}];

/// [`check_agreement_against`] over this suite's own registry and side labels.
fn check_targets_agree(
    duck_db: &Path,
    bq_db: &Path,
    window_label: &str,
) -> Result<Vec<RelationDiff>, String> {
    check_agreement_against(
        duck_db,
        bq_db,
        window_label,
        TARGET_DIVERGENCE_REGISTRY,
        SIDES,
    )
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

    let err = compare_databases(&duck, &bq, SIDES)
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

    let diffs = compare_databases(&duck, &bq, SIDES).expect("bookkeeping tables are excluded");
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

    let diffs = compare_databases(&duck, &bq, SIDES).expect("coverage matches");
    let d = diffs
        .iter()
        .find(|d| d.relation == "gold_repo_dim")
        .expect("gold_repo_dim present in the diff set");
    assert_eq!(
        (d.left_only, d.right_only),
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
        err.contains("duckdb_only=1") && err.contains("bigquery_only=1"),
        "expected both counts to be named: {err}"
    );
}

/// The sweep fails closed on an empty registry. An empty registry plus a
/// vacuous sweep is indistinguishable from success, so the registry-consulting
/// path itself is driven over a real mismatch with the registry emptied — the
/// control holds whatever [`TARGET_DIVERGENCE_REGISTRY`] happens to contain, so
/// adding or removing an entry can never quietly retire it.
#[test]
fn the_sweep_fails_closed_on_an_empty_registry() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (duck, bq) = perturbed_pair(tmp.path());

    let err = check_agreement_against(&duck, &bq, "synthetic", &[], SIDES)
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
/// [`Side`] variants and every leg of [`check_bound`] live while the
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

    let holding = RegisteredDivergence {
        relation: "gold_repo_activity_daily",
        reason: "test-local: the bigquery leg is genuinely behind on event_count",
        bound: DivergenceBound::MonotoneDivergence {
            key_col: "repo_id",
            exact_columns: &[],
            monotone_columns: &["event_count"],
            behind_side: Side::Right,
        },
    };
    check_bound(&duck, &bq, &holding, SIDES)
        .expect("the bigquery leg never leads on event_count; the bound should hold");

    let flipped = RegisteredDivergence {
        relation: "gold_repo_activity_daily",
        reason: "test-local: duckdb wrongly licensed as the behind side",
        bound: DivergenceBound::MonotoneDivergence {
            key_col: "repo_id",
            exact_columns: &[],
            monotone_columns: &["event_count"],
            behind_side: Side::Left,
        },
    };
    let err = check_bound(&duck, &bq, &flipped, SIDES)
        .expect_err("duckdb leads on event_count; behind_side: Duckdb forbids that");
    assert!(
        err.contains("event_count"),
        "expected the leading column to be named: {err}"
    );

    let unlicensed = RegisteredDivergence {
        relation: "gold_repo_activity_daily",
        reason: "test-local: event_count wrongly licensed via exact match",
        bound: DivergenceBound::MonotoneDivergence {
            key_col: "repo_id",
            exact_columns: &["event_count"],
            monotone_columns: &[],
            behind_side: Side::Right,
        },
    };
    let err = check_bound(&duck, &bq, &unlicensed, SIDES)
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

    let entry = RegisteredDivergence {
        relation: "gold_repo_dim",
        reason: "test-local: a bound that names every column must still reject a lost row",
        bound: DivergenceBound::MonotoneDivergence {
            key_col: "repo_id",
            exact_columns: &[],
            monotone_columns: &["event_count"],
            behind_side: Side::Right,
        },
    };
    let err = check_bound(&duck, &bq, &entry, SIDES)
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
                 WHERE table_catalog = 'right_db' AND table_name = 'gold_events_enriched' \
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
    assert_eq!((d.left_rows, d.right_rows), (3, 3));
    assert_eq!((d.left_only, d.right_only), (0, 0));
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

// ---------------------------------------------------------------------------
// The committed parity report, and the two-sided liveness ratchet over it
// ---------------------------------------------------------------------------

/// The measured result of the live sweep, committed so the registry has
/// something to be checked against per-PR. Written by
/// [`duckdb_and_bigquery_agree_on_every_model`]; read here.
const PARITY_REPORT_PATH: &str =
    "docs/outcomes/20260906-bigquery-dogfood-spine/phases/13-parity.json";

/// The measured result of the live sweep, as committed.
fn parity_report() -> serde_json::Value {
    let path = repo_root().join(PARITY_REPORT_PATH);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read the committed parity report {path:?}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path:?}: {e}"))
}

fn report_checkpoints(report: &serde_json::Value) -> Vec<&serde_json::Value> {
    report["checkpoints"]
        .as_array()
        .expect("the report carries a `checkpoints` array")
        .iter()
        .collect()
}

/// Relations the report shows diverging at *any* compared checkpoint.
fn divergent_relations(report: &serde_json::Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for cp in report_checkpoints(report) {
        for rel in cp["relations"].as_array().expect("relations array") {
            let duck_only = rel["duck_only"].as_i64().expect("duck_only");
            let bq_only = rel["bq_only"].as_i64().expect("bq_only");
            if duck_only != 0 || bq_only != 0 {
                out.insert(rel["relation"].as_str().expect("relation name").to_string());
            }
        }
    }
    out
}

/// The report is total: the same relation set at every compared checkpoint,
/// one row per model on both targets, and no blank cell. A report that quietly
/// covered twelve relations at one checkpoint and fourteen at another would
/// make the registry ratchet below vacuous for the two it dropped.
#[test]
fn the_parity_report_covers_every_model_on_both_targets() {
    let report = parity_report();
    let checkpoints = report_checkpoints(&report);
    assert!(
        !checkpoints.is_empty(),
        "the parity report compares no checkpoint at all"
    );

    let windows: Vec<i64> = checkpoints
        .iter()
        .map(|cp| cp["window"].as_i64().expect("window number"))
        .collect();
    let days = report["schedule"]["days"]
        .as_i64()
        .expect("the schedule declares its length");
    assert_eq!(
        windows.last().copied(),
        Some(days),
        "the final window is compared in full, always — that is the end state \
         criterion 6 is about"
    );

    let mut expected: Option<BTreeSet<String>> = None;
    for cp in &checkpoints {
        let label = cp["label"].as_str().expect("checkpoint label");
        let mut names = BTreeSet::new();
        for rel in cp["relations"].as_array().expect("relations array") {
            let name = rel["relation"].as_str().expect("relation name").to_string();
            for cell in ["duck_rows", "bq_rows", "duck_only", "bq_only"] {
                assert!(
                    rel[cell].as_i64().is_some(),
                    "checkpoint {label}, relation {name}: `{cell}` is blank"
                );
            }
            assert!(
                names.insert(name.clone()),
                "checkpoint {label} lists `{name}` twice"
            );
        }
        match &expected {
            None => expected = Some(names),
            Some(first) => assert_eq!(
                &names, first,
                "checkpoint {label} compares a different relation set than the first \
                 checkpoint — the comparison silently narrowed"
            ),
        }
    }

    let names = expected.expect("at least one checkpoint");
    assert_eq!(
        names.len(),
        14,
        "expected the 14 models that run on both targets (16 less the \
         compile-refused pair), got: {names:?}"
    );
    for excluded in EXCLUDED_MODELS {
        let physical = excluded.replace('.', "_");
        assert!(
            !names.contains(&physical),
            "`{excluded}` is refused at compile time on GoogleSQL and must not appear \
             in the parity report"
        );
    }
}

/// The two-sided liveness ratchet. An entry naming a relation the report shows
/// agreeing is stale and must be deleted; a relation the report shows diverging
/// with no entry is an unregistered divergence. Mirrors
/// `github_activity_oracle.rs::registry_entries_are_all_live`.
#[test]
fn registry_entries_are_all_live() {
    let report = parity_report();
    let measured = divergent_relations(&report);
    let registered: BTreeSet<String> = TARGET_DIVERGENCE_REGISTRY
        .iter()
        .map(|e| e.relation.to_string())
        .collect();

    let stale: Vec<&String> = registered.difference(&measured).collect();
    assert!(
        stale.is_empty(),
        "TARGET_DIVERGENCE_REGISTRY entries name relations the committed parity report \
         shows agreeing on every compared checkpoint — delete them: {stale:?}"
    );
    let unregistered: Vec<&String> = measured.difference(&registered).collect();
    assert!(
        unregistered.is_empty(),
        "the committed parity report shows these relations diverging with no registry \
         entry — register each with a root-caused reason and a checkable bound: \
         {unregistered:?}"
    );
}

// ---------------------------------------------------------------------------
// The live sweep
// ---------------------------------------------------------------------------

/// One compared checkpoint: the DuckDB leg's database file at that window, and
/// the directory of per-relation NDJSON the BigQuery leg exported at the same
/// window. Produced by `scripts/bq-dogfood-parity.sh manifest`.
#[derive(serde::Deserialize)]
struct Checkpoint {
    label: String,
    window: i64,
    day: String,
    duck_db_path: PathBuf,
    ndjson_dir: PathBuf,
}

#[derive(serde::Deserialize)]
struct ParityManifest {
    checkpoints: Vec<Checkpoint>,
}

/// The whole sweep over the live snapshots, and the writer of the committed
/// parity report.
///
/// Gating, per the plan: with `SMELT_BQ_DOGFOOD_LIVE=1` set this **fails**
/// rather than skipping when the snapshots are absent — a live gate that goes
/// green because nothing was there is worse than no gate. With it unset the
/// test skips, because the snapshots are hundreds of megabytes of exported rows
/// that no per-PR run produces.
#[test]
fn duckdb_and_bigquery_agree_on_every_model() {
    if std::env::var("SMELT_BQ_DOGFOOD_LIVE").as_deref() != Ok("1") {
        eprintln!(
            "SMELT_BQ_DOGFOOD_LIVE is not 1 — skipping the live sweep. Produce the \
             snapshots with scripts/bq-dogfood-parity.sh."
        );
        return;
    }
    let manifest_path = std::env::var("PARITY_MANIFEST")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_root().join("target/phase13/parity-manifest.json"));
    let manifest: ParityManifest = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path).unwrap_or_else(|e| {
            panic!("SMELT_BQ_DOGFOOD_LIVE=1 but no parity manifest at {manifest_path:?}: {e}")
        }),
    )
    .unwrap_or_else(|e| panic!("parse {manifest_path:?}: {e}"));
    assert!(
        !manifest.checkpoints.is_empty(),
        "{manifest_path:?} declares no checkpoint — the sweep would pass vacuously"
    );

    let scratch = tempfile::TempDir::new().expect("tempdir");
    let mut checkpoints_json = Vec::new();
    let mut failures = Vec::new();

    for cp in &manifest.checkpoints {
        assert!(
            cp.duck_db_path.exists(),
            "checkpoint {}: no DuckDB snapshot at {:?}",
            cp.label,
            cp.duck_db_path
        );
        assert!(
            cp.ndjson_dir.is_dir(),
            "checkpoint {}: no BigQuery snapshot directory at {:?}",
            cp.label,
            cp.ndjson_dir
        );
        let landed = scratch.path().join(format!("{}.duckdb", cp.label));
        load_bigquery_snapshot(&cp.duck_db_path, &cp.ndjson_dir, &landed);

        let diffs = match check_targets_agree(&cp.duck_db_path, &landed, &cp.label) {
            Ok(diffs) => diffs,
            Err(msg) => {
                failures.push(msg);
                // Still record the raw numbers, so the report names what
                // diverged rather than merely that something did.
                compare_databases(&cp.duck_db_path, &landed, SIDES)
                    .unwrap_or_else(|e| panic!("checkpoint {}: {e}", cp.label))
            }
        };
        checkpoints_json.push(serde_json::json!({
            "label": cp.label,
            "window": cp.window,
            "day": cp.day,
            "relations": diffs.iter().map(|d| serde_json::json!({
                "relation": d.relation,
                "duck_rows": d.left_rows,
                "bq_rows": d.right_rows,
                "duck_only": d.left_only,
                "bq_only": d.right_only,
            })).collect::<Vec<_>>(),
        }));
    }

    let report = serde_json::json!({
        "schedule": {
            "start_date": "2026-08-05",
            "days": 30,
            "checkpoints": manifest.checkpoints.iter().map(|c| c.window).collect::<Vec<_>>(),
        },
        "excluded_models": EXCLUDED_MODELS,
        "checkpoints": checkpoints_json,
    });
    // `PARITY_REPORT_OUT` exists for the D2 attribution run: the same sweep is
    // run a second time against the DuckDB leg replayed over a pre-staged
    // source, and its result is a separate artifact rather than an overwrite of
    // the natural pair's.
    let out = std::env::var("PARITY_REPORT_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_root().join(PARITY_REPORT_PATH));
    std::fs::write(
        &out,
        serde_json::to_string_pretty(&report).expect("serialise the parity report") + "\n",
    )
    .unwrap_or_else(|e| panic!("write {out:?}: {e}"));
    eprintln!("wrote {out:?}");

    assert!(
        failures.is_empty(),
        "the two targets disagree:\n{}",
        failures.join("\n\n")
    );
}

/// The `ArrivalLag` bound holds for the shape it is registered against — the
/// lagging leg is the leading leg's not-yet-arrived tail short — and rejects
/// both ways it could be abused: a row the lagging leg holds and the leading
/// leg does not, and a row missing from *inside* the range the lagging leg has
/// already loaded. The second is the one that matters: without it the bound
/// would license a genuine row-loss bug in `bronze_events`.
#[test]
fn an_arrival_lag_bound_rejects_a_lost_row_inside_the_loaded_range() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let entry = |reason| RegisteredDivergence {
        relation: "bronze_events",
        reason,
        bound: DivergenceBound::ArrivalLag {
            event_time_column: "created_at",
            behind_side: Side::Left,
        },
    };

    // The real shape: DuckDB has loaded through 2026-08-06, BigQuery holds the
    // whole three-day source.
    let duck = tmp.path().join("lag_duck.duckdb");
    let bq = tmp.path().join("lag_bq.duckdb");
    synth_db(
        &duck,
        "CREATE TABLE main.bronze_events AS SELECT * FROM (VALUES \
           ('a', TIMESTAMP '2026-08-05 01:00:00'), ('b', TIMESTAMP '2026-08-06 01:00:00')) \
           AS t(id, created_at);",
    );
    synth_db(
        &bq,
        "CREATE TABLE main.bronze_events AS SELECT * FROM (VALUES \
           ('a', TIMESTAMP '2026-08-05 01:00:00'), ('b', TIMESTAMP '2026-08-06 01:00:00'), \
           ('c', TIMESTAMP '2026-08-07 01:00:00')) AS t(id, created_at);",
    );
    check_bound(
        &duck,
        &bq,
        &entry("test-local: the tail has not arrived yet"),
        SIDES,
    )
    .expect("a pure arrival lag is exactly what this bound licenses");

    // A row lost from inside the range DuckDB has already loaded. Same subset
    // relation, same direction — and it must still be refused.
    let duck_lost = tmp.path().join("lag_duck_lost.duckdb");
    synth_db(
        &duck_lost,
        "CREATE TABLE main.bronze_events AS SELECT * FROM (VALUES \
           ('b', TIMESTAMP '2026-08-06 01:00:00')) AS t(id, created_at);",
    );
    let err = check_bound(
        &duck_lost,
        &bq,
        &entry("test-local: a lost row must not hide behind an arrival lag"),
        SIDES,
    )
    .expect_err("a row missing from inside the loaded range is a lost row, not a lag");
    assert!(
        err.contains("lost row"),
        "expected the failure to name it as a lost row: {err}"
    );

    // A row the lagging leg holds and the leading leg does not.
    let duck_extra = tmp.path().join("lag_duck_extra.duckdb");
    synth_db(
        &duck_extra,
        "CREATE TABLE main.bronze_events AS SELECT * FROM (VALUES \
           ('a', TIMESTAMP '2026-08-05 01:00:00'), ('b', TIMESTAMP '2026-08-06 01:00:00'), \
           ('z', TIMESTAMP '2026-08-06 02:00:00')) AS t(id, created_at);",
    );
    let err = check_bound(
        &duck_extra,
        &bq,
        &entry("test-local: the lagging side may never hold an extra row"),
        SIDES,
    )
    .expect_err("an arrival lag never adds rows to the lagging side");
    assert!(
        err.contains("only on the duckdb leg"),
        "expected the extra row on the lagging side to be named: {err}"
    );
}

/// The attribution run (`13-parity.md` §"Arrival-order attribution"): with the
/// DuckDB leg replayed over a source staged up front — the same arrival order
/// BigQuery's warehouse-resident source has — **no** relation diverges at any
/// checkpoint. This is what makes the one registered divergence a statement
/// about arrival order rather than about either engine, and it is gated here
/// rather than asserted in prose.
#[test]
fn controlling_arrival_order_removes_every_divergence() {
    let path = repo_root()
        .join("docs/outcomes/20260906-bigquery-dogfood-spine/phases/13-parity-attribution.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read the attribution report {path:?}: {e}"));
    let report: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path:?}: {e}"));

    let checkpoints = report_checkpoints(&report);
    assert_eq!(
        checkpoints.len(),
        report_checkpoints(&parity_report()).len(),
        "the attribution run must cover the same checkpoints as the natural pair, or it \
         attributes less than it claims"
    );
    let divergent = divergent_relations(&report);
    assert!(
        divergent.is_empty(),
        "the arrival-order attribution run still shows divergence in {divergent:?} — those \
         differences are NOT attributable to arrival order and belong to \
         20260906-bigquery-correctness"
    );
}

/// Criterion 6's core claim, gated against the committed report rather than
/// left to the summary's prose: at the final window the two targets agree
/// exactly on every compared relation, with no registered divergence standing.
#[test]
fn the_two_targets_agree_at_the_final_window() {
    let report = parity_report();
    let checkpoints = report_checkpoints(&report);
    let last = checkpoints.last().expect("at least one checkpoint");
    let label = last["label"].as_str().expect("label");
    for rel in last["relations"].as_array().expect("relations array") {
        let name = rel["relation"].as_str().expect("relation name");
        assert_eq!(
            (
                rel["duck_only"].as_i64().expect("duck_only"),
                rel["bq_only"].as_i64().expect("bq_only")
            ),
            (0, 0),
            "final window {label}: `{name}` differs between the targets"
        );
    }
}
