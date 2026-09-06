/// T3 over external sources — the point-lookup enrichment recompute
/// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase
/// F5): `examples/timeseries/models/daily_events_enriched.sql`'s
/// `raw.users` source now declares `unique_key: [user_id]` +
/// `referential_integrity: [user_id]`
/// (`examples/timeseries/models/sources/raw/users.yml`), so its
/// `{user_name}` `UpstreamMutation` cell's enrichment join now closes P1
/// (`skeleton_closure_pinned.rs`'s discriminating pair) and the fingerprint
/// sidecar's synthesized changed-key set (F3/F4) licenses a delta-restricted
/// recompute (`choice::resolve_recompute_restriction` — the SAME gate E3
/// built for model edges, unioned onto this external-source cell by
/// `derive::mutation_enrichment_closure`).
///
/// **Known production gap** (documented here, not silently worked around):
/// this restriction is not yet dispatched live by `execute.rs`'s regular
/// incremental batch loop — `resolve_live_column_scoped_cell`/
/// `execute_column_scoped_merge_full`'s call site in `crates/smelt-runtime/
/// src/execute.rs` is outside this phase's allowed files (only `crates/
/// smelt-logical/src/maintenance/{derive,choice,emit}.rs` and this test
/// file are). Mirroring `real_fixture_daily_events_status_would_admit_
/// partition_local_yes_cell` above and `fingerprint_sidecar.rs`'s own
/// `apply_changed_keys` doc comment ("this is the minimal per-key delta
/// application the T3 licence union (Phase F5) will later wire into the
/// real maintenance driver; here it is test-local scaffolding"), these
/// tests prove the mechanism — the derived plan cell, the sidecar-derived
/// exact delta, the licence decision, and the emitted delta-restricted
/// statement — is correctly engineered end to end against a real DuckDB
/// backend, driven directly rather than through `execute_project`.
use std::collections::BTreeMap;
use std::path::Path;

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_logical::analysis::fingerprint::Projection;
use smelt_logical::maintenance::choice::{
    enrichment_restrict_column, resolve_recompute_restriction, RecomputeRestriction,
};
use smelt_logical::maintenance::derive::{
    derive_maintenance_plan_with_referential_integrity, ModelInputs,
};
use smelt_logical::maintenance::emit::{
    emit_count_preservation_probe, emit_delete_insert_delta_restricted, MaintenanceDialect, Region,
};
use smelt_logical::maintenance::{
    Grain, MutationProfile, OutputSpec, RowPreservation, SkeletonSourceClosure, SourceFacts,
    Trigger,
};
use smelt_runtime::maintenance_driver::{
    diff_fingerprint_sidecar_changed_keys, execute_delete_insert_with_delta_restriction,
    refresh_fingerprint_sidecar, RestrictionDeltaSource,
};

/// Seed the observed-delta table for `upstream_model`'s `[window_start,
/// window_end)` with `changed_keys`, mirroring `delta_restricted_
/// recompute.rs`'s own helper — the SAME `_smelt_observed_delta` table
/// `read_observed_delta_changed_keys` reads.
async fn record_observed_delta(
    backend: &DuckDbBackend,
    schema: &str,
    upstream_model: &str,
    window_start: &str,
    window_end: &str,
    changed_keys: &[&str],
) {
    let ensure = smelt_state::ddl_duckdb::generate_observed_delta_table_ddl(schema);
    backend
        .execute_sql(&ensure)
        .await
        .expect("ensure observed-delta table");
    let keys_list = changed_keys
        .iter()
        .map(|k| format!("('{k}', NULL)"))
        .collect::<Vec<_>>()
        .join(", ");
    let changed_keys_query =
        format!("SELECT * FROM (VALUES {keys_list}) AS t(delta_key, delta_partition)");
    let upsert = smelt_state::ddl_duckdb::generate_observed_delta_upsert_sql(
        schema,
        upstream_model,
        window_start,
        window_end,
        &changed_keys_query,
    );
    backend
        .execute_sql(&upsert)
        .await
        .expect("record observed delta");
}

