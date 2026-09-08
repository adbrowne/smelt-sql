#![cfg(feature = "duckdb")]
//! Per-window full-refresh oracle for `examples/github_activity/`
//! (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/06-plan.md`,
//! `phases/08-plan.md`).
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
//! `every_window_matches_the_full_refresh_oracle` checks after **every** one
//! of the 30 windows (phase 8 measured this at 108s wall time, well under a
//! 5-minute per-PR budget, so the phase 6 plan's first-10-plus-final sampling
//! was dropped rather than kept). The full 30-day full-refresh oracle itself
//! is built at most once per test binary run ([`full_replay_pair`]), shared
//! between the tests that need it.

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

/// A registry entry's bound: two shapes are live today
/// (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/08-plan.md`), none
/// of them a magic row count. A third, `FoldEquality` (the `(key, clock)`
/// tuple the presented table's `MERGE ... ON` addresses by), was retired
/// (`docs/outcomes/20260906-bigquery-correctness/phases/03-plan.md`) once
/// `emit_succession_full_rebuild`'s own fold made its two entries
/// (`silver_repo_naming`, `silver_actor_naming`) compare exactly equal.
/// `DIVERGENCE_REGISTRY` is currently empty (all four root causes fixed, not
/// registered — phase 7 of `docs/outcomes/20260906-bigquery-correctness`
/// fixed the last one), so no variant is constructed by the registry today;
/// phase 8's `check_bound_*` tests construct it locally to keep the
/// `MonotoneDivergence` arm and both `Side` variants live.
enum Bound {
    /// One side of the pair is always at or ahead of the other on
    /// `monotone_columns` (`behind_side` names which side is never allowed
    /// to lead), and every row present in both legs matches exactly on
    /// every column in `exact_columns`.
    MonotoneDivergence {
        key_col: &'static str,
        exact_columns: &'static [&'static str],
        monotone_columns: &'static [&'static str],
        behind_side: Side,
    },
}

/// Which side of a [`Bound::MonotoneDivergence`] is never allowed to lead.
/// Neither variant has a current registry entry (`DIVERGENCE_REGISTRY` is
/// empty — see [`Bound`]'s doc comment); phase 8's `check_bound_*` tests
/// construct both variants locally to keep `check_bound`'s dispatch on both
/// arms live.
#[derive(PartialEq, Eq)]
enum Side {
    Incremental,
    Oracle,
}

/// A composition-relevant divergence between the incremental replay and the
/// full-refresh oracle, bounded by a checkable predicate.
struct DivergenceEntry {
    relation: &'static str,
    reason: &'static str,
    bound: Bound,
}

/// Enrichment column names for `gold_events_enriched` (excludes `id`, the
/// key, and `current_repo_name`, the enrichment column phase 5's heal keeps
/// current).
const ENRICHED_OTHER_COLUMNS: &[&str] = &[
    "type",
    "actor_id",
    "actor_login",
    "repo_id",
    "repo_name",
    "org_id",
    "public",
    "created_at",
    "event_date",
];

