//! T4 — the fingerprint sidecar build + synthesized external change feed
//! (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phases
//! F3–F4; `docs/specs/sources.md` §"The fingerprint sidecar").
//!
//! For a `mutable_snapshot` external source with no native change feed,
//! `smelt_runtime::maintenance_driver::diff_fingerprint_sidecar_changed_keys`
//! synthesizes an exact changed-key set by comparing the source's current
//! row-content digest (restricted to the P4 fingerprint projection) against
//! a warehouse-resident sidecar of previously-observed digests;
//! `refresh_fingerprint_sidecar` then brings the sidecar up to date,
//! transactionally with the consuming write it rides alongside. This suite
//! proves, against a real DuckDB backend:
//!
//! - an absent sidecar makes every current source row "changed" (the
//!   whole-table delta the widen-never-narrow default produces) and the
//!   first diff/refresh populates the sidecar as a byproduct;
//! - a source edit touching a small subset of a 1000-row source makes the
//!   next diff's changed-key set exactly those edited keys — the digest-
//!   soundness oracle's positive leg (real content edits are detected);
//! - an edit to a column OUTSIDE the P4 projection yields an EMPTY changed
//!   set — the oracle's negative leg (no false-positive "changed" verdict);
//! - a deleted source row surfaces as a changed key (a deletion) and is
//!   GC'd from the sidecar on refresh;
//! - the sidecar refresh commits atomically with the write it rides with —
//!   a failed write leaves the sidecar exactly as it was, so a re-run
//!   recomputes the identical changed-key set (no half-committed digest);
//! - (Phase F4) a projection change, a source column entering the
//!   projection, a model-definition edit (holding the projection fixed),
//!   and a hand-corrupted stamp all invalidate the affected partition —
//!   degrading to the SAME whole-table delta an absent sidecar produces,
//!   never a narrower comparison and never a silent skip — and a
//!   mid-sequence invalidation still matches the full-refresh oracle on
//!   every subsequent step.

use smelt_backend::{Backend, BackendError, StatementGroup};
use smelt_backend_duckdb::DuckDbBackend;
use smelt_logical::analysis::fingerprint::Projection;
use smelt_runtime::maintenance_driver::{
    diff_fingerprint_sidecar_changed_keys, refresh_fingerprint_sidecar,
};
use std::cell::RefCell;
use std::sync::Once;
use tracing_subscriber::layer::SubscriberExt;

const SOURCE_ADDRESS: &str = "smelt.sources.dim_users";
const SOURCE_TABLE: &str = "main.dim_users";

/// A placeholder "consuming model's SQL text" — these tests aren't about
/// any real model body, just a stable value threaded through as
/// `model_sql` (`compute_fingerprint_sidecar_stamp`'s model-definition-
/// provenance component) for every diff/refresh that isn't specifically
/// exercising a model-definition edit.
const MODEL_SQL: &str = "SELECT id, name, tier FROM smelt.sources.dim_users";

/// A second, distinct model SQL text — used by the invalidation tests below
/// to simulate a model-definition edit that leaves the P4 projection
/// unchanged (so `projection_identity` alone cannot detect it; only the
/// stamp's model-hash component can).
const EDITED_MODEL_SQL: &str =
    "SELECT id, name, tier FROM smelt.sources.dim_users WHERE tier IS NOT NULL";

fn projection() -> Projection {
    Projection::Columns(
        ["name".to_string(), "tier".to_string()]
            .into_iter()
            .collect(),
    )
}

fn all_columns() -> Vec<String> {
    vec![
        "id".to_string(),
        "name".to_string(),
        "tier".to_string(),
        "notes".to_string(),
    ]
}

async fn create_source(backend: &DuckDbBackend, n: i64) {
    backend
        .execute_sql(
            "CREATE TABLE main.dim_users (id INTEGER, name VARCHAR, tier VARCHAR, notes VARCHAR)",
        )
        .await
        .expect("create source table");
    let insert_sql = format!(
        "INSERT INTO main.dim_users \
         SELECT i AS id, 'user_' || i AS name, CASE WHEN i % 2 = 0 THEN 'gold' ELSE 'silver' END \
         AS tier, 'note_' || i AS notes FROM generate_series(1, {n}) AS t(i)"
    );
    backend
        .execute_sql(&insert_sql)
        .await
        .expect("seed source table");
}

/// A single-column P4 projection (`tier` only) — used by the bug-2
/// regression conformance test below, where the digest expression is never
/// wrapped in `CONCAT` and so exercises the single-column code path
/// directly.
fn single_column_projection() -> Projection {
    Projection::Columns(["tier".to_string()].into_iter().collect())
}

async fn diff_with_projection_and_model(
    backend: &DuckDbBackend,
    projection: &Projection,
    model_sql: &str,
) -> Vec<String> {
    diff_fingerprint_sidecar_changed_keys(
        backend,
        "main",
        SOURCE_ADDRESS,
        SOURCE_TABLE,
        &["id".to_string()],
        projection,
        &all_columns(),
        model_sql,
    )
    .await
    .expect("diff")
}

async fn diff_with_projection(backend: &DuckDbBackend, projection: &Projection) -> Vec<String> {
    diff_with_projection_and_model(backend, projection, MODEL_SQL).await
}

async fn diff(backend: &DuckDbBackend) -> Vec<String> {
    diff_with_projection(backend, &projection()).await
}