/// The real fixture's SQL body (frontmatter stripped), read straight off
/// disk so this suite can never silently drift from the file
/// `skeleton_closure_pinned.rs` also pins.
fn model_sql_body() -> (smelt_core::ModelMetadata, String) {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");
    let text = std::fs::read_to_string(project_dir.join("models/daily_events_enriched.sql"))
        .expect("read daily_events_enriched.sql");
    let smelt_core::FileMetadata::Single {
        metadata,
        sql_offset,
    } = smelt_core::extract_file_metadata(&text).expect("parse frontmatter")
    else {
        panic!("daily_events_enriched.sql must be a single-model file");
    };
    (*metadata, text[sql_offset..].to_string())
}

/// `raw.users`' declared `unique_key: [user_id]` doubles as
/// `SourceFacts::unique_key` (P1 conjunct 3's one-to-one fact) — a
/// SourceFacts list built by hand, not `smelt-db::build_source_facts`
/// (which does not populate `unique_key` yet, per `execute.rs`'s own
/// documented gap), matching `real_fixture_daily_events_status_would_
/// admit_partition_local_yes_cell`'s established pattern above.
fn source_facts() -> Vec<SourceFacts> {
    vec![
        SourceFacts {
            name: "raw.events".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: None,
            unique_key: vec!["event_id".to_string()],
            allow_full_scan: false,
        },
        SourceFacts {
            name: "raw.users".to_string(),
            mutation: MutationProfile::MutableSnapshot,
            partition_col: None,
            unique_key: vec!["user_id".to_string()],
            allow_full_scan: true,
        },
    ]
}

/// Derive the plan and return the `{user_name}` `UpstreamMutation`
/// cell's own `skeleton_source_closure` verdict — the P1 wiring this
/// phase's `derive::mutation_enrichment_closure` adds.
fn user_name_cell_closure() -> Option<SkeletonSourceClosure> {
    let (metadata, sql_body) = model_sql_body();
    let sources = source_facts();
    let partition_col = metadata
        .timeseries
        .as_ref()
        .map(|t| t.partition_column.clone());
    let skeleton = smelt_logical::maintenance::skeleton::skeleton_columns(
        &sql_body,
        &[],
        partition_col.as_deref(),
    );
    let grouping =
        smelt_logical::maintenance::grouping::derive_column_groups(&sql_body, &sources, &skeleton);
    assert!(
        grouping.degenerate.is_empty(),
        "expected no degenerate column-group collapses: {:?}",
        grouping.degenerate
    );

    let inputs = ModelInputs {
        sql: &sql_body,
        output: OutputSpec {
            table: "daily_events_enriched".to_string(),
            grain: Grain::Partition {
                partition_col: partition_col.unwrap_or_default(),
            },
            skeleton_columns: skeleton,
        },
        sources,
        column_groups: grouping.groups,
        fold: None,
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    };

    let mut source_ri = BTreeMap::new();
    source_ri.insert("raw.users".to_string(), vec!["user_id".to_string()]);

    let trigger = Trigger::UpstreamMutation {
        source: "raw.users".to_string(),
    };
    let plan = derive_maintenance_plan_with_referential_integrity(
        &inputs,
        std::slice::from_ref(&trigger),
        &source_ri,
    );
    assert!(
        plan.refusals.is_empty(),
        "expected no admission refusals: {:?}",
        plan.refusals
    );
    let cell = plan
        .cell_for(&trigger)
        .unwrap_or_else(|| panic!("no cell admitted for {trigger:?}: {plan:#?}"));
    cell.skeleton_source_closure.clone()
}

