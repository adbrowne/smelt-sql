//! T4 — the fingerprint sidecar build + synthesized external change feed
//! (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase
//! F3; `docs/specs/sources.md` §"The fingerprint sidecar").
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
//!   recomputes the identical changed-key set (no half-committed digest).

use smelt_backend::{Backend, BackendError, StatementGroup};
use smelt_backend_duckdb::DuckDbBackend;
use smelt_logical::analysis::fingerprint::Projection;
use smelt_runtime::maintenance_driver::{
    diff_fingerprint_sidecar_changed_keys, refresh_fingerprint_sidecar,
};

const SOURCE_ADDRESS: &str = "smelt.sources.dim_users";
const SOURCE_TABLE: &str = "main.dim_users";

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

async fn diff_with_projection(backend: &DuckDbBackend, projection: &Projection) -> Vec<String> {
    diff_fingerprint_sidecar_changed_keys(
        backend,
        "main",
        SOURCE_ADDRESS,
        SOURCE_TABLE,
        &["id".to_string()],
        projection,
        &all_columns(),
    )
    .await
    .expect("diff")
}

async fn diff(backend: &DuckDbBackend) -> Vec<String> {
    diff_with_projection(backend, &projection()).await
}

async fn refresh_with_projection(
    backend: &DuckDbBackend,
    projection: &Projection,
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
        write_group,
    )
    .await
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