async fn refresh_with_projection_and_model(
    backend: &DuckDbBackend,
    projection: &Projection,
    model_sql: &str,
    write_group: &StatementGroup,
) -> Result<(), BackendError> {
    refresh_fingerprint_sidecar(
        backend,
        "main",
        SOURCE_ADDRESS,
        SOURCE_TABLE,
        &["id".to_string()],
        projection,
        &all_columns(),
        model_sql,
        write_group,
    )
    .await
}

async fn refresh_with_projection(
    backend: &DuckDbBackend,
    projection: &Projection,
    write_group: &StatementGroup,
) -> Result<(), BackendError> {
    refresh_with_projection_and_model(backend, projection, MODEL_SQL, write_group).await
}

async fn refresh(
    backend: &DuckDbBackend,
    write_group: &StatementGroup,
) -> Result<(), BackendError> {
    refresh_with_projection(backend, &projection(), write_group).await
}

fn empty_write_group() -> StatementGroup {
    StatementGroup {
        statements: vec![],
        transactional: false,
    }
}

async fn sidecar_row_count(backend: &DuckDbBackend) -> usize {
    let batches = backend
        .execute_sql("SELECT COUNT(*) FROM main._smelt_fingerprint_sidecar")
        .await
        .expect("count sidecar rows");
    let batch = &batches[0];
    let col = batch
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .expect("COUNT(*) is BIGINT");
    col.value(0) as usize
}

// ── Conformance leg: sidecar-fed run sequence vs. full-refresh oracle ──
// (Phase F3's TDD list: "a sidecar-fed run sequence equals the full-refresh
// oracle after every step — the digest soundness oracle gate F1 names".)
//
// Unlike the tests above, which assert directly on the diff's changed-key
// set, the tests below drive a small downstream "maintained" table through
// several runs using ONLY the sidecar's synthesized changed-key set to
// decide which rows to delete-then-reinsert, and after every step compare
// the maintained table's full content against a from-scratch full-refresh
// recompute of the source's CURRENT state — the multiset-equality oracle
// (`EXCEPT ALL`, checked in both directions, the same shape
// `crates/smelt-runtime/tests/oracle/mod.rs` uses for the property-
// discovery Link-C oracle) so neither an extra nor a missing row can hide.

/// `left_sql`/`right_sql` must each already be a complete `SELECT`
/// statement (a bare table name is not valid on either side of `EXCEPT
/// ALL` — callers use `SELECT * FROM <table>` for a maintained table).
async fn except_all_count(backend: &DuckDbBackend, left_sql: &str, right_sql: &str) -> i64 {
    let sql = format!("SELECT count(*) FROM (({left_sql}) EXCEPT ALL ({right_sql})) AS d");
    let batches = backend
        .execute_sql(&sql)
        .await
        .expect("except all count query");
    let batch = &batches[0];
    let col = batch
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .expect("COUNT(*) is BIGINT");
    col.value(0)
}

/// The full-refresh-oracle equality check: `left_sql` (the maintained
/// table's current content) and `right_sql` (a from-scratch recompute over
/// the source's current state) must be equal multisets — zero rows on
/// either side of `EXCEPT ALL`.
async fn assert_multiset_equal(
    backend: &DuckDbBackend,
    left_sql: &str,
    right_sql: &str,
    context: &str,
) {
    let left_only = except_all_count(backend, left_sql, right_sql).await;
    let right_only = except_all_count(backend, right_sql, left_sql).await;
    assert_eq!(
        (left_only, right_only),
        (0, 0),
        "{context}: the maintained table diverges from the full-refresh oracle (rows only in \
         maintained: {left_only}, rows only in the oracle: {right_only})"
    );
}

/// Apply the sidecar's synthesized `changed_keys` to `maintained_table`:
/// delete every row whose key is in `changed_keys`, then reinsert whatever
/// `source_projection_sql` currently has for those keys — a row the source
/// deleted simply isn't reinserted, a new or edited row lands with its
/// current value. This is the minimal per-key delta application the T3
/// licence union (Phase F5) will later wire into the real maintenance
/// driver; here it is test-local scaffolding so this conformance leg does
/// not depend on that not-yet-landed admission wiring.
async fn apply_changed_keys(
    backend: &DuckDbBackend,
    maintained_table: &str,
    key_column: &str,
    source_projection_sql: &str,
    changed_keys: &[String],
) {
    if changed_keys.is_empty() {
        return;
    }
    let key_list = changed_keys
        .iter()
        .map(|k| format!("'{}'", k.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");
    backend
        .execute_sql(&format!(
            "DELETE FROM {maintained_table} WHERE {key_column} IN ({key_list})"
        ))
        .await
        .expect("delete stale changed keys from the maintained table");
    backend
        .execute_sql(&format!(
            "INSERT INTO {maintained_table} SELECT * FROM ({source_projection_sql}) AS \
             _smelt_conformance_delta WHERE _smelt_conformance_delta.{key_column} IN ({key_list})"
        ))
        .await
        .expect("insert refreshed changed keys into the maintained table");
}

#[tokio::test]
async fn absent_sidecar_yields_whole_table_delta_and_first_refresh_populates_it() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let backend = DuckDbBackend::new(&temp_dir.path().join("test.duckdb"), "main")
        .await
        .unwrap();
    create_source(&backend, 1000).await;

    let changed = diff(&backend).await;
    assert_eq!(
        changed.len(),
        1000,
        "an absent sidecar must report every current source row as changed"
    );

    refresh(&backend, &empty_write_group())
        .await
        .expect("first refresh populates the sidecar");
    assert_eq!(
        sidecar_row_count(&backend).await,
        1000,
        "the refresh must populate exactly one sidecar row per source key"
    );

    // A second diff against the now-populated, unchanged sidecar reports
    // nothing as changed.
    let changed_after_refresh = diff(&backend).await;
    assert!(
        changed_after_refresh.is_empty(),
        "an unchanged source against a populated sidecar must report no changes: \
         {changed_after_refresh:?}"
    );
}

#[tokio::test]
async fn a_two_row_edit_in_a_thousand_row_source_is_detected_exactly() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let backend = DuckDbBackend::new(&temp_dir.path().join("test.duckdb"), "main")
        .await
        .unwrap();
    create_source(&backend, 1000).await;
    diff(&backend).await; // populate the sidecar table (create-if-absent)
    refresh(&backend, &empty_write_group())
        .await
        .expect("populate sidecar baseline");

    backend
        .execute_sql("UPDATE main.dim_users SET tier = 'platinum' WHERE id IN (17, 842)")
        .await
        .expect("edit exactly 2 of 1000 rows");

    let changed = diff(&backend).await;
    let mut sorted = changed;
    sorted.sort();
    assert_eq!(
        sorted,
        vec!["17".to_string(), "842".to_string()],
        "the diff must detect exactly the 2 edited keys out of 1000, no more, no fewer"
    );
}