/// The declared-facts variant of the real fixture's `{user_name}` cell
/// closes P1 through `derive_maintenance_plan_with_referential_
/// integrity` — the same verdict `skeleton_closure_pinned.rs` proves
/// directly against `skeleton_source_closure`, now reached through the
/// full plan-derivation path (`ModelInputs` → `derive_mutation` →
/// `mutation_enrichment_closure`) a real caller would use.
#[test]
fn closure_admits_and_restrict_column_resolves() {
    let closure = user_name_cell_closure();
    assert_eq!(
        closure,
        Some(SkeletonSourceClosure::Closed {
            row_preservation:
                smelt_logical::maintenance::RowPreservation::DeclaredReferentialIntegrity {
                    source: "raw.users".to_string()
                }
        })
    );

    let dimension_key = ["user_id".to_string()];
    let restrict_column = enrichment_restrict_column(&dimension_key);
    assert_eq!(restrict_column, Some("user_id"));

    // Absent an RI fact (`derive_maintenance_plan`'s own default path),
    // the SAME cell shape must still carry no closure verdict at all —
    // proving the opt-in wiring is additive, never a default-on change.
    let (metadata, sql_body) = model_sql_body();
    let sources = source_facts();
    let partition_col = metadata
        .timeseries
        .as_ref()
        .map(|t| t.partition_column.clone());
    let skeleton = smelt_logical::maintenance::skeleton::skeleton_columns(
        &sql_body,
        &[],
        partition_col.as_deref(),
    );
    let grouping =
        smelt_logical::maintenance::grouping::derive_column_groups(&sql_body, &sources, &skeleton);
    let inputs = ModelInputs {
        sql: &sql_body,
        output: OutputSpec {
            table: "daily_events_enriched".to_string(),
            grain: Grain::Partition {
                partition_col: partition_col.unwrap_or_default(),
            },
            skeleton_columns: skeleton,
        },
        sources,
        column_groups: grouping.groups,
        fold: None,
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    };
    let trigger = Trigger::UpstreamMutation {
        source: "raw.users".to_string(),
    };
    let default_plan = smelt_logical::maintenance::derive::derive_maintenance_plan(
        &inputs,
        std::slice::from_ref(&trigger),
    );
    let default_cell = default_plan.cell_for(&trigger).expect("cell admitted");
    assert_eq!(
        default_cell.skeleton_source_closure, None,
        "derive_maintenance_plan (no RI facts supplied) must stay byte-identical to its \
         pre-F5 behaviour — None, never an attempted-and-open verdict"
    );
}

/// The digest columns (`user_name` only, per `analysis::fingerprint::
/// fingerprint_projection`'s P4 derivation) a fingerprint sidecar
/// digests over `raw.users` for this model.
fn projection() -> Projection {
    Projection::Columns(["user_name".to_string()].into_iter().collect())
}

fn all_users_columns() -> Vec<String> {
    vec![
        "user_id".to_string(),
        "user_name".to_string(),
        "signup_date".to_string(),
    ]
}

fn empty_write_group() -> smelt_backend::StatementGroup {
    smelt_backend::StatementGroup {
        statements: vec![],
        transactional: false,
    }
}

async fn seed(backend: &DuckDbBackend) {
    backend
        .execute_sql(
            "CREATE TABLE main.sources_raw_events (event_id INTEGER, user_id INTEGER, \
             event_type VARCHAR, event_timestamp TIMESTAMP)",
        )
        .await
        .expect("create events source table");
    backend
        .execute_sql(
            "INSERT INTO main.sources_raw_events VALUES \
             (1, 1, 'login', TIMESTAMP '2025-01-10 08:00:00'), \
             (2, 1, 'click', TIMESTAMP '2025-01-10 09:00:00'), \
             (3, 2, 'login', TIMESTAMP '2025-01-10 10:00:00'), \
             (4, 2, 'click', TIMESTAMP '2025-01-10 11:00:00'), \
             (5, 3, 'login', TIMESTAMP '2025-01-10 12:00:00'), \
             (6, 3, 'click', TIMESTAMP '2025-01-10 13:00:00')",
        )
        .await
        .expect("seed events");
    backend
        .execute_sql(
            "CREATE TABLE main.sources_raw_users (user_id INTEGER, user_name VARCHAR, \
             signup_date DATE)",
        )
        .await
        .expect("create users source table");
    backend
        .execute_sql(
            "INSERT INTO main.sources_raw_users VALUES \
             (1, 'Alice', DATE '2025-01-01'), \
             (2, 'Bob', DATE '2025-01-02'), \
             (3, 'Carol', DATE '2025-01-03')",
        )
        .await
        .expect("seed users");
}

fn enrichment_select(events_table: &str, users_table: &str) -> String {
    format!(
        "SELECT e.event_id, CAST(e.event_timestamp AS DATE) AS event_date, e.user_id, \
         e.event_type, u.user_name FROM {events_table} e JOIN {users_table} u ON \
         e.user_id = u.user_id"
    )
}

