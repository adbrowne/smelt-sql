use super::*;

/// A licensed restriction (`P1` Closed ∧ a non-empty recorded delta) must
/// execute exactly `emit_delete_insert_delta_restricted`'s own output, byte
/// for byte.
#[tokio::test]
async fn delta_restricted_recompute_statements_come_from_the_emitter() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    let backend = RecordingBackend::new(inner);

    backend
        .execute_sql("CREATE TABLE main.enriched (event_id VARCHAR, event_date DATE, tier VARCHAR)")
        .await
        .expect("create target table");
    backend
        .execute_sql(
            "INSERT INTO main.enriched VALUES ('ev-1', '2026-07-01', 'OLD'), \
             ('ev-2', '2026-07-01', 'OLD')",
        )
        .await
        .expect("seed target table");
    backend
        .execute_sql(
            "CREATE TABLE main.enrichment_recompute (event_id VARCHAR, event_date DATE, tier VARCHAR)",
        )
        .await
        .expect("create recompute source");
    backend
        .execute_sql(
            "INSERT INTO main.enrichment_recompute VALUES ('ev-1', '2026-07-01', 'NEW'), \
             ('ev-2', '2026-07-01', 'NEW')",
        )
        .await
        .expect("seed recompute source");

    let ensure_sql = smelt_state::ddl_duckdb::generate_observed_delta_table_ddl("main");
    backend
        .execute_sql(&ensure_sql)
        .await
        .expect("ensure observed-delta table");
    let upsert_sql = smelt_state::ddl_duckdb::generate_observed_delta_upsert_sql(
        "main",
        "silver.fact",
        "2026-07-01",
        "2026-07-02",
        "SELECT * FROM (VALUES ('ev-1', NULL)) AS t(delta_key, delta_partition)",
    );
    backend
        .execute_sql(&upsert_sql)
        .await
        .expect("record the upstream observed delta");

    let region = smelt_logical::maintenance::emit::Region {
        start: "'2026-07-01'".to_string(),
        end: "'2026-07-02'".to_string(),
    };
    let body = "SELECT event_id, event_date, tier FROM main.enrichment_recompute";
    let closure = smelt_logical::maintenance::SkeletonSourceClosure::Closed {
        row_preservation: smelt_logical::maintenance::RowPreservation::JoinShape,
    };

    smelt_runtime::maintenance_driver::execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "enriched",
        "event_date",
        &region,
        body,
        body,
        Some("event_id"),
        Some(&closure),
        RestrictionDeltaSource::ModelEdge {
            upstream_model: "silver.fact",
            window_start: "2026-07-01",
            window_end: "2026-07-02",
        },
        None,
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
        &[],
        &[],
    )
    .await
    .expect("delta-restricted recompute must succeed");

    let groups = backend.recorded_groups();
    let delete_insert_groups: Vec<_> = groups
        .iter()
        .filter(|g| g.statements[0].sql.starts_with("DELETE FROM main.enriched"))
        .collect();
    assert_eq!(
        delete_insert_groups.len(),
        1,
        "exactly one delta-restricted DELETE+INSERT group: {groups:?}"
    );
    let group = delete_insert_groups[0];

    let expected = smelt_logical::maintenance::emit::emit_delete_insert_delta_restricted(
        "main.enriched",
        "event_date",
        &region,
        body,
        "event_id",
        &["ev-1".to_string()],
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        group.statements, expected.statements,
        "the executed delta-restricted group must be byte-identical to a direct emitter call \
         over the same inputs"
    );
    assert_eq!(group.transactional, expected.transactional);
}

