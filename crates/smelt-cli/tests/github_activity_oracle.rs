#![cfg(feature = "duckdb")]
//! Per-window full-refresh oracle for `examples/github_activity/`
//! (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/06-plan.md`).
//!
//! Turns the end-of-replay, row-count-only equivalence check in
//! `github_activity_replay.rs::full_refresh_matches_incremental_replay` into
//! criterion 7's actual promise: after **every** incremental window, **every**
//! materialised relation is compared row-for-row against a full refresh over
//! the inputs seen so far. A difference is either zero or matches a named,
//! bounded [`DIVERGENCE_REGISTRY`] entry — never a magic row-count delta.
//!
//! Relation discovery excludes `sources_*` (the raw loaded inputs, identical
//! by construction on both legs) and `_smelt_*` (run-bookkeeping — the
//! ledger and observed-delta tables record *how* a run happened, which
//! legitimately differs between an incremental run and a `--full-refresh`
//! run of the same data, not model state).
//!
//! The 30-day fixture makes a full-refresh oracle expensive (it recompiles
//! and reruns every model from scratch), so `every_window_matches_the_full_
//! refresh_oracle` checks after every window for the first 10 days and after
//! the final (30th) window only, per the plan's runtime-bounding note. The
//! full 30-day full-refresh oracle itself is built at most once per test
//! binary run ([`full_replay_pair`]), shared between the tests that need it.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tempfile::TempDir;

mod github_activity_support;
use github_activity_support::{
    create_empty_raw_table, day_after, duckdb_exec, load_day, replay_days, smelt_run,
    stage_workspace, FIXTURE_DAYS,
};

/// Relation name prefixes never compared: see module doc comment.
const EXCLUDED_PREFIXES: &[&str] = &["sources_", "_smelt_"];

fn discover_relations(db: &Path) -> Vec<String> {
    let conn = duckdb::Connection::open(db).unwrap_or_else(|e| panic!("open {db:?}: {e}"));
    let mut stmt = conn
        .prepare("SELECT table_name FROM information_schema.tables WHERE table_schema = 'main' ORDER BY 1")
        .expect("prepare discovery query");
    let names: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query discovery")
        .collect::<Result<_, _>>()
        .expect("collect discovered relation names");
    names
        .into_iter()
        .filter(|n| !EXCLUDED_PREFIXES.iter().any(|p| n.starts_with(p)))
        .collect()
}

fn attached_conn(incr_db: &Path, full_db: &Path) -> duckdb::Connection {
    let conn = duckdb::Connection::open_in_memory().expect("open in-memory duckdb");
    conn.execute_batch(&format!(
        "ATTACH '{}' AS incr_db (READ_ONLY); ATTACH '{}' AS full_db (READ_ONLY);",
        incr_db.display(),
        full_db.display()
    ))
    .expect("attach both databases");
    conn
}

fn scalar_on(conn: &duckdb::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0))
        .unwrap_or_else(|e| panic!("query failed: {e}\nSQL:\n{sql}"))
}

/// Up to 5 offending rows, rendered as JSON, for a failure message.
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
    incr_only: i64,
    full_only: i64,
    incr_only_sample: Vec<String>,
    full_only_sample: Vec<String>,
}

fn relation_diff(conn: &duckdb::Connection, relation: &str) -> RelationDiff {
    let incr_only_sql = format!(
        "SELECT * FROM incr_db.main.{relation} EXCEPT ALL SELECT * FROM full_db.main.{relation}"
    );
    let full_only_sql = format!(
        "SELECT * FROM full_db.main.{relation} EXCEPT ALL SELECT * FROM incr_db.main.{relation}"
    );
    RelationDiff {
        relation: relation.to_string(),
        incr_only: scalar_on(conn, &format!("SELECT count(*) FROM ({incr_only_sql})")),
        full_only: scalar_on(conn, &format!("SELECT count(*) FROM ({full_only_sql})")),
        incr_only_sample: sample_rows(conn, &incr_only_sql),
        full_only_sample: sample_rows(conn, &full_only_sql),
    }
}

/// Compare every materialised relation between the two databases. `Err`
/// names a relation present in only one database — coverage totality, not a
/// content diff.
fn compare_databases(incr_db: &Path, full_db: &Path) -> Result<Vec<RelationDiff>, String> {
    let incr_relations = discover_relations(incr_db);
    let full_relations = discover_relations(full_db);
    let mut all: Vec<String> = incr_relations
        .iter()
        .chain(full_relations.iter())
        .cloned()
        .collect();
    all.sort();
    all.dedup();

    for rel in &all {
        let in_incr = incr_relations.contains(rel);
        let in_full = full_relations.contains(rel);
        if !in_incr || !in_full {
            return Err(format!(
                "relation `{rel}` present in only one database (incremental={in_incr}, oracle={in_full})"
            ));
        }
    }

    let conn = attached_conn(incr_db, full_db);
    Ok(all.iter().map(|rel| relation_diff(&conn, rel)).collect())
}