async fn user_names(backend: &DuckDbBackend) -> Vec<(i64, String)> {
    let batches = backend
        .execute_sql(
            "SELECT user_id, user_name FROM main.daily_events_enriched ORDER BY user_id, \
             event_id",
        )
        .await
        .expect("read maintained table");
    let mut out = Vec::new();
    for batch in &batches {
        use arrow::array::{Array, Int32Array, StringArray};
        let ids = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .expect("user_id is INTEGER");
        let names = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("user_name is VARCHAR");
        for i in 0..batch.num_rows() {
            out.push((ids.value(i) as i64, names.value(i).to_string()));
        }
    }
    out
}

async fn except_all_count(backend: &DuckDbBackend, left: &str, right: &str) -> i64 {
    let sql = format!("SELECT count(*) FROM (({left}) EXCEPT ALL ({right})) AS d");
    let batches = backend.execute_sql(&sql).await.expect("except all query");
    use arrow::array::Int64Array;
    let batch = &batches[0];
    let col = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("COUNT(*) is BIGINT");
    col.value(0)
}

/// One renamed user out of three: the delta-restricted recompute's
/// emitted statements carry the semi-join predicate on `user_id`, touch
/// only that user's 2 fact rows, leave the other 4 rows byte-identical,
/// and the maintained table still matches a from-scratch full-refresh
/// oracle over the source's current state. The count-preservation
/// tripwire also passes (clean data, no dangling `user_id`).
#[tokio::test]
async fn point_lookup_recompute_touches_only_the_renamed_users_rows() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("point_lookup.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    seed(&backend).await;

    let body = enrichment_select("main.sources_raw_events", "main.sources_raw_users");
    backend
        .execute_sql(&format!(
            "CREATE TABLE main.daily_events_enriched AS {body}"
        ))
        .await
        .expect("baseline full refresh");

    // Populate the sidecar against the ORIGINAL (pre-rename) content —
    // the baseline every subsequent diff compares against.
    let (_, sql_body) = model_sql_body();
    let consumer_address = "smelt.models.daily_events_enriched";
    refresh_fingerprint_sidecar(
        &backend,
        "main",
        "smelt.sources.raw.users",
        "main.sources_raw_users",
        &["user_id".to_string()],
        &projection(),
        &all_users_columns(),
        &sql_body,
        consumer_address,
        &empty_write_group(),
    )
    .await
    .expect("populate baseline sidecar");

    // Rename user 1 — the ONLY declared-projection column that changed.
    backend
        .execute_sql("UPDATE main.sources_raw_users SET user_name = 'Alicia' WHERE user_id = 1")
        .await
        .expect("rename user 1");

    let changed_keys = diff_fingerprint_sidecar_changed_keys(
        &backend,
        "main",
        "smelt.sources.raw.users",
        "main.sources_raw_users",
        &["user_id".to_string()],
        &projection(),
        &all_users_columns(),
        &sql_body,
        consumer_address,
    )
    .await
    .expect("diff sidecar");
    assert_eq!(
        changed_keys,
        vec!["1".to_string()],
        "renaming exactly 1 of 3 users must synthesize exactly that user's changed-key set"
    );

    let closure = user_name_cell_closure();
    let restriction = resolve_recompute_restriction(closure.as_ref(), Some(&changed_keys));
    let RecomputeRestriction::Restricted { delta_keys } = restriction else {
        panic!("expected Restricted, got {restriction:?}");
    };
    let dimension_key = ["user_id".to_string()];
    let restrict_column = enrichment_restrict_column(&dimension_key).expect("single-column key");

    let region = Region {
        start: "'2025-01-10'".to_string(),
        end: "'2025-01-11'".to_string(),
    };
    let group = emit_delete_insert_delta_restricted(
        "main.daily_events_enriched",
        "event_date",
        &region,
        &body,
        restrict_column,
        &delta_keys,
        MaintenanceDialect::DuckDb,
    );
    assert!(
        group.statements[0].sql.contains("user_id IN ('1')"),
        "DELETE must carry the semi-join predicate: {}",
        group.statements[0].sql
    );
    assert!(
        group.statements[1].sql.contains("user_id IN ('1')"),
        "INSERT must carry the semi-join predicate: {}",
        group.statements[1].sql
    );

    backend
        .execute_statement_group(&group)
        .await
        .expect("execute delta-restricted recompute");

    let names = user_names(&backend).await;
    assert_eq!(
        names,
        vec![
            (1, "Alicia".to_string()),
            (1, "Alicia".to_string()),
            (2, "Bob".to_string()),
            (2, "Bob".to_string()),
            (3, "Carol".to_string()),
            (3, "Carol".to_string()),
        ],
        "only user 1's 2 rows change; users 2 and 3's rows are untouched"
    );

    // End state equals a from-scratch full refresh of the CURRENT
    // source state — the row-count-preserving semi-join restriction
    // did not silently under- or over-write.
    let oracle = enrichment_select("main.sources_raw_events", "main.sources_raw_users");
    let maintained = "SELECT * FROM main.daily_events_enriched".to_string();
    let left_only = except_all_count(&backend, &maintained, &oracle).await;
    let right_only = except_all_count(&backend, &oracle, &maintained).await;
    assert_eq!(
        (left_only, right_only),
        (0, 0),
        "the delta-restricted recompute must match a full-refresh oracle exactly"
    );

    // The RI count-preservation tripwire: clean data (every fact
    // user_id has a matching dimension row) — no violation.
    let driving_select = "SELECT event_id FROM main.sources_raw_events WHERE CAST(\
         event_timestamp AS DATE) >= '2025-01-10' AND CAST(event_timestamp AS DATE) < \
         '2025-01-11'"
        .to_string();
    let enriched_select = format!(
        "{} WHERE CAST(e.event_timestamp AS DATE) >= '2025-01-10' AND CAST(\
         e.event_timestamp AS DATE) < '2025-01-11'",
        enrichment_select("main.sources_raw_events", "main.sources_raw_users")
    );
    let probe = emit_count_preservation_probe(&driving_select, &enriched_select);
    let (driving_count, enriched_count) = run_count_preservation_probe(&backend, &probe).await;
    assert_eq!(
        driving_count, enriched_count,
        "clean data: the count-preservation tripwire must not fire"
    );
}