/// State residency (`docs/outcomes/20260904-state-residency/outcome.md`
/// criterion 1): the delta-restricted branch of `execute_delete_insert_
/// with_delta_restriction` — phase 2 left this path recording no
/// reconciliation-ledger reset at all — must, when handed non-empty
/// `ensure_sqls`/`pre_write_sqls`, route the write through `Backend::
/// execute_write_with_bookkeeping` and record the SAME reset pair a caller
/// (`execute.rs`) builds via `generate_ledger_recompute_reset_sqls`, byte
/// for byte, alongside its own delta-restricted DELETE+INSERT.
#[tokio::test]
async fn delta_restricted_recompute_records_the_ledger_reset() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    let backend = RecordingBackend::new(inner);

    backend
        .execute_sql("CREATE TABLE main.enriched (event_id VARCHAR, event_date DATE, tier VARCHAR)")
        .await
        .expect("create target table");
    backend
        .execute_sql(
            "INSERT INTO main.enriched VALUES ('ev-1', '2026-07-01', 'OLD'), \
             ('ev-2', '2026-07-01', 'OLD')",
        )
        .await
        .expect("seed target table");
    backend
        .execute_sql(
            "CREATE TABLE main.enrichment_recompute (event_id VARCHAR, event_date DATE, tier VARCHAR)",
        )
        .await
        .expect("create recompute source");
    backend
        .execute_sql(
            "INSERT INTO main.enrichment_recompute VALUES ('ev-1', '2026-07-01', 'NEW'), \
             ('ev-2', '2026-07-01', 'NEW')",
        )
        .await
        .expect("seed recompute source");

    let ensure_sql = smelt_state::ddl_duckdb::generate_observed_delta_table_ddl("main");
    backend
        .execute_sql(&ensure_sql)
        .await
        .expect("ensure observed-delta table");
    let upsert_sql = smelt_state::ddl_duckdb::generate_observed_delta_upsert_sql(
        "main",
        "silver.fact",
        "2026-07-01",
        "2026-07-02",
        "SELECT * FROM (VALUES ('ev-1', NULL)) AS t(delta_key, delta_partition)",
    );
    backend
        .execute_sql(&upsert_sql)
        .await
        .expect("record the upstream observed delta");

    let region = smelt_logical::maintenance::emit::Region {
        start: "'2026-07-01'".to_string(),
        end: "'2026-07-02'".to_string(),
    };
    let body = "SELECT event_id, event_date, tier FROM main.enrichment_recompute";
    let closure = smelt_logical::maintenance::SkeletonSourceClosure::Closed {
        row_preservation: smelt_logical::maintenance::RowPreservation::JoinShape,
    };

    let ledger_ensure_sqls = vec![smelt_state::ddl_duckdb::generate_ledger_table_ddl("main")];
    let ledger_pre_write_sqls = smelt_state::ddl_duckdb::generate_ledger_recompute_reset_sqls(
        "main",
        "silver.enriched",
        "{*}",
        "2026-07-01",
        "2026-07-02",
        "self",
        "2026-07-02",
    );

    smelt_runtime::maintenance_driver::execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "enriched",
        "event_date",
        &region,
        body,
        body,
        Some("event_id"),
        Some(&closure),
        RestrictionDeltaSource::ModelEdge {
            upstream_model: "silver.fact",
            window_start: "2026-07-01",
            window_end: "2026-07-02",
        },
        None,
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
        &ledger_ensure_sqls,
        &ledger_pre_write_sqls,
    )
    .await
    .expect("delta-restricted recompute with ledger bookkeeping must succeed");

    let sql_log = backend.recorded_sql();
    assert!(
        sql_log.contains(&ledger_ensure_sqls[0]),
        "the ledger ensure DDL must be sent as raw SQL: {sql_log:?}"
    );
    for stmt in &ledger_pre_write_sqls {
        assert!(
            sql_log.contains(stmt),
            "the delta-restricted branch must record the SAME ledger reset a plain DeleteInsert \
             write would (byte-identical to `generate_ledger_recompute_reset_sqls`): {stmt}\n\
             recorded: {sql_log:?}"
        );
    }

    let groups = backend.recorded_groups();
    let delete_insert_groups: Vec<_> = groups
        .iter()
        .filter(|g| g.statements[0].sql.starts_with("DELETE FROM main.enriched"))
        .collect();
    assert_eq!(
        delete_insert_groups.len(),
        1,
        "exactly one delta-restricted DELETE+INSERT group: {groups:?}"
    );
    let group = delete_insert_groups[0];
    for stmt in &group.statements {
        assert!(
            !stmt.sql.contains("_smelt_ledger"),
            "ledger bookkeeping must never appear inside the maintenance StatementGroup: {}",
            stmt.sql
        );
    }

    let expected = smelt_logical::maintenance::emit::emit_delete_insert_delta_restricted(
        "main.enriched",
        "event_date",
        &region,
        body,
        "event_id",
        &["ev-1".to_string()],
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        group.statements, expected.statements,
        "the write group itself must still be byte-identical to the emitter's own output — \
         bookkeeping must not alter what gets written"
    );
}