/// `silver_repo_naming` and `silver_actor_naming` no longer have entries here:
/// both traced to the same root cause (the window-forward patch loop
/// addresses the presented table by `(key, clock)`, converging a same-second
/// tie to one presented row, while `--full-refresh` re-ran the model's raw
/// compiled `SELECT` with no such addressing and kept every tied physical
/// row) and were fixed by folding `emit_succession_full_rebuild`'s rebuild on
/// `(key_cols, clock_col)` (`docs/outcomes/20260906-bigquery-correctness/
/// phases/03-plan.md`; `crates/smelt-logical/src/maintenance/emit/
/// succession.rs`). The two legs now compare equal on both relations.
///
/// `gold_events_enriched` no longer has an entry here: phases 4-5 of
/// `docs/outcomes/20260906-bigquery-correctness` derive and dispatch a real
/// `UpstreamMutation(gold.repo_dim)` / `Technique::ColumnScopedMerge` cell
/// (an enrichment-keyed route in `append_model_edge_cells`, dispatched once
/// per run over the model's unwindowed output), so `current_repo_name` now
/// heals and the two legs compare exactly equal on this relation too — see
/// `gold_events_enriched_matches_the_full_refresh_oracle` below.
///
/// `silver_actor_sessions` no longer has an entry here: its divergence
/// traced to the **oracle itself under-computing**, not the incremental leg
/// — `compute_calendar_windows` (`crates/smelt-runtime/src/windowing.rs`)
/// applied the Form-B forward-reach rebase only to the two *outer* edges of
/// a single invocation's whole requested range, never to an interior chunk
/// boundary, so a single `--full-refresh` spanning many days truncated any
/// session that continued past its own day. Phase 6 of
/// `docs/outcomes/20260906-bigquery-correctness` folds the skew into every
/// chunk's own scan (`docs/specs/incremental_shapes.md` §"Execution model
/// (DuckDB)", `docs/specs/model_transforms.md` §Semantics "The output window
/// is derived, never assumed"), so the two legs now compare exactly equal —
/// see `silver_actor_sessions_matches_the_full_refresh_oracle` below.
///
/// `marts_daily_active_contributors` no longer has an entry here: its
/// divergence was a **fourth** root cause, direction reversed from
/// `silver_actor_sessions`'s — this mart is Form A relative to
/// `silver.actor_sessions` (its own `partition_column` is
/// `session_start_date`, the exact column it reads, no skew of its own), so
/// it never revisited an already-written partition once
/// `actor_sessions`'s *own* Form-B rebase legitimately rewrote an earlier
/// day's partition: a missing-repair-edge gap, the same shape as
/// `gold_events_enriched`'s, but triggered by a Form-B model's ordinary
/// self-rebase rather than a renamed dimension. Phase 7 of
/// `docs/outcomes/20260906-bigquery-correctness` fixes the propagation
/// itself rather than adding a repair edge: `build_model_plans` now widens a
/// model's own run window to cover the derived output window of every
/// in-run upstream (`docs/specs/model_transforms.md` §Semantics "The
/// derived output window propagates within a run."), so a run that rebases
/// `actor_sessions`'s partition also re-runs this mart over that same
/// rebased window, and the two legs now compare exactly equal — see
/// `marts_daily_active_contributors_matches_the_full_refresh_oracle` below.
///
/// **The registry is now empty.** [`an_unregistered_divergence_fails`]
/// still runs and still fails closed on a genuine unregistered mismatch —
/// it is not a name lookup into this (now-empty) slice, it queries every
/// materialised relation directly, so an empty `DIVERGENCE_REGISTRY` does
/// not make it vacuous. Proven, not just asserted in prose: phase 8's
/// `assert_matches_oracle_fails_closed_on_an_empty_registry` drives the
/// registry-consulting comparator itself (not `compare_databases`) over a
/// perturbed relation and checks it reports the mismatch;
/// `check_bound_accepts_a_holding_bound`, `check_bound_rejects_a_leading_side`
/// and `check_bound_rejects_divergence_outside_the_licensed_columns` exercise
/// `check_bound`'s `MonotoneDivergence` arm and both `Side` variants, which an
/// empty registry otherwise leaves with no live test; and
/// `no_relation_diverges_unexplained` names criterion 5's "count of
/// unexplained differences is zero" as a direct assertion over the full
/// 30-day fixture, not left to the generic sweep's silence.
const DIVERGENCE_REGISTRY: &[DivergenceEntry] = &[];

/// A registry entry's bound, dispatched on [`Bound`]'s shape.
fn check_bound(incr_db: &Path, full_db: &Path, entry: &DivergenceEntry) -> Result<(), String> {
    let conn = attached_conn(incr_db, full_db);
    match &entry.bound {
        Bound::MonotoneDivergence {
            key_col,
            exact_columns,
            monotone_columns,
            behind_side,
        } => {
            check_key_sets_equal(&conn, entry.relation, key_col)?;
            check_columns_match_exactly(&conn, entry.relation, key_col, exact_columns)?;

            let (op, leader, follower) = match behind_side {
                Side::Oracle => ("<", "oracle", "incremental"),
                Side::Incremental => (">", "incremental", "oracle"),
            };
            for col in *monotone_columns {
                let behind = scalar_on(
                    &conn,
                    &format!(
                        "SELECT count(*) FROM incr_db.main.{r} i \
                         JOIN full_db.main.{r} f USING ({key_col}) \
                         WHERE i.{col} {op} f.{col}",
                        r = entry.relation
                    ),
                );
                if behind != 0 {
                    return Err(format!(
                        "{behind} row(s) have the {leader} leg's `{col}` ahead of the \
                         {follower} leg's — expected {follower} to never lead on a monotone \
                         column"
                    ));
                }
            }
            Ok(())
        }
    }
}

/// Every row keyed by `key_col` exists on both sides — a divergence bound
/// never licenses a missing or extra row, only a differing value.
fn check_key_sets_equal(
    conn: &duckdb::Connection,
    relation: &str,
    key_col: &str,
) -> Result<(), String> {
    let key_only_incr = scalar_on(
        conn,
        &format!(
            "SELECT count(*) FROM incr_db.main.{relation} i WHERE NOT EXISTS \
             (SELECT 1 FROM full_db.main.{relation} f WHERE f.{key_col} = i.{key_col})"
        ),
    );
    let key_only_full = scalar_on(
        conn,
        &format!(
            "SELECT count(*) FROM full_db.main.{relation} f WHERE NOT EXISTS \
             (SELECT 1 FROM incr_db.main.{relation} i WHERE i.{key_col} = f.{key_col})"
        ),
    );
    if key_only_incr != 0 || key_only_full != 0 {
        return Err(format!(
            "row-key-set mismatch on `{key_col}`: {key_only_incr} incremental-only, \
             {key_only_full} oracle-only — a divergence bound never licenses a missing or \
             extra row"
        ));
    }
    Ok(())
}