/// Execute an [`emit_count_preservation_probe`] statement and read back
/// its `(driving_count, enriched_count)` pair.
async fn run_count_preservation_probe(
    backend: &DuckDbBackend,
    probe: &smelt_logical::maintenance::emit::MaintenanceStatement,
) -> (i64, i64) {
    let batches = backend
        .execute_sql(&probe.sql)
        .await
        .expect("execute count-preservation probe");
    use arrow::array::Int64Array;
    let batch = &batches[0];
    let driving = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("driving_count is BIGINT")
        .value(0);
    let enriched = batch
        .column(1)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("enriched_count is BIGINT")
        .value(0);
    (driving, enriched)
}

/// The count-preservation tripwire's negative leg: a dangling fact key
/// (an event whose `user_id` has no matching dimension row) makes the
/// inner-join enrichment's row count fall short of the driving side's —
/// the declared `referential_integrity` is disproven, and the check
/// (mirroring the not-yet-wired `SourceCountPreservationViolated`
/// runtime failure — see this module's own doc comment) fails loudly
/// rather than silently trusting a violated declaration.
#[tokio::test]
async fn violated_referential_integrity_fails_the_tripwire_loudly() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("violated_ri.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    backend
        .execute_sql(
            "CREATE TABLE main.sources_raw_events (event_id INTEGER, user_id INTEGER, \
             event_type VARCHAR, event_timestamp TIMESTAMP)",
        )
        .await
        .expect("create events source table");
    backend
        .execute_sql(
            "INSERT INTO main.sources_raw_events VALUES \
             (1, 1, 'login', TIMESTAMP '2025-01-10 08:00:00'), \
             (2, 99, 'login', TIMESTAMP '2025-01-10 09:00:00')",
        )
        .await
        .expect("seed events (event 2 has a dangling user_id 99)");
    backend
        .execute_sql(
            "CREATE TABLE main.sources_raw_users (user_id INTEGER, user_name VARCHAR, \
             signup_date DATE)",
        )
        .await
        .expect("create users source table");
    backend
        .execute_sql("INSERT INTO main.sources_raw_users VALUES (1, 'Alice', DATE '2025-01-01')")
        .await
        .expect("seed users (no row for user 99 — the declared referential_integrity is false)");

    let driving_select =
        "SELECT event_id FROM main.sources_raw_events WHERE CAST(event_timestamp AS DATE) \
         >= '2025-01-10' AND CAST(event_timestamp AS DATE) < '2025-01-11'"
            .to_string();
    let enriched_select = format!(
        "{} WHERE CAST(e.event_timestamp AS DATE) >= '2025-01-10' AND CAST(\
         e.event_timestamp AS DATE) < '2025-01-11'",
        enrichment_select("main.sources_raw_events", "main.sources_raw_users")
    );
    let probe = emit_count_preservation_probe(&driving_select, &enriched_select);
    let (driving_count, enriched_count) = run_count_preservation_probe(&backend, &probe).await;

    assert_eq!(driving_count, 2, "both events are the driving side");
    assert_eq!(
        enriched_count, 1,
        "the inner join drops event 2 — its user_id 99 has no dimension row"
    );

    let result = check_count_preservation(driving_count, enriched_count, "raw.users");
    assert!(
        result.is_err(),
        "a violated referential_integrity must fail the tripwire, not pass it silently"
    );
    assert!(result
        .unwrap_err()
        .contains("SourceCountPreservationViolated"));
}