#[tokio::test]
async fn an_edit_outside_the_p4_projection_yields_an_empty_changed_set() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let backend = DuckDbBackend::new(&temp_dir.path().join("test.duckdb"), "main")
        .await
        .unwrap();
    create_source(&backend, 100).await;
    diff(&backend).await;
    refresh(&backend, &empty_write_group())
        .await
        .expect("populate sidecar baseline");

    // `notes` is not part of the P4 projection (`name`, `tier`) — editing it
    // must never dirty the changed-key set.
    backend
        .execute_sql("UPDATE main.dim_users SET notes = 'edited' WHERE id IN (5, 50, 95)")
        .await
        .expect("edit an out-of-projection column");

    let changed = diff(&backend).await;
    assert!(
        changed.is_empty(),
        "an edit outside the P4 projection must never appear in the changed-key set: {changed:?}"
    );
}

#[tokio::test]
async fn a_deleted_source_row_surfaces_as_changed_and_is_gcd_on_refresh() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let backend = DuckDbBackend::new(&temp_dir.path().join("test.duckdb"), "main")
        .await
        .unwrap();
    create_source(&backend, 10).await;
    diff(&backend).await;
    refresh(&backend, &empty_write_group())
        .await
        .expect("populate sidecar baseline");
    assert_eq!(sidecar_row_count(&backend).await, 10);

    backend
        .execute_sql("DELETE FROM main.dim_users WHERE id = 7")
        .await
        .expect("delete one source row");

    let changed = diff(&backend).await;
    assert_eq!(
        changed,
        vec!["7".to_string()],
        "a deleted source row's key must surface in the changed-key set"
    );

    refresh(&backend, &empty_write_group())
        .await
        .expect("refresh GCs the deleted key");
    assert_eq!(
        sidecar_row_count(&backend).await,
        9,
        "the refresh's GC must drop the deleted key's sidecar row"
    );
}

#[tokio::test]
async fn a_failed_write_leaves_the_sidecar_untouched_so_the_next_diff_recomputes_the_same_delta() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let backend = DuckDbBackend::new(&temp_dir.path().join("test.duckdb"), "main")
        .await
        .unwrap();
    create_source(&backend, 20).await;
    diff(&backend).await;
    refresh(&backend, &empty_write_group())
        .await
        .expect("populate sidecar baseline");

    backend
        .execute_sql("UPDATE main.dim_users SET tier = 'platinum' WHERE id = 3")
        .await
        .expect("edit 1 row");

    let changed_before = diff(&backend).await;
    assert_eq!(changed_before, vec!["3".to_string()]);

    // A "consuming write" that fails — the sidecar refresh riding with it
    // must roll back entirely, leaving the sidecar exactly as it was before
    // this attempt.
    let failing_write_group = StatementGroup {
        statements: vec![smelt_backend::MaintenanceStatement {
            sql: "INSERT INTO main.does_not_exist VALUES (1)".to_string(),
        }],
        transactional: false,
    };
    let result = refresh(&backend, &failing_write_group).await;
    assert!(result.is_err(), "the failed write must surface an error");

    // A re-run's diff must recompute the IDENTICAL changed-key set — proof
    // the sidecar was never half-committed.
    let changed_after_failed_refresh = diff(&backend).await;
    assert_eq!(
        changed_after_failed_refresh,
        vec!["3".to_string()],
        "a failed write must leave the sidecar untouched, so a re-run recomputes the same delta"
    );
}