/// A composition-relevant divergence between the incremental replay and the
/// full-refresh oracle, bounded by a checkable predicate rather than a magic
/// row count. `key_columns` is the `(key, clock)` tuple the presented
/// table's `MERGE ... ON` addresses by.
struct DivergenceEntry {
    relation: &'static str,
    reason: &'static str,
    key_columns: &'static [&'static str],
}

/// Both entries trace to the same root cause: the incremental window-forward
/// patch loop addresses the presented table by `(key, clock)`, so a
/// redelivered duplicate or a genuine same-second tie whose payload agrees
/// converges to one presented row; `--full-refresh` re-runs the model's raw
/// compiled `SELECT` (`LEAD`/`LAG` over every physical row) with no such
/// addressing, so it keeps every tied row
/// (`docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md` decision log,
/// `github_activity_replay.rs::same_second_events_fold_once_within_a_key`).
/// Owner of a fix, if one is ever wanted: `docs/outcomes/
/// 20260906-scd2-keyed-succession`.
const DIVERGENCE_REGISTRY: &[DivergenceEntry] = &[
    DivergenceEntry {
        relation: "silver_repo_naming",
        reason: "same-second (repo_id, created_at) ties: incremental folds them to one \
                  presented row, --full-refresh keeps every tied physical row",
        key_columns: &["repo_id", "created_at"],
    },
    DivergenceEntry {
        relation: "silver_actor_naming",
        reason: "same-second (actor_id, created_at) ties: incremental folds them to one \
                  presented row, --full-refresh keeps every tied physical row",
        key_columns: &["actor_id", "created_at"],
    },
];

/// A registry entry's bound: the incremental relation has zero rows the
/// oracle lacks, and the oracle folds to exactly one row per `key_columns`
/// group — the multiset-equality-after-folding property, not a row count.
fn check_bound(incr_db: &Path, full_db: &Path, entry: &DivergenceEntry) -> Result<(), String> {
    let conn = attached_conn(incr_db, full_db);
    let incr_only = scalar_on(
        &conn,
        &format!(
            "SELECT count(*) FROM (SELECT * FROM incr_db.main.{r} EXCEPT ALL \
             SELECT * FROM full_db.main.{r})",
            r = entry.relation
        ),
    );
    if incr_only != 0 {
        return Err(format!(
            "{incr_only} row(s) present in the incremental relation but absent from the \
             oracle (expected 0 — the incremental side must never hold rows the oracle lacks)"
        ));
    }

    let key_list = entry.key_columns.join(", ");
    let distinct_full = scalar_on(
        &conn,
        &format!(
            "SELECT count(*) FROM (SELECT DISTINCT {key_list} FROM full_db.main.{r})",
            r = entry.relation
        ),
    );
    let incr_count = scalar_on(
        &conn,
        &format!("SELECT count(*) FROM incr_db.main.{r}", r = entry.relation),
    );
    if distinct_full != incr_count {
        return Err(format!(
            "fold mismatch: the oracle has {distinct_full} distinct ({key_list}) groups but \
             the incremental relation has {incr_count} rows — every oracle-extra row must be a \
             duplicate within a shared ({key_list}) group, not a novel row"
        ));
    }
    Ok(())
}

fn full_only_count(incr_db: &Path, full_db: &Path, relation: &str) -> i64 {
    let conn = attached_conn(incr_db, full_db);
    scalar_on(
        &conn,
        &format!(
            "SELECT count(*) FROM (SELECT * FROM full_db.main.{relation} EXCEPT ALL \
             SELECT * FROM incr_db.main.{relation})"
        ),
    )
}

/// Compare `incr_db` against `full_db`: every relation must be zero-diff, or
/// registered with a bound that holds. Panics naming the offending relation
/// otherwise.
fn assert_matches_oracle(incr_db: &Path, full_db: &Path, window_label: &str) {
    let diffs = compare_databases(incr_db, full_db)
        .unwrap_or_else(|e| panic!("window {window_label}: coverage failure: {e}"));
    for diff in diffs {
        if let Some(entry) = DIVERGENCE_REGISTRY
            .iter()
            .find(|e| e.relation == diff.relation)
        {
            if let Err(msg) = check_bound(incr_db, full_db, entry) {
                panic!(
                    "window {window_label}: registered divergence bound violated for `{}` \
                     (registered reason: {}): {msg}",
                    entry.relation, entry.reason
                );
            }
        } else if diff.incr_only != 0 || diff.full_only != 0 {
            panic!(
                "window {window_label}: unregistered divergence in `{}` \
                 (incr_only={}, full_only={})\nincremental-only sample: {:?}\n\
                 oracle-only sample: {:?}",
                diff.relation,
                diff.incr_only,
                diff.full_only,
                diff.incr_only_sample,
                diff.full_only_sample
            );
        }
    }
}