/// Mirrors the shape `execute_delete_insert_with_delta_restriction`'s
/// real dispatch now checks (`docs/outcomes/20260809-probe-backed-facts/
/// phases/03-plan.md`) — kept here as a direct unit check of
/// [`emit_count_preservation_probe`]'s own result shape, independent of
/// the full async runtime path exercised by the tests below.
fn check_count_preservation(
    driving_count: i64,
    enriched_count: i64,
    source: &str,
) -> Result<(), String> {
    if enriched_count < driving_count {
        Err(format!(
            "SourceCountPreservationViolated: '{source}' declares referential_integrity, but \
             an enrichment join over the touched region returned {enriched_count} row(s) \
             against {driving_count} driving row(s) — some driving row's join key has no \
             match in the dimension"
        ))
    } else {
        Ok(())
    }
}

/// A `Closed { DeclaredReferentialIntegrity }` restriction over a
/// dangling fact key fails the run loudly (`SourceCountPreservationViolated`,
/// naming the source and the counts), and the target table is
/// byte-unchanged — the probe runs before any write
/// (`docs/outcomes/20260809-probe-backed-facts/phases/03-plan.md` test
/// 5).
#[tokio::test]
async fn declared_ri_restriction_over_a_dangling_key_fails_before_any_write() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("dangling_ri.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    seed(&backend).await;
    // Introduce a dangling fact key: event 7 references user 99, which
    // has no row in main.sources_raw_users.
    backend
        .execute_sql(
            "INSERT INTO main.sources_raw_events VALUES \
             (7, 99, 'login', TIMESTAMP '2025-01-10 14:00:00')",
        )
        .await
        .expect("seed dangling event");

    let body = enrichment_select("main.sources_raw_events", "main.sources_raw_users");
    backend
        .execute_sql(&format!(
            "CREATE TABLE main.daily_events_enriched AS {body}"
        ))
        .await
        .expect("baseline full refresh");
    let baseline_rows = user_names(&backend).await;

    record_observed_delta(
        &backend,
        "main",
        "raw.events",
        "2025-01-10",
        "2025-01-11",
        &["1", "2", "3", "99"],
    )
    .await;

    let closure = SkeletonSourceClosure::Closed {
        row_preservation: RowPreservation::DeclaredReferentialIntegrity {
            source: "main.sources_raw_users".to_string(),
        },
    };
    let region = Region {
        start: "'2025-01-10'".to_string(),
        end: "'2025-01-11'".to_string(),
    };
    let result = execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "daily_events_enriched",
        "event_date",
        &region,
        &body,
        &body,
        Some("user_id"),
        Some(&closure),
        RestrictionDeltaSource::ModelEdge {
            upstream_model: "raw.events",
            window_start: "2025-01-10",
            window_end: "2025-01-11",
        },
        None,
        MaintenanceDialect::DuckDb,
        &super::no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
        &[],
        &[],
    )
    .await;

    let err = result.expect_err("a dangling fact key must fail the tripwire");
    let message = err.to_string();
    assert!(
        message.contains("SourceCountPreservationViolated"),
        "expected the named tripwire diagnostic, got: {message}"
    );
    assert!(
        message.contains("main.sources_raw_users"),
        "expected the declared source named in the error, got: {message}"
    );
    assert!(
        message.contains("or drop the declaration"),
        "expected the remedy in the error, got: {message}"
    );

    // The probe runs before any write — the target table is
    // byte-unchanged from the baseline full refresh.
    let after_rows = user_names(&backend).await;
    assert_eq!(
        baseline_rows, after_rows,
        "a failed tripwire must leave the target table byte-unchanged"
    );
}