/// The region family's own change-suppressed conditional variant
/// (`RegionWrite::Suppressed`, `docs/outcomes/20260815-definition-delta-
/// migrate/phases/27b-plan.md`) executes exactly `emit_diff_patch`'s own
/// output — no delta restriction admitted (`restrict_column: None`), so the
/// dispatch falls through past the T3 arm straight to the region-write
/// dimension.
#[tokio::test]
async fn region_conditional_write_matches_the_emitted_group_byte_for_byte() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    let backend = RecordingBackend::new(inner);

    backend
        .execute_sql(
            "CREATE TABLE main.regions (region_id VARCHAR, region_date DATE, amount INTEGER)",
        )
        .await
        .expect("create target table");
    backend
        .execute_sql(
            "INSERT INTO main.regions VALUES ('r1', '2026-07-01', 10), ('r2', '2026-07-01', 20)",
        )
        .await
        .expect("seed target table");

    let region = smelt_logical::maintenance::emit::Region {
        start: "'2026-07-01'".to_string(),
        end: "'2026-07-02'".to_string(),
    };
    let body = "SELECT region_id, region_date, amount FROM (VALUES \
                ('r1', DATE '2026-07-01', 10), ('r2', DATE '2026-07-01', 25)) \
                AS t(region_id, region_date, amount)";
    let region_write = smelt_logical::maintenance::choice::RegionWrite::Suppressed {
        key: vec!["region_id".to_string()],
        compared_columns: vec!["amount".to_string()],
    };

    smelt_runtime::maintenance_driver::execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "regions",
        "region_date",
        &region,
        body,
        body,
        None,
        None,
        RestrictionDeltaSource::ModelEdge {
            upstream_model: "sources.regions_raw",
            window_start: "2026-07-01",
            window_end: "2026-07-02",
        },
        Some(&region_write),
        MaintenanceDialect::DuckDb,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
        &[],
        &[],
    )
    .await
    .expect("suppressed region recompute must succeed");

    let groups = backend.recorded_groups();
    let diff_patch_groups: Vec<_> = groups
        .iter()
        .filter(|g| {
            g.statements[0]
                .sql
                .starts_with("CREATE TEMP TABLE __smelt_diff_patch_main_regions")
        })
        .collect();
    assert_eq!(
        diff_patch_groups.len(),
        1,
        "exactly one staged diff_patch group: {groups:?}"
    );
    let group = diff_patch_groups[0];

    let slice_predicate = region.predicate(Some("main.regions"), "region_date");
    let expected = emit_diff_patch(
        "main.regions",
        "__smelt_diff_patch_main_regions",
        &["region_id".to_string()],
        body,
        &["amount".to_string()],
        &slice_predicate,
        &smelt_logical::maintenance::diff_patch::DeleteLeg::Complete,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        group.statements, expected.statements,
        "the executed region conditional group must be byte-identical to a direct emitter call \
         over the same inputs"
    );
    assert_eq!(group.transactional, expected.transactional);
}

/// An `Open` closure (or an absent/empty delta — asserted below) must
/// execute exactly `emit_delete_insert`'s own unrestricted output, never a
/// partially-restricted variant.
#[tokio::test]
async fn open_closure_recompute_statements_come_from_the_unrestricted_emitter() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    let backend = RecordingBackend::new(inner);

    backend
        .execute_sql("CREATE TABLE main.enriched (event_id VARCHAR, event_date DATE, tier VARCHAR)")
        .await
        .expect("create target table");
    backend
        .execute_sql(
            "CREATE TABLE main.enrichment_recompute (event_id VARCHAR, event_date DATE, tier VARCHAR)",
        )
        .await
        .expect("create recompute source");

    let region = smelt_logical::maintenance::emit::Region {
        start: "'2026-07-01'".to_string(),
        end: "'2026-07-02'".to_string(),
    };
    let body = "SELECT event_id, event_date, tier FROM main.enrichment_recompute";
    let closure = smelt_logical::maintenance::SkeletonSourceClosure::Open {
        reason: "test".to_string(),
    };

    smelt_runtime::maintenance_driver::execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "enriched",
        "event_date",
        &region,
        body,
        body,
        Some("event_id"),
        Some(&closure),
        RestrictionDeltaSource::ModelEdge {
            upstream_model: "silver.fact",
            window_start: "2026-07-01",
            window_end: "2026-07-02",
        },
        None,
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
        &[],
        &[],
    )
    .await
    .expect("unrestricted recompute must succeed");

    let groups = backend.recorded_groups();
    let delete_insert_groups: Vec<_> = groups
        .iter()
        .filter(|g| g.statements[0].sql.starts_with("DELETE FROM main.enriched"))
        .collect();
    assert_eq!(delete_insert_groups.len(), 1, "{groups:?}");
    let group = delete_insert_groups[0];

    let expected = smelt_logical::maintenance::emit::emit_delete_insert(
        "main.enriched",
        "event_date",
        &region,
        body,
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        group.statements, expected.statements,
        "an Open closure must execute the byte-identical unrestricted emitter output"
    );
}