/// The digest-soundness oracle (`docs/specs/sources.md` §"The fingerprint
/// sidecar" — "Digest": "real content edits ⇒ exactly the edited keys
/// detected; no false-negative 'unchanged' verdict observed"): a sequence
/// of runs, each editing a distinct known key subset, must have its diff
/// exactly match the edited set at every step — never over- nor
/// under-reporting.
#[tokio::test]
async fn a_sidecar_fed_run_sequence_detects_exactly_the_edited_keys_at_every_step() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let backend = DuckDbBackend::new(&temp_dir.path().join("test.duckdb"), "main")
        .await
        .unwrap();
    create_source(&backend, 200).await;
    diff(&backend).await;
    refresh(&backend, &empty_write_group())
        .await
        .expect("run 0: populate sidecar baseline");

    let steps: Vec<(Vec<i64>, &str)> = vec![
        (vec![1], "tier"),
        (vec![2, 3, 4], "tier"),
        (vec![], "tier"), // no edits this step — must detect nothing
        (vec![100, 199], "name"),
        (vec![50], "notes"), // out-of-projection — must detect nothing
    ];

    for (ids, column) in steps {
        if !ids.is_empty() {
            let id_list = ids
                .iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let new_value = match column {
                "tier" => "'edited_tier'",
                "name" => "'edited_name'",
                _ => "'edited_notes'",
            };
            backend
                .execute_sql(&format!(
                    "UPDATE main.dim_users SET {column} = {new_value} WHERE id IN ({id_list})"
                ))
                .await
                .expect("apply step edit");
        }

        let mut changed = diff(&backend).await;
        changed.sort();
        let expected: Vec<String> = if column == "notes" {
            vec![] // outside the P4 projection
        } else {
            let mut e: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
            e.sort();
            e
        };
        assert_eq!(
            changed, expected,
            "step editing {ids:?}.{column} must detect exactly {expected:?}, got {changed:?}"
        );

        refresh(&backend, &empty_write_group())
            .await
            .expect("refresh after each step");
    }
}

/// The conformance leg the Phase F3 TDD list names explicitly: "a sidecar-
/// fed run sequence equals the full-refresh oracle after every step (the
/// digest soundness oracle gate F1 names)". This drives a small downstream
/// "maintained" table through several runs, applying ONLY the sidecar's
/// synthesized changed-key set at each step (never a blind full rebuild),
/// and after every step asserts the maintained table's full content is
/// multiset-equal to a from-scratch full-refresh recompute of the source's
/// current state.
///
/// Run 3 is the regression pin for the NULL-vs-empty-string digest
/// collision (bug 1): `id = 12`'s `tier` transitions from the empty string
/// to NULL. Pre-fix, DuckDB's `CONCAT` silently drops a NULL argument, so
/// `CONCAT(name, sep, NULL)` and `CONCAT(name, sep, '')` hashed identically
/// — the diff would have omitted `id = 12` from the changed-key set,
/// `apply_changed_keys` would never have touched it, and the maintained
/// table would have kept the stale empty-string value forever: a genuine,
/// silent divergence from the oracle that only a full-content comparison
/// (not a changed-key-set assertion) can catch.
#[tokio::test]
async fn sidecar_fed_run_sequence_matches_full_refresh_oracle_at_every_step() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let backend = DuckDbBackend::new(&temp_dir.path().join("test.duckdb"), "main")
        .await
        .unwrap();
    create_source(&backend, 30).await;

    let maintained = "main.maintained_users";
    let maintained_select = format!("SELECT * FROM {maintained}");
    let oracle_sql = "SELECT id, name, tier FROM main.dim_users";
    let key_column = "id";

    // Run 0: absent sidecar ⇒ whole-table delta ⇒ from-scratch build.
    let changed = diff(&backend).await;
    assert_eq!(
        changed.len(),
        30,
        "run 0: every row must be seen as changed"
    );
    backend
        .execute_sql(&format!("CREATE TABLE {maintained} AS {oracle_sql}"))
        .await
        .expect("run 0: initial full build of the maintained table");
    refresh(&backend, &empty_write_group())
        .await
        .expect("run 0: populate sidecar baseline");
    assert_multiset_equal(
        &backend,
        &maintained_select,
        oracle_sql,
        "after run 0 (initial build)",
    )
    .await;

    // Run 1: an ordinary in-projection edit.
    backend
        .execute_sql("UPDATE main.dim_users SET tier = 'platinum' WHERE id IN (3, 9)")
        .await
        .expect("run 1: apply edit");
    let changed = diff(&backend).await;
    apply_changed_keys(&backend, maintained, key_column, oracle_sql, &changed).await;
    refresh(&backend, &empty_write_group())
        .await
        .expect("run 1: refresh sidecar");
    assert_multiset_equal(
        &backend,
        &maintained_select,
        oracle_sql,
        "after run 1 (ordinary edit)",
    )
    .await;

    // Run 2: establish a baseline empty-string `tier` for id=12 — the
    // pre-transition state the regression step below compares against.
    backend
        .execute_sql("UPDATE main.dim_users SET tier = '' WHERE id = 12")
        .await
        .expect("run 2: set tier to empty string");
    let changed = diff(&backend).await;
    assert!(
        changed.contains(&"12".to_string()),
        "run 2: the empty-string edit must be detected as changed"
    );
    apply_changed_keys(&backend, maintained, key_column, oracle_sql, &changed).await;
    refresh(&backend, &empty_write_group())
        .await
        .expect("run 2: refresh sidecar");
    assert_multiset_equal(
        &backend,
        &maintained_select,
        oracle_sql,
        "after run 2 (empty-string baseline)",
    )
    .await;

    // Run 3 — THE regression step: id=12's tier goes from '' to NULL.
    backend
        .execute_sql("UPDATE main.dim_users SET tier = NULL WHERE id = 12")
        .await
        .expect("run 3: transition tier from empty string to NULL");
    let changed = diff(&backend).await;
    assert!(
        changed.contains(&"12".to_string()),
        "run 3: an empty-string-to-NULL transition must be detected as changed — this is the \
         exact NULL-vs-empty-string digest collision bug 1 fixes"
    );
    apply_changed_keys(&backend, maintained, key_column, oracle_sql, &changed).await;
    refresh(&backend, &empty_write_group())
        .await
        .expect("run 3: refresh sidecar");
    assert_multiset_equal(
        &backend,
        &maintained_select,
        oracle_sql,
        "after run 3 (empty-string -> NULL transition, the digest-collision regression step)",
    )
    .await;

    // Run 4: NULL transitions back to a real value.
    backend
        .execute_sql("UPDATE main.dim_users SET tier = 'gold' WHERE id = 12")
        .await
        .expect("run 4: transition tier from NULL back to a real value");
    let changed = diff(&backend).await;
    assert!(
        changed.contains(&"12".to_string()),
        "run 4: a NULL-to-real-value transition must be detected as changed"
    );
    apply_changed_keys(&backend, maintained, key_column, oracle_sql, &changed).await;
    refresh(&backend, &empty_write_group())
        .await
        .expect("run 4: refresh sidecar");
    assert_multiset_equal(
        &backend,
        &maintained_select,
        oracle_sql,
        "after run 4 (NULL -> real value)",
    )
    .await;

    // Run 5: a deletion — the key must surface as changed, and the
    // maintained table must lose the row, matching an oracle that no
    // longer has it either.
    backend
        .execute_sql("DELETE FROM main.dim_users WHERE id = 20")
        .await
        .expect("run 5: delete a source row");
    let changed = diff(&backend).await;
    assert!(
        changed.contains(&"20".to_string()),
        "run 5: a deleted source row's key must surface as changed"
    );
    apply_changed_keys(&backend, maintained, key_column, oracle_sql, &changed).await;
    refresh(&backend, &empty_write_group())
        .await
        .expect("run 5: refresh sidecar");
    assert_multiset_equal(
        &backend,
        &maintained_select,
        oracle_sql,
        "after run 5 (deletion)",
    )
    .await;
}