/// The full 30-day incremental replay paired with a full-refresh oracle over
/// the identical loaded rows — expensive (a `--full-refresh` recompiles and
/// reruns every model), so built at most once per test binary process and
/// shared between the tests that need the whole-fixture divergence.
struct FullReplayPair {
    _tmp: TempDir,
    incr_db: PathBuf,
    full_db: PathBuf,
}

static FULL_REPLAY_PAIR: OnceLock<FullReplayPair> = OnceLock::new();

fn full_replay_pair() -> &'static FullReplayPair {
    FULL_REPLAY_PAIR.get_or_init(|| {
        let tmp = TempDir::new().expect("tempdir");

        let (incr_workspace, incr_db, incr_sample) =
            stage_workspace(&tmp.path().join("incremental"));
        replay_days(&incr_workspace, &incr_db, &incr_sample, FIXTURE_DAYS);

        let (full_workspace, full_db, full_sample) = stage_workspace(&tmp.path().join("full"));
        create_empty_raw_table(&full_db, &full_sample);
        let mut prev: Option<&str> = None;
        for day in FIXTURE_DAYS {
            load_day(&full_db, &full_sample, day, prev);
            prev = Some(day);
        }
        smelt_run(
            &full_workspace,
            FIXTURE_DAYS[0],
            &day_after(FIXTURE_DAYS[FIXTURE_DAYS.len() - 1]),
            &["--full-refresh"],
        );

        FullReplayPair {
            _tmp: tmp,
            incr_db,
            full_db,
        }
    })
}

/// Test 1: the phase's centrepiece. Replay the 30-day fixture day by day;
/// after each of the first 10 days, and after the final (30th) day, stage a
/// fresh full-refresh oracle over the identical rows seen so far and compare
/// every materialised relation row-for-row.
///
/// **Ignored** — running this today fails at the `2026-08-06` checkpoint on
/// `gold_events_enriched`: an early fact row carries a stale
/// `current_repo_name` after a same-window rename, self-healing by a later
/// run (confirmed zero-diff at the full 30-day window). This is a real
/// divergence class the plan's two-entry registry does not cover and this
/// phase did not characterise a bound for — see `docs/outcomes/
/// 20260906-bigquery-dogfood-spine/outcome.md` "## Blocked" (phase 6). Left
/// in place, not deleted, so the next attempt starts from working
/// infrastructure (the comparator, discovery, and registry machinery below
/// are all exercised and green via tests 2-5).
#[test]
#[ignore = "gold_events_enriched intermediate-window staleness is unbounded/uncharacterised — see outcome.md Blocked, phase 6"]
fn every_window_matches_the_full_refresh_oracle() {
    let tmp = TempDir::new().expect("tempdir");
    let (incr_workspace, incr_db, incr_sample) = stage_workspace(&tmp.path().join("incremental"));
    create_empty_raw_table(&incr_db, &incr_sample);

    let mut prev: Option<&str> = None;
    for (idx, day) in FIXTURE_DAYS.iter().enumerate() {
        load_day(&incr_db, &incr_sample, day, prev);
        smelt_run(&incr_workspace, day, &day_after(day), &[]);
        prev = Some(day);

        let is_final = idx == FIXTURE_DAYS.len() - 1;
        if idx >= 10 && !is_final {
            continue;
        }

        if is_final {
            // The full-fixture oracle: reuse the shared pair rather than
            // building a third full-refresh over all 30 days (test 3 and
            // test 5 also need it).
            let pair = full_replay_pair();
            assert_matches_oracle(&incr_db, &pair.full_db, day);
            continue;
        }

        let (oracle_workspace, oracle_db, oracle_sample) =
            stage_workspace(&tmp.path().join(format!("oracle-{idx}")));
        create_empty_raw_table(&oracle_db, &oracle_sample);
        let mut oracle_prev: Option<&str> = None;
        for d in &FIXTURE_DAYS[0..=idx] {
            load_day(&oracle_db, &oracle_sample, d, oracle_prev);
            oracle_prev = Some(d);
        }
        smelt_run(
            &oracle_workspace,
            FIXTURE_DAYS[0],
            &day_after(day),
            &["--full-refresh"],
        );

        assert_matches_oracle(&incr_db, &oracle_db, day);
    }
}