/// Every column in `columns` matches exactly between rows sharing `key_col`
/// — the bound licenses divergence only in the columns named explicitly by
/// its own shape, never elsewhere.
fn check_columns_match_exactly(
    conn: &duckdb::Connection,
    relation: &str,
    key_col: &str,
    columns: &[&str],
) -> Result<(), String> {
    for col in columns {
        let n = scalar_on(
            conn,
            &format!(
                "SELECT count(*) FROM incr_db.main.{relation} i \
                 JOIN full_db.main.{relation} f USING ({key_col}) \
                 WHERE i.{col} IS DISTINCT FROM f.{col}"
            ),
        );
        if n != 0 {
            return Err(format!(
                "{n} row(s) differ on `{col}`, which this bound does not license to diverge"
            ));
        }
    }
    Ok(())
}

/// Column-by-column diff for two relations sharing key column `key_col`,
/// restricted to `columns` (excluding the key itself). Used only for
/// measurement (`every_window_deep_sweep`) — the per-PR gate compares whole
/// rows via [`relation_diff`], not per-column.
struct ColumnDiff {
    key_only_incr: i64,
    key_only_full: i64,
    differing: Vec<(String, i64)>,
}

fn column_level_diff(
    conn: &duckdb::Connection,
    relation: &str,
    key_col: &str,
    columns: &[&str],
) -> ColumnDiff {
    let mut differing = Vec::new();
    for col in columns {
        if *col == key_col {
            continue;
        }
        let n = scalar_on(
            conn,
            &format!(
                "SELECT count(*) FROM incr_db.main.{relation} i \
                 JOIN full_db.main.{relation} f USING ({key_col}) \
                 WHERE i.{col} IS DISTINCT FROM f.{col}"
            ),
        );
        if n > 0 {
            differing.push((col.to_string(), n));
        }
    }
    let key_only_incr = scalar_on(
        conn,
        &format!(
            "SELECT count(*) FROM incr_db.main.{relation} i WHERE NOT EXISTS \
             (SELECT 1 FROM full_db.main.{relation} f WHERE f.{key_col} = i.{key_col})"
        ),
    );
    let key_only_full = scalar_on(
        conn,
        &format!(
            "SELECT count(*) FROM full_db.main.{relation} f WHERE NOT EXISTS \
             (SELECT 1 FROM incr_db.main.{relation} i WHERE i.{key_col} = f.{key_col})"
        ),
    );
    ColumnDiff {
        key_only_incr,
        key_only_full,
        differing,
    }
}

/// The `id`s where `gold_events_enriched`'s `current_repo_name` differs
/// between the two databases, paired with the affected `repo_id` — measurement
/// support for tracking whether a specific stale row heals across later
/// windows.
fn stale_enrichment_ids(conn: &duckdb::Connection) -> Vec<(i64, i64)> {
    let sql = "SELECT i.id, i.repo_id FROM incr_db.main.gold_events_enriched i \
               JOIN full_db.main.gold_events_enriched f USING (id) \
               WHERE i.current_repo_name IS DISTINCT FROM f.current_repo_name \
               ORDER BY 1";
    let mut stmt = conn.prepare(sql).expect("prepare stale_enrichment_ids");
    stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
        .expect("query stale_enrichment_ids")
        .collect::<Result<_, _>>()
        .expect("collect stale_enrichment_ids")
}