/// Regression for the NULL-digest crash (bug 2): pre-fix, a single-column
/// P4 projection built `sha256(CAST(col AS VARCHAR))` directly (no
/// `CONCAT`, since there is only one column to digest), so a projected
/// value transitioning to NULL produced `sha256(NULL) = NULL` in DuckDB —
/// which then violated the sidecar's `digest VARCHAR NOT NULL` column on
/// upsert, crashing the refresh outright rather than merely producing a
/// wrong result. This drives the same run-sequence-vs-full-refresh-oracle
/// conformance shape as the multi-column test above, but with a
/// ONE-column projection, so the fix's `COALESCE` is the only thing
/// standing between a NULL projected value and a bare `sha256(NULL)`.
#[tokio::test]
async fn single_column_projection_survives_a_null_transition_and_matches_the_oracle() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let backend = DuckDbBackend::new(&temp_dir.path().join("test.duckdb"), "main")
        .await
        .unwrap();
    create_source(&backend, 10).await;

    let single_projection = single_column_projection();
    let maintained = "main.maintained_tier_only";
    let maintained_select = format!("SELECT * FROM {maintained}");
    let oracle_sql = "SELECT id, tier FROM main.dim_users";
    let key_column = "id";

    let changed = diff_with_projection(&backend, &single_projection).await;
    assert_eq!(
        changed.len(),
        10,
        "run 0: every row must be seen as changed"
    );
    backend
        .execute_sql(&format!("CREATE TABLE {maintained} AS {oracle_sql}"))
        .await
        .expect("run 0: initial full build");
    refresh_with_projection(&backend, &single_projection, &empty_write_group())
        .await
        .expect("run 0: populate sidecar baseline");
    assert_multiset_equal(
        &backend,
        &maintained_select,
        oracle_sql,
        "after run 0 (initial build)",
    )
    .await;

    // The regression step: tier goes to NULL for a row.
    backend
        .execute_sql("UPDATE main.dim_users SET tier = NULL WHERE id = 4")
        .await
        .expect("edit: set tier to NULL");
    let changed = diff_with_projection(&backend, &single_projection).await;
    assert!(
        changed.contains(&"4".to_string()),
        "a NULL projected value must still be detected as changed, not silently dropped"
    );
    apply_changed_keys(&backend, maintained, key_column, oracle_sql, &changed).await;
    // Pre-fix, this refresh would fail here: `sha256(NULL)` upserted into
    // the sidecar's `digest VARCHAR NOT NULL` column violates the
    // constraint and the whole run crashes.
    refresh_with_projection(&backend, &single_projection, &empty_write_group())
        .await
        .expect(
            "refresh must survive a NULL single-column projection value, not crash on a NOT \
             NULL constraint violation",
        );
    assert_multiset_equal(
        &backend,
        &maintained_select,
        oracle_sql,
        "after the NULL transition",
    )
    .await;

    // Transitioning back to a real value keeps working too.
    backend
        .execute_sql("UPDATE main.dim_users SET tier = 'restored' WHERE id = 4")
        .await
        .expect("edit: restore a real value");
    let changed = diff_with_projection(&backend, &single_projection).await;
    assert!(changed.contains(&"4".to_string()));
    apply_changed_keys(&backend, maintained, key_column, oracle_sql, &changed).await;
    refresh_with_projection(&backend, &single_projection, &empty_write_group())
        .await
        .expect("refresh after restoring a real value");
    assert_multiset_equal(
        &backend,
        &maintained_select,
        oracle_sql,
        "after restoring a real value",
    )
    .await;
}