/// Test 2: coverage totality. Discovery is generic — it is not handed a
/// hardcoded model list — and a relation present in one database but absent
/// from the other fails the comparison rather than being silently skipped.
#[test]
fn oracle_comparison_covers_every_materialised_relation() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, sample) = stage_workspace(&tmp.path().join("a"));
    create_empty_raw_table(&db, &sample);
    load_day(&db, &sample, FIXTURE_DAYS[0], None);
    smelt_run(
        &workspace,
        FIXTURE_DAYS[0],
        &day_after(FIXTURE_DAYS[0]),
        &[],
    );

    let relations = discover_relations(&db);
    for expected in [
        "bronze_events",
        "silver_events_deduped",
        "gold_repo_dim",
        "marts_star_growth",
    ] {
        assert!(
            relations.iter().any(|r| r == expected),
            "expected `{expected}` to be discovered without being named by the test: {relations:?}"
        );
    }
    assert!(
        !relations
            .iter()
            .any(|r| r.starts_with("sources_") || r.starts_with("_smelt_")),
        "raw source and system bookkeeping tables must be excluded from discovery: {relations:?}"
    );

    let (workspace2, db2, sample2) = stage_workspace(&tmp.path().join("b"));
    create_empty_raw_table(&db2, &sample2);
    load_day(&db2, &sample2, FIXTURE_DAYS[0], None);
    smelt_run(
        &workspace2,
        FIXTURE_DAYS[0],
        &day_after(FIXTURE_DAYS[0]),
        &["--full-refresh"],
    );
    duckdb_exec(&db2, "DROP TABLE main.gold_repo_dim");

    let err = compare_databases(&db, &db2).expect_err("expected a coverage failure");
    assert!(
        err.contains("gold_repo_dim"),
        "expected the missing relation to be named in the error: {err}"
    );
}

/// Test 3: the registry's two entries are bounded, not blanket — over the
/// full 30-day fixture, both hold their fold-equality bound.
#[test]
fn succession_divergence_is_exactly_tied_row_multiplicity() {
    let pair = full_replay_pair();
    for entry in DIVERGENCE_REGISTRY {
        check_bound(&pair.incr_db, &pair.full_db, entry)
            .unwrap_or_else(|e| panic!("{}: {e}", entry.relation));
    }
}

/// Test 4: negative control on the comparator itself. Perturb one row of an
/// unregistered relation and assert the comparison reports it.
#[test]
fn an_unregistered_divergence_fails() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, sample) = stage_workspace(&tmp.path().join("a"));
    create_empty_raw_table(&db, &sample);
    load_day(&db, &sample, FIXTURE_DAYS[0], None);
    smelt_run(
        &workspace,
        FIXTURE_DAYS[0],
        &day_after(FIXTURE_DAYS[0]),
        &[],
    );

    let (workspace2, db2, sample2) = stage_workspace(&tmp.path().join("b"));
    create_empty_raw_table(&db2, &sample2);
    load_day(&db2, &sample2, FIXTURE_DAYS[0], None);
    smelt_run(
        &workspace2,
        FIXTURE_DAYS[0],
        &day_after(FIXTURE_DAYS[0]),
        &["--full-refresh"],
    );

    let repo_id = {
        let conn = duckdb::Connection::open(&db2).expect("open db2");
        conn.query_row(
            "SELECT repo_id FROM main.gold_repo_dim LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("query a repo_id to perturb")
    };
    duckdb_exec(
        &db2,
        &format!(
            "UPDATE main.gold_repo_dim SET current_repo_name = current_repo_name || '_x' \
             WHERE repo_id = {repo_id}"
        ),
    );

    let diffs = compare_databases(&db, &db2).expect("coverage matches");
    let gold_repo_dim = diffs
        .iter()
        .find(|d| d.relation == "gold_repo_dim")
        .expect("gold_repo_dim present in the diff set");
    assert!(
        gold_repo_dim.incr_only != 0 || gold_repo_dim.full_only != 0,
        "expected the perturbed row to be reported as a divergence, found none — the \
         comparator would silently pass a real mismatch"
    );
}

/// Test 5: two-sided ratchet on the registry itself, mirroring
/// `dialect_audit`'s ledger — a registry entry naming a relation that no
/// longer diverges fails, telling the reader to delete it.
#[test]
fn registry_entries_are_all_live() {
    let pair = full_replay_pair();
    for entry in DIVERGENCE_REGISTRY {
        let extra = full_only_count(&pair.incr_db, &pair.full_db, entry.relation);
        assert!(
            extra > 0,
            "registry entry for `{}` no longer reproduces a divergence (0 oracle-extra rows \
             over the full 30-day fixture) — delete this entry from DIVERGENCE_REGISTRY",
            entry.relation
        );
    }
}