/// The same call over conforming data (no dangling key) succeeds and
/// returns the delta-restricted `StatementGroup` unchanged — the probe
/// does not perturb the emitted statements
/// (`docs/outcomes/20260809-probe-backed-facts/phases/03-plan.md` test
/// 6).
#[tokio::test]
async fn declared_ri_restriction_over_conforming_data_succeeds_unperturbed() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("conforming_ri.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    seed(&backend).await;

    let body = enrichment_select("main.sources_raw_events", "main.sources_raw_users");
    backend
        .execute_sql(&format!(
            "CREATE TABLE main.daily_events_enriched AS {body}"
        ))
        .await
        .expect("baseline full refresh");

    record_observed_delta(
        &backend,
        "main",
        "raw.events",
        "2025-01-10",
        "2025-01-11",
        &["1"],
    )
    .await;

    let closure = SkeletonSourceClosure::Closed {
        row_preservation: RowPreservation::DeclaredReferentialIntegrity {
            source: "main.sources_raw_users".to_string(),
        },
    };
    let region = Region {
        start: "'2025-01-10'".to_string(),
        end: "'2025-01-11'".to_string(),
    };
    let group = execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "daily_events_enriched",
        "event_date",
        &region,
        &body,
        &body,
        Some("user_id"),
        Some(&closure),
        RestrictionDeltaSource::ModelEdge {
            upstream_model: "raw.events",
            window_start: "2025-01-10",
            window_end: "2025-01-11",
        },
        None,
        MaintenanceDialect::DuckDb,
        &super::no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
        &[],
        &[],
    )
    .await
    .expect("conforming data must not fail the tripwire");

    assert!(
        group.statements[0].sql.contains("user_id IN ('1')"),
        "the restricted DELETE must still carry the semi-join predicate: {}",
        group.statements[0].sql
    );
    assert!(
        group.statements[1].sql.contains("user_id IN ('1')"),
        "the restricted INSERT must still carry the semi-join predicate: {}",
        group.statements[1].sql
    );
}

/// A declared-RI `Closed` verdict whose body the probe builder cannot
/// reconstruct (no join against the declared source in this body) falls
/// back to the ordinary widened-scan group — the narrowing is dropped,
/// never silently attempted with no verification
/// (`docs/outcomes/20260809-probe-backed-facts/phases/03-plan.md` test
/// 7).
#[tokio::test]
async fn unbuildable_probe_falls_back_to_the_widened_scan() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("unbuildable_probe.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    seed(&backend).await;

    // No join against `main.sources_raw_users` in this body at all —
    // the probe builder cannot locate the declared enrichment join.
    let body =
        "SELECT event_id, CAST(event_timestamp AS DATE) AS event_date, user_id, event_type, \
         'unknown' AS user_name FROM main.sources_raw_events"
            .to_string();
    backend
        .execute_sql(&format!(
            "CREATE TABLE main.daily_events_enriched AS {body}"
        ))
        .await
        .expect("baseline full refresh");

    record_observed_delta(
        &backend,
        "main",
        "raw.events",
        "2025-01-10",
        "2025-01-11",
        &["1"],
    )
    .await;

    let closure = SkeletonSourceClosure::Closed {
        row_preservation: RowPreservation::DeclaredReferentialIntegrity {
            source: "main.sources_raw_users".to_string(),
        },
    };
    let region = Region {
        start: "'2025-01-10'".to_string(),
        end: "'2025-01-11'".to_string(),
    };
    let group = execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "daily_events_enriched",
        "event_date",
        &region,
        &body,
        &body,
        Some("user_id"),
        Some(&closure),
        RestrictionDeltaSource::ModelEdge {
            upstream_model: "raw.events",
            window_start: "2025-01-10",
            window_end: "2025-01-11",
        },
        None,
        MaintenanceDialect::DuckDb,
        &super::no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
        &[],
        &[],
    )
    .await
    .expect("an unbuildable probe must fall back, never fail the run");

    assert!(
        !group.statements[0].sql.contains("user_id IN"),
        "an unbuildable probe must drop the restriction — the widened-scan DELETE carries \
         no semi-join predicate: {}",
        group.statements[0].sql
    );
}