// ── Phase F4: sidecar invalidation ──────────────────────────────────────
// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase F4
// — "Sidecar invalidation")
//
// A stale-stamped partition must always degrade to the SAME whole-table
// delta an absent sidecar produces above — never a narrower, partially-
// trusted comparison, and never a silent skip. The tests below drive every
// invalidation trigger `compute_fingerprint_sidecar_stamp` covers
// (projection identity, a source column entering/not entering the
// projection, model-definition provenance, and a hand-corrupted stamp) and
// assert the degradation target directly: the changed-key set widens to
// cover every currently-existing source row, exactly like a first run
// against a never-populated sidecar.

// Per-thread WARN buffer. `None` means "this thread is not capturing";
// `Some` means a `capture_warnings` call is active on this thread and every
// WARN event emitted here lands in the buffer. Keeping the buffer
// thread-local (rather than handing a shared `Arc<Mutex<Vec<_>>>` to a
// *scoped* subscriber) is what makes the capture safe under the
// multi-threaded test harness: see `install_capturing_subscriber` below.
thread_local! {
    static CAPTURED_WARNINGS: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/// A minimal `tracing_subscriber::Layer` recording every WARN-level event's
/// formatted `message` field into the emitting thread's buffer (if that
/// thread is capturing) — used by the corrupted-stamp test below to prove
/// the mismatch is logged loudly (`tracing::warn!`), never silently
/// swallowed.
struct CapturingLayer;

/// Installs the capturing layer as the process-wide *global* default
/// subscriber, exactly once per test binary.
///
/// A global default is load-bearing, not incidental. `tracing` caches each
/// callsite's `Interest` **globally**, and a brand-new callsite's interest is
/// computed from whatever dispatcher is default *on the thread that happens
/// to hit it first* (`tracing_core::callsite::register` →
/// `Dispatchers::rebuilder` → `Rebuilder::JustOne` →
/// `dispatcher::get_default`). With a thread-scoped
/// `tracing::subscriber::set_default`, a sibling test running concurrently on
/// another thread can therefore reach the `warn!` callsite first, see no
/// subscriber, and have `Interest::never` cached for it process-wide — after
/// which the capturing test's own `warn!` is elided before it is ever
/// dispatched, and the capture comes back empty. Registering one global
/// subscriber up front means every thread resolves the callsite through this
/// layer, so interest is `always` no matter who wins the race, while the
/// thread-local buffer keeps each test's captured messages its own.
fn install_capturing_subscriber() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let subscriber = tracing_subscriber::registry().with(CapturingLayer);
        tracing::subscriber::set_global_default(subscriber)
            .expect("install the WARN-capturing global subscriber");
    });
}

/// Runs `future` with WARN capture enabled on this thread, returning its
/// output alongside every WARN message the future logged.
async fn capture_warnings<F: std::future::Future>(future: F) -> (F::Output, Vec<String>) {
    install_capturing_subscriber();
    CAPTURED_WARNINGS.with(|c| *c.borrow_mut() = Some(Vec::new()));
    let output = future.await;
    let captured = CAPTURED_WARNINGS
        .with(|c| c.borrow_mut().take())
        .expect("WARN capture buffer must still be installed on this thread");
    (output, captured)
}

impl<S> tracing_subscriber::Layer<S> for CapturingLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if *event.metadata().level() != tracing::Level::WARN {
            return;
        }
        struct MessageVisitor<'a>(&'a mut String);
        impl tracing::field::Visit for MessageVisitor<'_> {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    use std::fmt::Write;
                    let _ = write!(self.0, "{value:?}");
                }
            }
        }
        let mut message = String::new();
        event.record(&mut MessageVisitor(&mut message));
        CAPTURED_WARNINGS.with(|c| {
            if let Some(buffer) = c.borrow_mut().as_mut() {
                buffer.push(message);
            }
        });
    }
}

#[tokio::test]
async fn changing_the_projection_yields_a_fresh_partition_and_a_whole_table_delta() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let backend = DuckDbBackend::new(&temp_dir.path().join("test.duckdb"), "main")
        .await
        .unwrap();
    create_source(&backend, 50).await;

    // Establish a baseline under the original 2-column projection.
    diff(&backend).await;
    refresh(&backend, &empty_write_group())
        .await
        .expect("populate baseline sidecar partition");
    let changed_unchanged_projection = diff(&backend).await;
    assert!(
        changed_unchanged_projection.is_empty(),
        "an unchanged source under the SAME projection must report no changes: \
         {changed_unchanged_projection:?}"
    );

    // A projection change (the model now digests only `tier`, dropping
    // `name`) lands in a fresh, unpopulated partition by construction — the
    // next diff under the NEW projection must see every row as changed,
    // exactly like the very first diff against an absent sidecar.
    let new_projection = single_column_projection();
    let changed_new_projection = diff_with_projection(&backend, &new_projection).await;
    assert_eq!(
        changed_new_projection.len(),
        50,
        "a projection change must yield the whole-table delta under the new projection's \
         fresh partition, never a partial or empty comparison against the old partition's data"
    );

    refresh_with_projection(&backend, &new_projection, &empty_write_group())
        .await
        .expect("the sidecar rebuilds under the new projection");
    let changed_after_rebuild = diff_with_projection(&backend, &new_projection).await;
    assert!(
        changed_after_rebuild.is_empty(),
        "once rebuilt under the new projection, an unchanged source reports no changes: \
         {changed_after_rebuild:?}"
    );
}