/// Measurement-only sweep (`docs/outcomes/20260906-bigquery-dogfood-spine/
/// phases/08-plan.md` task 1): build a growing full-refresh oracle after
/// **every** one of the 30 windows (not just the first 10 + final, unlike the
/// per-PR centrepiece) and record, per day, which relations diverge and on
/// which columns — plus the exact `gold_events_enriched` ids that stay stale,
/// so a later run can tell which ones healed and which never did.
///
/// `#[ignore]`d by design: this is measurement, not a gate. Run with
/// `cargo test -p smelt-cli --test github_activity_oracle every_window_deep_sweep \
///  -- --ignored --nocapture`. Its output is transcribed into the phase 8
/// summary and `examples/github_activity/README.md`, not asserted here.
#[test]
#[ignore = "measurement-only sweep, not a gate — see doc comment"]
fn every_window_deep_sweep() {
    const ENRICHED_COLUMNS: &[&str] = &[
        "type",
        "actor_id",
        "actor_login",
        "repo_id",
        "repo_name",
        "org_id",
        "public",
        "created_at",
        "event_date",
        "current_repo_name",
    ];

    let tmp = TempDir::new().expect("tempdir");
    let (incr_workspace, incr_db, incr_sample) = stage_workspace(&tmp.path().join("incremental"));
    create_empty_raw_table(&incr_db, &incr_sample);

    let mut prev: Option<&str> = None;
    for (idx, day) in FIXTURE_DAYS.iter().enumerate() {
        load_day(&incr_db, &incr_sample, day, prev);
        smelt_run(&incr_workspace, day, &day_after(day), &[]);
        prev = Some(day);

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

        let diffs = compare_databases(&incr_db, &oracle_db)
            .unwrap_or_else(|e| panic!("day {day}: coverage failure: {e}"));
        let conn = attached_conn(&incr_db, &oracle_db);
        for diff in &diffs {
            if diff.incr_only == 0 && diff.full_only == 0 {
                continue;
            }
            if diff.relation == "gold_events_enriched" {
                let cd = column_level_diff(&conn, "gold_events_enriched", "id", ENRICHED_COLUMNS);
                let stale = stale_enrichment_ids(&conn);
                eprintln!(
                    "DEEPSWEEP day={day} idx={idx} relation=gold_events_enriched \
                     key_only_incr={} key_only_full={} differing_columns={:?} \
                     stale_ids={:?}",
                    cd.key_only_incr, cd.key_only_full, cd.differing, stale
                );
            } else {
                eprintln!(
                    "DEEPSWEEP day={day} idx={idx} relation={} incr_only={} full_only={}",
                    diff.relation, diff.incr_only, diff.full_only
                );
            }
        }
    }
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
/// registered with a bound that holds. `Err` names the offending relation
/// rather than panicking, so the registry-consulting comparator itself
/// (not just [`compare_databases`]) can be driven from a test that expects
/// the mismatch (phase 8's `assert_matches_oracle_fails_closed_on_an_empty_
/// registry`) without unwinding.
fn check_matches_oracle(incr_db: &Path, full_db: &Path, window_label: &str) -> Result<(), String> {
    let diffs = compare_databases(incr_db, full_db)
        .map_err(|e| format!("window {window_label}: coverage failure: {e}"))?;
    for diff in diffs {
        if let Some(entry) = DIVERGENCE_REGISTRY
            .iter()
            .find(|e| e.relation == diff.relation)
        {
            if let Err(msg) = check_bound(incr_db, full_db, entry) {
                return Err(format!(
                    "window {window_label}: registered divergence bound violated for `{}` \
                     (registered reason: {}): {msg}",
                    entry.relation, entry.reason
                ));
            }
        } else if diff.incr_only != 0 || diff.full_only != 0 {
            return Err(format!(
                "window {window_label}: unregistered divergence in `{}` \
                 (incr_only={}, full_only={})\nincremental-only sample: {:?}\n\
                 oracle-only sample: {:?}",
                diff.relation,
                diff.incr_only,
                diff.full_only,
                diff.incr_only_sample,
                diff.full_only_sample
            ));
        }
    }
    Ok(())
}

/// Panicking wrapper over [`check_matches_oracle`] — every existing caller
/// stays on this, so no current test's behaviour moves.
fn assert_matches_oracle(incr_db: &Path, full_db: &Path, window_label: &str) {
    if let Err(msg) = check_matches_oracle(incr_db, full_db, window_label) {
        panic!("{msg}");
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
/// after **every** day (not sampled — see below), stage a fresh full-refresh
/// oracle over the identical rows seen so far and compare every materialised
/// relation row-for-row, against the 1-entry [`DIVERGENCE_REGISTRY`].
///
/// Was `#[ignore]`d (`docs/outcomes/20260906-bigquery-dogfood-spine/
/// outcome.md` "## Blocked", phase 6) on an uncharacterised `gold_events_
/// enriched` divergence, at the time believed to self-heal. Phase 8's
/// `every_window_deep_sweep` measured the real shape (it does not self-heal;
/// see `DIVERGENCE_REGISTRY`'s doc comment) and phase 8 also found two
/// further, previously-unknown divergent relations
/// (`silver_actor_sessions`/`marts_daily_active_contributors`, a genuine
/// full-refresh-oracle bug — same doc comment). All three were registered
/// bounds at the time, so this ran unignored; `gold_events_enriched`'s own
/// divergence was fixed and de-registered by phases 4-5 (see
/// `gold_events_enriched_matches_the_full_refresh_oracle`), and
/// `silver_actor_sessions`'s by phase 6 (see
/// `silver_actor_sessions_matches_the_full_refresh_oracle`), leaving the
/// 1-entry registry above.
///
/// Checks **every** day, not the first-10-plus-final sampling the phase 6
/// plan allowed for runtime: phase 8 task 9 measured the every-day sweep
/// (`every_window_deep_sweep`) at 108s wall time for the full 30 days, well
/// under the plan's 5-minute budget, so the cheaper sampling was never
/// needed and would have hidden the sessions divergence (which only starts
/// at day 12) had it stayed in place.
#[test]
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

/// Test 3: every registry entry is bounded, not blanket — over the full
/// 30-day fixture, it holds its declared `MonotoneDivergence` bound.
#[test]
fn succession_divergence_is_exactly_tied_row_multiplicity() {
    let pair = full_replay_pair();
    for entry in DIVERGENCE_REGISTRY {
        check_bound(&pair.incr_db, &pair.full_db, entry)
            .unwrap_or_else(|e| panic!("{}: {e}", entry.relation));
    }
}

/// Test (phase 5, `docs/outcomes/20260906-bigquery-correctness`): with the
/// enrichment-keyed heal live, `gold_events_enriched` no longer diverges from
/// the full-refresh oracle at all — replaces the two tests that asserted the
/// (now-fixed) `StaleButHistoricallyValid` divergence was present
/// (`enrichment_staleness_is_confined_to_the_enriched_column`,
/// `enrichment_staleness_is_never_a_fabricated_value`). With no registry
/// entry for this relation, `assert_matches_oracle`'s own unregistered-
/// divergence sweep (exercised by `every_window_matches_the_full_refresh_
/// oracle`) already enforces exact equality on every window; this test names
/// the invariant directly, once, over the full 30-day fixture, so a reader
/// sees it without deriving it from the generic sweep's silence.
#[test]
fn gold_events_enriched_matches_the_full_refresh_oracle() {
    let pair = full_replay_pair();
    let conn = attached_conn(&pair.incr_db, &pair.full_db);

    check_key_sets_equal(&conn, "gold_events_enriched", "id")
        .expect("gold_events_enriched must have identical id key sets on both legs");
    let all_columns: Vec<&str> = {
        let mut cols: Vec<&str> = ENRICHED_OTHER_COLUMNS.to_vec();
        cols.push("current_repo_name");
        cols
    };
    check_columns_match_exactly(&conn, "gold_events_enriched", "id", &all_columns).expect(
        "gold_events_enriched must match the full-refresh oracle on every column, including \
         current_repo_name, now that the enrichment-keyed heal (phases 4-5) is live",
    );
}

/// Test 5 (phase 6, `docs/outcomes/20260906-bigquery-correctness`): with
/// every chunk's own scan folding the Form-B skew (`compute_calendar_
/// windows`'s interior-chunk widening), the full-refresh oracle no longer
/// under-computes a cross-midnight session, so `silver_actor_sessions` no
/// longer diverges from the incremental replay at all — replaces the
/// `MonotoneDivergence` bound this const's doc comment used to name. With no
/// registry entry for this relation, `assert_matches_oracle`'s own
/// unregistered-divergence sweep (exercised by `every_window_matches_the_
/// full_refresh_oracle`) already enforces exact equality on every window;
/// this test names the invariant directly, once, over the full 30-day
/// fixture, so a reader sees it without deriving it from the generic
/// sweep's silence.
#[test]
fn silver_actor_sessions_matches_the_full_refresh_oracle() {
    let pair = full_replay_pair();
    let conn = attached_conn(&pair.incr_db, &pair.full_db);

    check_key_sets_equal(&conn, "silver_actor_sessions", "session_id")
        .expect("silver_actor_sessions must have identical session_id key sets on both legs");
    check_columns_match_exactly(
        &conn,
        "silver_actor_sessions",
        "session_id",
        &[
            "actor_id",
            "session_start_ts",
            "session_start_date",
            "session_start",
            "session_end",
            "event_count",
        ],
    )
    .expect(
        "silver_actor_sessions must match the full-refresh oracle on every column, now that \
         every interior chunk's scan carries the model's own Form-B forward reach (phase 6)",
    );
}

/// Test 8 (phase 7, `docs/outcomes/20260906-bigquery-correctness`): with
/// `build_model_plans` widening a downstream's run window to cover every
/// in-run upstream's derived output window (`docs/specs/model_transforms.md`
/// §Semantics "The derived output window propagates within a run."), a run
/// that rebases `actor_sessions`'s partitions also re-runs this Form-A mart
/// over the same rebased window, so `total_events` no longer freezes at
/// first-write time — replaces the `MonotoneDivergence` bound this const's
/// doc comment used to name. With no registry entry for this relation,
/// `assert_matches_oracle`'s own unregistered-divergence sweep (exercised by
/// `every_window_matches_the_full_refresh_oracle`) already enforces exact
/// equality on every window; this test names the invariant directly, once,
/// over the full 30-day fixture.
#[test]
fn marts_daily_active_contributors_matches_the_full_refresh_oracle() {
    let pair = full_replay_pair();
    let conn = attached_conn(&pair.incr_db, &pair.full_db);

    check_key_sets_equal(
        &conn,
        "marts_daily_active_contributors",
        "session_start_date",
    )
    .expect(
        "marts_daily_active_contributors must have identical session_start_date key sets \
             on both legs",
    );
    check_columns_match_exactly(
        &conn,
        "marts_daily_active_contributors",
        "session_start_date",
        &["total_sessions", "distinct_actors", "total_events"],
    )
    .expect(
        "marts_daily_active_contributors must match the full-refresh oracle on every column, \
         including total_events, now that a downstream's run window widens to cover its \
         Form-B upstream's rebased output window (phase 7)",
    );
}

/// Stages a one-day incremental replay and a one-day `--full-refresh` oracle
/// over identical loaded rows, then perturbs one `gold_repo_dim` row's
/// `current_repo_name` in the oracle leg (appending `_x`, which is always
/// lexicographically greater than the original — a standard string
/// comparison never orders a string before one of its own proper prefixes —
/// so the oracle leg is the one left leading on this column). Shared by
/// [`an_unregistered_divergence_fails`] and phase 8's registry fail-closed
/// tests, which all need the same perturbed pair; returns the `TempDir` so
/// the caller keeps both databases alive.
fn perturbed_one_day_pair() -> (TempDir, PathBuf, PathBuf) {
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

    (tmp, db, db2)
}

/// Test 4: negative control on the comparator itself. Perturb one row of an
/// unregistered relation and assert the comparison reports it.
#[test]
fn an_unregistered_divergence_fails() {
    let (_tmp, db, db2) = perturbed_one_day_pair();

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

/// Test 1 (phase 8): the registry-consulting comparator itself — not just
/// [`compare_databases`] — reports a genuine unregistered divergence. Today
/// `DIVERGENCE_REGISTRY` is empty, so this is also the only live coverage of
/// [`check_matches_oracle`]'s unregistered-divergence branch.
#[test]
fn assert_matches_oracle_fails_closed_on_an_empty_registry() {
    assert!(
        DIVERGENCE_REGISTRY.is_empty(),
        "this test's assertions assume an empty registry — see the DIVERGENCE_REGISTRY \
         doc comment for what changes once an entry is added"
    );
    let (_tmp, db, db2) = perturbed_one_day_pair();

    let err = check_matches_oracle(&db, &db2, "day0")
        .expect_err("expected the perturbed gold_repo_dim row to be reported as a divergence");
    assert!(
        err.contains("gold_repo_dim"),
        "expected the offending relation to be named: {err}"
    );
    assert!(
        err.contains("unregistered divergence"),
        "expected the unregistered-divergence branch, not the registered-bound branch: {err}"
    );
}

/// Test 2 (phase 8): a [`Bound::MonotoneDivergence`] holds when the side it
/// licenses to be behind (`Side::Incremental` — the incremental leg here,
/// since [`perturbed_one_day_pair`] leaves the oracle leg leading on
/// `current_repo_name`) is in fact the side that never leads.
#[test]
fn check_bound_accepts_a_holding_bound() {
    let (_tmp, db, db2) = perturbed_one_day_pair();
    let entry = DivergenceEntry {
        relation: "gold_repo_dim",
        reason: "test-local: incremental leg genuinely behind on current_repo_name",
        bound: Bound::MonotoneDivergence {
            key_col: "repo_id",
            exact_columns: &["first_seen_at"],
            monotone_columns: &["current_repo_name"],
            behind_side: Side::Incremental,
        },
    };
    check_bound(&db, &db2, &entry)
        .expect("the incremental leg never leads on current_repo_name; the bound should hold");
}

/// Test 3 (phase 8): the same bound with `behind_side` flipped to the side
/// that is actually leading (`Side::Oracle`) is rejected, naming the column.
#[test]
fn check_bound_rejects_a_leading_side() {
    let (_tmp, db, db2) = perturbed_one_day_pair();
    let entry = DivergenceEntry {
        relation: "gold_repo_dim",
        reason: "test-local: oracle leg wrongly licensed to lead",
        bound: Bound::MonotoneDivergence {
            key_col: "repo_id",
            exact_columns: &["first_seen_at"],
            monotone_columns: &["current_repo_name"],
            behind_side: Side::Oracle,
        },
    };
    let err = check_bound(&db, &db2, &entry)
        .expect_err("the oracle leg leads on current_repo_name; behind_side: Oracle forbids that");
    assert!(
        err.contains("current_repo_name"),
        "expected the leading column to be named: {err}"
    );
}

/// Test 4 (phase 8): a bound never blanket-licenses a relation — moving the
/// diverging column into `exact_columns` (instead of `monotone_columns`)
/// rejects it too, since exact-match licenses no divergence at all.
#[test]
fn check_bound_rejects_divergence_outside_the_licensed_columns() {
    let (_tmp, db, db2) = perturbed_one_day_pair();
    let entry = DivergenceEntry {
        relation: "gold_repo_dim",
        reason: "test-local: current_repo_name wrongly licensed via exact match",
        bound: Bound::MonotoneDivergence {
            key_col: "repo_id",
            exact_columns: &["first_seen_at", "current_repo_name"],
            monotone_columns: &[],
            behind_side: Side::Incremental,
        },
    };
    let err = check_bound(&db, &db2, &entry).expect_err(
        "current_repo_name differs between legs and exact_columns licenses no divergence",
    );
    assert!(
        err.contains("current_repo_name"),
        "expected the unlicensed column to be named: {err}"
    );
}

/// Test 5 (phase 8): criterion 5's "count of unexplained differences is
/// zero" asserted directly, in one named test, over the full 30-day
/// fixture — not left to be derived from the generic sweep's silence.
#[test]
fn no_relation_diverges_unexplained() {
    assert!(
        DIVERGENCE_REGISTRY.is_empty(),
        "this test's zero-unexplained-count claim assumes an empty registry"
    );
    let pair = full_replay_pair();
    let diffs = compare_databases(&pair.incr_db, &pair.full_db)
        .unwrap_or_else(|e| panic!("coverage failure: {e}"));
    for diff in &diffs {
        assert_eq!(
            (diff.incr_only, diff.full_only),
            (0, 0),
            "relation `{}` diverges unexplained (incr_only={}, full_only={}) though \
             DIVERGENCE_REGISTRY is empty — criterion 5's zero-unexplained-count claim is \
             false",
            diff.relation,
            diff.incr_only,
            diff.full_only
        );
    }
}

/// Test 5: two-sided ratchet on the registry itself, mirroring
/// `dialect_audit`'s ledger — a registry entry naming a relation that no
/// longer diverges fails, telling the reader to delete it. On an empty
/// registry (today) that loop is vacuous by construction, so this test also
/// requires [`no_relation_diverges_unexplained`]'s condition directly: an
/// empty `DIVERGENCE_REGISTRY` is a claim that nothing diverges, not an
/// omission this ratchet is silent about either way.
#[test]
fn registry_entries_are_all_live() {
    let pair = full_replay_pair();
    if DIVERGENCE_REGISTRY.is_empty() {
        let diffs = compare_databases(&pair.incr_db, &pair.full_db)
            .unwrap_or_else(|e| panic!("coverage failure: {e}"));
        for diff in &diffs {
            assert_eq!(
                (diff.incr_only, diff.full_only),
                (0, 0),
                "DIVERGENCE_REGISTRY is empty, but relation `{}` diverges (incr_only={}, \
                 full_only={}) — an empty registry claims nothing diverges, not licence for \
                 this ratchet to be a no-op",
                diff.relation,
                diff.incr_only,
                diff.full_only
            );
        }
        return;
    }
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

/// The interim findings handoff banked by phase 15
/// (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/15-plan.md`) — a
/// cheap, string-level drift gate over a committed doc, not a content check.
const FINDINGS_HANDOFF: &str =
    include_str!("../../../docs/handoffs/2026-09-08-github-activity-findings.md");

/// Relation names the handoff's "five registered divergences" table claims
/// are registered — parsed from its own markdown table rather than assumed,
/// so a renamed or retired `DIVERGENCE_REGISTRY` entry cannot leave a stale
/// row silently behind (test `findings_handoff_names_no_unknown_relation`).
///
/// Scoped to the `## The registered divergences` section (bounded by its
/// heading and the next `##`/`###` heading): the generic "any backtick-leading
/// table row" scan used to run over the whole document, which flagged phase
/// 9's unrelated `## Criterion 6` table (a test-name column, not a registered
/// relation) as a stale registered-divergence claim — see
/// `the_handoff_scan_is_scoped_to_the_divergence_section`.
fn handoff_claimed_relations() -> Vec<String> {
    claimed_relations_in(FINDINGS_HANDOFF)
}

/// The pure scan behind [`handoff_claimed_relations`], taking the document
/// text as a parameter so the scoping behaviour itself is testable against a
/// synthetic document rather than only the one committed handoff.
fn claimed_relations_in(text: &str) -> Vec<String> {
    const SECTION_HEADING: &str = "## The registered divergences";
    let lines: Vec<&str> = text.lines().collect();
    let Some(start) = lines.iter().position(|l| l.trim() == SECTION_HEADING) else {
        return Vec::new();
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| {
            let l = l.trim();
            l.starts_with("## ") || l.starts_with("### ")
        })
        .map(|offset| start + 1 + offset)
        .unwrap_or(lines.len());

    lines[start + 1..end]
        .iter()
        .filter_map(|line| {
            let line = line.trim();
            if !line.starts_with("| `") {
                return None;
            }
            let rest = &line[3..];
            let end = rest.find('`')?;
            Some(rest[..end].to_string())
        })
        .collect()
}

/// Test 6 (phase 15, accept direction): every `DIVERGENCE_REGISTRY` entry's
/// relation name appears verbatim in the findings handoff.
#[test]
fn every_registry_entry_is_named_in_the_findings_handoff() {
    for entry in DIVERGENCE_REGISTRY {
        assert!(
            FINDINGS_HANDOFF.contains(entry.relation),
            "docs/handoffs/2026-09-08-github-activity-findings.md does not name registry \
             relation `{}` — every DIVERGENCE_REGISTRY entry must be traceable in the \
             findings handoff",
            entry.relation
        );
    }
}

/// Test 7 (phase 15, reverse direction): every relation the handoff's
/// divergence table claims is registered actually resolves to a
/// `DIVERGENCE_REGISTRY` entry, so a renamed or retired entry cannot leave a
/// stale row in the document. `DIVERGENCE_REGISTRY` is now empty (phase 7 of
/// `docs/outcomes/20260906-bigquery-correctness` fixed the fourth and final
/// root cause rather than registering it), so the handoff's own divergence
/// table is expected to claim nothing either — the reverse-direction check
/// becomes "the doc claims no stale registered relation."
#[test]
fn findings_handoff_names_no_unknown_relation() {
    let claimed = handoff_claimed_relations();
    if DIVERGENCE_REGISTRY.is_empty() {
        assert!(
            claimed.is_empty(),
            "DIVERGENCE_REGISTRY is empty, but the findings handoff's divergence table still \
             claims relation(s) {claimed:?} as registered — stale table row"
        );
        return;
    }
    for relation in &claimed {
        assert!(
            DIVERGENCE_REGISTRY.iter().any(|e| e.relation == relation),
            "findings handoff names relation `{relation}` as registered, but no \
             DIVERGENCE_REGISTRY entry with that name exists — stale or renamed entry"
        );
    }
}

/// Phase 10 test 4: a backtick-leading table row in a section *other* than
/// `## The registered divergences` (e.g. phase 9's `## Criterion 6` test-name
/// table) is not read as a registered-divergence claim. RED before
/// `handoff_claimed_relations()` was scoped to that section.
#[test]
fn the_handoff_scan_is_scoped_to_the_divergence_section() {
    let synthetic = "\
## The registered divergences

Nothing registered here.

## Criterion 6 — the two known live conformance failures

| Test: `diamond_propagation_suffices` | fixed |
| Test: `composed_keyed_pool_upholds_equivalence` | fixed |

## References
";
    let claimed = claimed_relations_in(synthetic);
    assert!(
        claimed.is_empty(),
        "expected no claimed relations from a table outside the divergence section, got \
         {claimed:?}"
    );
}

/// Phase 10 test 5: non-vacuity for test 4 — a fabricated backtick-leading
/// row *inside* the divergence section is still collected, so
/// `findings_handoff_names_no_unknown_relation` keeps its teeth.
#[test]
fn the_handoff_scan_still_catches_a_stale_claim_in_its_own_section() {
    let synthetic = "\
## The registered divergences

| `some_stale_relation` | still claimed |

## Criterion 6 — the two known live conformance failures

Unrelated section.
";
    let claimed = claimed_relations_in(synthetic);
    assert_eq!(
        claimed,
        vec!["some_stale_relation".to_string()],
        "expected the in-section row to still be collected, got {claimed:?}"
    );
}

/// Test 8 (phase 15): the handoff carries an explicit interim marker so a
/// downstream harvest phase cannot mistake it for the complete criterion-8
/// artifact — the live-BigQuery half lands in phase 16.
#[test]
fn findings_handoff_declares_its_interim_status() {
    assert!(
        FINDINGS_HANDOFF.contains("DuckDB half only"),
        "findings handoff must declare it is the DuckDB half only"
    );
    assert!(
        FINDINGS_HANDOFF.contains("phase 16"),
        "findings handoff must point to phase 16 for the live-BigQuery half"
    );
}