#[tokio::test]
async fn a_source_column_entering_the_projection_invalidates_one_that_doesnt_does_not() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let backend = DuckDbBackend::new(&temp_dir.path().join("test.duckdb"), "main")
        .await
        .unwrap();
    create_source(&backend, 20).await;
    diff(&backend).await;
    refresh(&backend, &empty_write_group())
        .await
        .expect("populate baseline sidecar partition (name, tier)");

    // A projection widened to also cover `notes` (previously OUTSIDE the
    // projection) is a different `projection_identity` — a fresh partition,
    // whole-table delta.
    let widened_projection = Projection::Columns(
        ["name".to_string(), "tier".to_string(), "notes".to_string()]
            .into_iter()
            .collect(),
    );
    let changed_widened = diff_with_projection(&backend, &widened_projection).await;
    assert_eq!(
        changed_widened.len(),
        20,
        "a source column entering the P4 projection must invalidate: the widened projection's \
         fresh partition reports every row as changed"
    );

    // A projection that does NOT change at all must still report no
    // changes against the unmodified baseline partition — proving the
    // widened-projection result above is really about the projection
    // change, not some unconditional every-diff-is-a-miss bug.
    let changed_same_projection = diff(&backend).await;
    assert!(
        changed_same_projection.is_empty(),
        "a projection that did not change must not invalidate: {changed_same_projection:?}"
    );
}

/// The trigger that CANNOT be detected by `projection_identity` alone: two
/// different model SQL texts can resolve to the identical P4 projection
/// (the P4 derivation over `MODEL_SQL` and `EDITED_MODEL_SQL` — both select
/// `name`/`tier` — is not re-run by this test; it stands in for "the P4
/// derivation happened to land on the same column set both times"). Only
/// the stamp's model-hash component
/// (`compute_fingerprint_sidecar_stamp`) can catch this — proving the
/// model-definition-provenance component is load-bearing, not decorative.
#[tokio::test]
async fn a_model_definition_edit_holding_the_projection_fixed_invalidates_the_whole_partition() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let backend = DuckDbBackend::new(&temp_dir.path().join("test.duckdb"), "main")
        .await
        .unwrap();
    create_source(&backend, 40).await;

    // Baseline under MODEL_SQL.
    diff_with_projection_and_model(&backend, &projection(), MODEL_SQL).await;
    refresh_with_projection_and_model(&backend, &projection(), MODEL_SQL, &empty_write_group())
        .await
        .expect("populate baseline sidecar partition under MODEL_SQL");
    let changed_same_model =
        diff_with_projection_and_model(&backend, &projection(), MODEL_SQL).await;
    assert!(
        changed_same_model.is_empty(),
        "an unchanged source under the SAME model definition must report no changes: \
         {changed_same_model:?}"
    );

    // The model recipe is edited — the SAME projection identity
    // (`cols:name,tier`), but a different model SQL text. Not a single
    // source row's content changed, yet the diff must report EVERY row as
    // changed: a stale partition never narrows to "just the rows I can
    // prove are affected", it always widens to everything.
    let changed_after_edit =
        diff_with_projection_and_model(&backend, &projection(), EDITED_MODEL_SQL).await;
    assert_eq!(
        changed_after_edit.len(),
        40,
        "a model-definition edit holding the projection fixed must invalidate the WHOLE \
         partition (widen-never-narrow), even though no source content changed: \
         {changed_after_edit:?}"
    );

    // Refreshing under the edited model SQL re-stamps every row, so a
    // subsequent diff under the SAME edited model reports no changes.
    refresh_with_projection_and_model(
        &backend,
        &projection(),
        EDITED_MODEL_SQL,
        &empty_write_group(),
    )
    .await
    .expect("the sidecar self-heals under the edited model definition");
    let changed_after_rebuild =
        diff_with_projection_and_model(&backend, &projection(), EDITED_MODEL_SQL).await;
    assert!(
        changed_after_rebuild.is_empty(),
        "once rebuilt under the edited model definition, an unchanged source reports no \
         changes: {changed_after_rebuild:?}"
    );
}

/// A hand-corrupted stamp (simulating on-disk corruption, or a stamp
/// written by a build with a different `FINGERPRINT_SIDECAR_DIGEST_VERSION`)
/// must never be trusted: the diff treats the corrupted rows as absent
/// (whole-table delta) and logs loudly — `tracing::warn!`, never a silent
/// skip, never a `println!` (the workspace's `no_println_in_libraries`
/// gate would catch a `println!` in production code, but the "loud, not
/// silent" contract itself is asserted here, directly).
#[tokio::test]
async fn a_hand_corrupted_stamp_is_detected_treated_as_absent_and_logged_loudly() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let backend = DuckDbBackend::new(&temp_dir.path().join("test.duckdb"), "main")
        .await
        .unwrap();
    create_source(&backend, 15).await;
    diff(&backend).await;
    refresh(&backend, &empty_write_group())
        .await
        .expect("populate baseline sidecar partition");

    // Hand-corrupt every stored stamp directly — simulating disk
    // corruption or a stamp written under a stale digest-version/model
    // hash, bypassing the normal refresh path entirely.
    backend
        .execute_sql("UPDATE main._smelt_fingerprint_sidecar SET stamp = 'corrupted-garbage-stamp'")
        .await
        .expect("hand-corrupt the stored stamp");

    let (changed, captured) = capture_warnings(diff(&backend)).await;

    assert_eq!(
        changed.len(),
        15,
        "a corrupted stamp must be treated as absent: the diff must report every current \
         source row as changed, the SAME whole-table delta an absent sidecar produces — never \
         a narrower comparison and never a silent skip: {changed:?}"
    );
    assert!(
        captured
            .iter()
            .any(|m| m.contains("stamp mismatch") || m.contains("stamp")),
        "a corrupted/mismatched stamp must be logged loudly (tracing::warn!), never silently \
         trusted or silently skipped; captured WARN messages: {captured:?}"
    );
}

/// The conformance leg the Phase F4 TDD list names explicitly: "an
/// invalidation mid-run-sequence (recipe edit) still matches the
/// full-refresh oracle on every subsequent step." Mirrors the multi-run
/// conformance shape above, but inserts a model-definition edit mid-
/// sequence (holding the projection fixed) — the changed-key set widens to
/// the whole table at that step, and applying it (even though it is a
/// strict superset of what actually changed) must still leave the
/// maintained table multiset-equal to the from-scratch oracle.
#[tokio::test]
async fn sidecar_fed_run_sequence_survives_a_mid_sequence_model_recipe_edit_and_matches_the_oracle()
{
    let temp_dir = tempfile::TempDir::new().unwrap();
    let backend = DuckDbBackend::new(&temp_dir.path().join("test.duckdb"), "main")
        .await
        .unwrap();
    create_source(&backend, 25).await;

    let maintained = "main.maintained_users_recipe_edit";
    let maintained_select = format!("SELECT * FROM {maintained}");
    let oracle_sql = "SELECT id, name, tier FROM main.dim_users";
    let key_column = "id";

    // Run 0: absent sidecar ⇒ whole-table delta ⇒ from-scratch build, under
    // MODEL_SQL.
    let changed = diff_with_projection_and_model(&backend, &projection(), MODEL_SQL).await;
    assert_eq!(
        changed.len(),
        25,
        "run 0: every row must be seen as changed"
    );
    backend
        .execute_sql(&format!("CREATE TABLE {maintained} AS {oracle_sql}"))
        .await
        .expect("run 0: initial full build of the maintained table");
    refresh_with_projection_and_model(&backend, &projection(), MODEL_SQL, &empty_write_group())
        .await
        .expect("run 0: populate sidecar baseline");
    assert_multiset_equal(
        &backend,
        &maintained_select,
        oracle_sql,
        "after run 0 (initial build)",
    )
    .await;

    // Run 1: an ordinary in-projection edit, still under MODEL_SQL.
    backend
        .execute_sql("UPDATE main.dim_users SET tier = 'platinum' WHERE id IN (2, 7)")
        .await
        .expect("run 1: apply edit");
    let changed = diff_with_projection_and_model(&backend, &projection(), MODEL_SQL).await;
    let mut sorted = changed.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        vec!["2".to_string(), "7".to_string()],
        "run 1: an ordinary edit under an unchanged recipe must detect exactly the edited keys"
    );
    apply_changed_keys(&backend, maintained, key_column, oracle_sql, &changed).await;
    refresh_with_projection_and_model(&backend, &projection(), MODEL_SQL, &empty_write_group())
        .await
        .expect("run 1: refresh sidecar");
    assert_multiset_equal(
        &backend,
        &maintained_select,
        oracle_sql,
        "after run 1 (ordinary edit)",
    )
    .await;

    // Run 2 — THE recipe-edit step: the model definition changes (holding
    // the projection fixed), no source content changes at all. The
    // widen-never-narrow diff must report EVERY row as changed; applying
    // that (a strict superset of the empty real delta) must still leave
    // the maintained table exactly equal to the oracle.
    let changed = diff_with_projection_and_model(&backend, &projection(), EDITED_MODEL_SQL).await;
    assert_eq!(
        changed.len(),
        25,
        "run 2: a mid-sequence model-definition edit must widen to the whole table, even \
         though no source content changed: {changed:?}"
    );
    apply_changed_keys(&backend, maintained, key_column, oracle_sql, &changed).await;
    refresh_with_projection_and_model(
        &backend,
        &projection(),
        EDITED_MODEL_SQL,
        &empty_write_group(),
    )
    .await
    .expect("run 2: refresh sidecar under the edited recipe");
    assert_multiset_equal(
        &backend,
        &maintained_select,
        oracle_sql,
        "after run 2 (the mid-sequence recipe-edit invalidation step)",
    )
    .await;

    // Run 3: back to an ordinary in-projection edit, now under
    // EDITED_MODEL_SQL — proving the sidecar is fully usable again after
    // the recipe-edit invalidation, not permanently wedged.
    backend
        .execute_sql("UPDATE main.dim_users SET tier = 'silver' WHERE id = 12")
        .await
        .expect("run 3: apply edit");
    let changed = diff_with_projection_and_model(&backend, &projection(), EDITED_MODEL_SQL).await;
    assert_eq!(
        changed,
        vec!["12".to_string()],
        "run 3: after the recipe-edit invalidation is absorbed, the sidecar must go back to \
         detecting exactly the edited keys, not the whole table again: {changed:?}"
    );
    apply_changed_keys(&backend, maintained, key_column, oracle_sql, &changed).await;
    refresh_with_projection_and_model(
        &backend,
        &projection(),
        EDITED_MODEL_SQL,
        &empty_write_group(),
    )
    .await
    .expect("run 3: refresh sidecar");
    assert_multiset_equal(
        &backend,
        &maintained_select,
        oracle_sql,
        "after run 3 (back to steady state post-invalidation)",
    )
    .await;
}
