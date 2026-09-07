use super::*;

/// The default `retain_departed` point's runtime half (`docs/outcomes/
/// 20260815-definition-delta-migrate/phases/32b-plan.md`): a snapshot-
/// reconcile keyed run's executed statements are exactly `emit_keyed_fold`
/// + `emit_departed_key_delete`, sent as one `transactional: true`
/// `StatementGroup` — and the post-run table is multiset-equal to a full
/// refresh of the new source (the departed key is gone from both).
#[tokio::test]
async fn snapshot_reconcile_delete_leg_parity() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();

    std::fs::write(
        project_dir.join("models/sources/devices.yml"),
        "description: Raw per-device rows, no clock.\n\
         columns:\n\
         \x20\x20- name: device_id\n\
         \x20\x20\x20\x20type: INTEGER\n\
         \x20\x20- name: amount\n\
         \x20\x20\x20\x20type: DOUBLE\n\
         mutation_profile:\n\
         \x20\x20kind: mutable_snapshot\n",
    )
    .unwrap();

    write_model(
        project_dir,
        "device_snapshot",
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         maintenance:\n\
         \x20\x20scan_bounds:\n\
         \x20\x20\x20\x20per_source:\n\
         \x20\x20\x20\x20\x20\x20devices:\n\
         \x20\x20\x20\x20\x20\x20\x20\x20allow_full_scan: true\n\
         ---\n\
         SELECT device_id, ANY_VALUE(amount) AS amount FROM smelt.sources.devices GROUP BY 1",
    );

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: statement_parity_departed_key_test\nversion: 1\npaths:\n  - models\ntargets:\n  \
         dev:\n    type: duckdb\n    database: {db}\n    schema: main\n\
         default_materialization: table\ntarget: dev\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    let config = Arc::new(Config::load(project_dir).expect("load config"));

    let mut request = make_request("dev", "2024-01-01", "2024-01-01");
    request.start = None;
    request.end = None;

    // Run 1: table does not exist — the create path, no delete leg to prove.
    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        let conn = duckdb::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE SCHEMA IF NOT EXISTS main; \
             CREATE OR REPLACE TABLE main.sources_devices AS \
             SELECT * FROM (VALUES (1, 10.0), (2, 5.0)) AS t(device_id, amount);",
        )
        .unwrap();
        drop(conn);

        let backend_slot = Arc::new(Mutex::new(None));
        let factory = RecordingBackendFactory {
            db_path: db_path.clone(),
            backend: Arc::clone(&backend_slot),
        };
        execute_project(
            "snapshot-reconcile-parity-run-1".to_string(),
            request.clone(),
            Arc::clone(&config),
            Arc::clone(&graph),
            Arc::clone(&db),
            project_dir,
            &factory,
            &NO_OP_REPORTER,
            CancellationToken::new(),
        )
        .await
        .expect("first run creates the table");
    }

    // Device 2 departs the source.
    {
        let conn = duckdb::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE OR REPLACE TABLE main.sources_devices AS \
             SELECT * FROM (VALUES (1, 10.0)) AS t(device_id, amount);",
        )
        .unwrap();
    }

    let (db, graph) = build_db_and_graph(project_dir, &config);
    let backend_slot = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    execute_project(
        "snapshot-reconcile-parity-run-2".to_string(),
        request,
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        project_dir,
        &factory,
        &NO_OP_REPORTER,
        CancellationToken::new(),
    )
    .await
    .expect("reconcile run deletes the departed key");

    let backend = backend_slot
        .lock()
        .unwrap()
        .take()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    let reconcile_group = groups
        .iter()
        .find(|g| g.statements.iter().any(|s| s.sql.starts_with("MERGE INTO")))
        .expect("reconcile run must execute via execute_statement_group");

    assert!(
        reconcile_group.transactional,
        "the merge + departed-key delete must execute as one transactional group"
    );

    // Recover the compiler's own delta SELECT (type-cast-wrapped, with its
    // header comment) from the executed merge text — the same "read the
    // embedded relation back" approach `extract_affected_keys_select` above
    // uses for the repair family, since the compiled SQL a real run embeds
    // is not byte-reconstructable from the model's source text alone.
    let merge_sql = &reconcile_group.statements[0].sql;
    let using_marker = "USING (";
    let delta_start = merge_sql.find(using_marker).expect("USING clause") + using_marker.len();
    let delta_end_marker = ") AS delta ON";
    let delta_end = merge_sql.rfind(delta_end_marker).expect("delta alias");
    let delta_select = &merge_sql[delta_start..delta_end];
    let expected_merge = emit_keyed_fold_suppressed(
        "main.device_snapshot",
        &["device_id".to_string()],
        &[("amount".to_string(), "delta.amount".to_string())],
        delta_select,
        None,
        &["amount".to_string()],
        MaintenanceDialect::DuckDb,
    );
    let expected_delete = smelt_logical::contract::retain_departed::reconcile_disposition(None);
    assert_eq!(
        expected_delete,
        smelt_logical::contract::retain_departed::DepartedKeyDisposition::Delete,
        "sanity: undeclared retain_departed resolves to the default delete point"
    );
    let expected_delete_stmt = emit_departed_key_delete(
        "main.device_snapshot",
        &["device_id".to_string()],
        delta_select,
        MaintenanceDialect::DuckDb,
    );

    assert_eq!(
        reconcile_group.statements.len(),
        2,
        "expected exactly the merge and the delete, got: {:#?}",
        reconcile_group.statements
    );
    assert_eq!(
        reconcile_group.statements[0], expected_merge.statements[0],
        "executed merge must be byte-identical to a direct emit_keyed_fold call"
    );
    assert_eq!(
        reconcile_group.statements[1], expected_delete_stmt,
        "executed delete must be byte-identical to a direct emit_departed_key_delete call"
    );

    assert!(
        multiset_equal(
            &*backend,
            "SELECT device_id, amount FROM main.device_snapshot",
            delta_select,
        )
        .await,
        "the post-run table must be multiset-equal to a full refresh of the new source"
    );
}

// =============================================================================
// Structural gate: no maintenance-statement authoring outside the emitter
// (`docs/specs/incremental_models.md` §"Statement emission (single owner)";
// `docs/plans/20260710-emit-unification.md` Phase 4). Same `rg`-over-sources
// style as `crates/smelt-core/tests/hardening_budget.rs`: a source scan, not
// a runtime assertion, so it catches a regression at review time rather than
// only when a fixture happens to exercise the reintroduced text.
// =============================================================================

/// Repo root, two levels up from this crate's manifest dir
/// (`crates/smelt-runtime` → `crates` → repo root) — the same derivation
/// `crates/smelt-core/tests/hardening_budget.rs::repo_root` uses.
pub(super) fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root")
        .to_path_buf()
}

/// One forbidden-shape hit: `(file, 1-based line number, line text)`.
struct StatementAuthoringHit {
    file: std::path::PathBuf,
    line_no: usize,
    text: String,
}

/// Known, pre-existing, out-of-scope matches this gate does not fail on —
/// `(file path suffix, distinguishing substring of the offending line)`.
/// Removing an entry without fixing (or re-justifying) the underlying
/// authoring is itself the review signal this gate exists to raise.
///
/// Every entry here belongs to `Backend::delete_partitions`/
/// `Backend::insert_overwrite` — DELETE/INSERT-OVERWRITE SQL that predates
/// `incremental_models.md`'s single-owner emitters entirely. `IncrementalStrategy`
/// has one dispatchable variant, `DeleteInsert`; `smelt_runtime::
/// maintenance_driver::resolve_incremental_strategy` and the batch loop's
/// own dispatch (`crates/smelt-runtime/src/execute.rs`) only ever resolve it.
/// `insert_into_from_query`/`insert_overwrite` remain on the `Backend` trait
/// as the capability that would admit an append-only or overwrite strategy
/// once plan derivation selects one; no plan derivation calls them today.
/// Routing this hand-authored SQL through `emit_delete_insert` too, closing
/// the remaining gap, is out of Phase 4's file scope (`docs/plans/
/// 20260710-emit-unification.md` Phase 4 "Critical files" — the backend
/// crates are not listed); tracked as follow-up, not fixed here.
const STATEMENT_AUTHORING_ALLOWLIST: &[(&str, &str)] = &[
    (
        "smelt-backend-duckdb/src/lib.rs",
        "DELETE FROM {} WHERE {} >= {} AND {} < {}",
    ),
    (
        "smelt-backend-duckdb/src/lib.rs",
        "DELETE FROM {} WHERE {} IN (SELECT DISTINCT {} FROM ({}))",
    ),
    (
        "smelt-backend-spark/src/sql.rs",
        "DELETE FROM {} WHERE {} IN ({})",
    ),
    (
        "smelt-backend-spark/src/sql.rs",
        "DELETE FROM {} WHERE {} >= {} AND {} < {}",
    ),
];

fn statement_authoring_is_allowlisted(path: &Path, line: &str) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    STATEMENT_AUTHORING_ALLOWLIST
        .iter()
        .any(|(file_suffix, substr)| normalized.ends_with(file_suffix) && line.contains(substr))
}

/// Scan one production `.rs` file for forbidden maintenance-statement
/// shapes. Stops at the first `#[cfg(test)]` line (test fixtures — e.g.
/// `maintenance_driver.rs`'s in-memory `SumRule`/`RecordingBackend` — build
/// deliberately statement-shaped strings to exercise dispatch without a
/// real backend; that is not production authoring) — the same truncation
/// `hardening_budget.rs::count_println_in_file` uses. Skips comment lines
/// (`//`, `///`, `//!`) since the forbidden shapes appear in doc comments
/// describing the emitter's own output.
fn scan_statement_authoring_file(path: &Path, hits: &mut Vec<StatementAuthoringHit>) {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("#[cfg(test)]") {
            break;
        }
        if trimmed.starts_with("//") {
            continue;
        }
        let forbidden = line.contains("DELETE FROM ")
            || line.contains("MERGE INTO ")
            || line.contains("CREATE TABLE {}.{} AS")
            // The staged-candidate conditional DELETE+INSERT's temp
            // relation (T2, `docs/plans/20260715-composed-axes-conditional-
            // maintenance.md` Phase C5) — a distinctive shape with no
            // pre-existing production match, unlike a bare `DROP TABLE `
            // (which the generic table-lifecycle helpers already construct
            // legitimately, outside any maintenance-statement family — see
            // `Backend::drop_table_if_exists`'s own implementations).
            || line.contains("CREATE TEMP TABLE ")
            // The backbuild family (`crates/smelt-logical/src/backbuild/
            // emit.rs`): `ALTER TABLE ` covers B1/B2/B3's `ADD`/`RENAME` and
            // C1's `DROP` (no other production code in the scanned crates
            // issues a bare `ALTER TABLE ` DDL string); `CREATE OR REPLACE
            // TABLE ` is the always-present model-level `FullRefresh`
            // baseline, distinct from the region family's own qualified
            // `CREATE TABLE {}.{} AS` shape above; `__backbuild_diff` is the
            // derived-table alias `emit_difference_insert` (E2/E4) wraps its
            // own `after_sql` argument in — a marker string with no
            // legitimate production match anywhere outside that one
            // authoring site, representative of the in-place-UPDATE/
            // difference-INSERT half of the family the way `CREATE TEMP
            // TABLE ` is representative of the staged-candidate shape above
            // (not every backbuild statement shape has an equally unique
            // marker; this one does, and catching a stray copy of it is
            // enough to catch a re-authored difference/branch INSERT).
            || line.contains("ALTER TABLE ")
            || line.contains("CREATE OR REPLACE TABLE ")
            || line.contains("__backbuild_diff")
            // The succession-patch family (`docs/outcomes/
            // 20260906-scd2-keyed-succession/phases/04-plan.md`): `MERGE
            // INTO `/`DELETE FROM ` above already catch the presented
            // `MERGE` and any hand-authored delete; `__tombstones` is the
            // reserved tombstone-ledger table-name suffix
            // (`smelt_logical::maintenance::emit::tombstone_table_name`),
            // a distinctive marker for the ledger insert and the
            // ledger-rebuild `SELECT` alike, on the same "unique marker
            // string" footing as `__backbuild_diff` above (a bare `SELECT
            // ...` has no unique shape to scan for).
            || line.contains("__tombstones");
        if !forbidden {
            continue;
        }
        if statement_authoring_is_allowlisted(path, line) {
            continue;
        }
        hits.push(StatementAuthoringHit {
            file: path.to_path_buf(),
            line_no: idx + 1,
            text: line.trim().to_string(),
        });
    }
}

/// `src/`-relative file paths excluded from the scan entirely: the
/// backbuild single-owner emitter module
/// (`docs/specs/architecture.md` §"Constraints & Invariants" item 12 —
/// every maintenance/backbuild statement is the output of a pure emitter in
/// one of these modules; scanning them for the shapes they themselves
/// author would be circular; the maintenance emitter module is excluded by
/// directory, see [`EMITTER_MODULE_DIR_EXCLUSIONS`]), plus `smelt-state`'s three per-dialect
/// schema-evolution DDL modules. Schema-evolution DDL is declared a
/// *separate* single-owner family, outside the maintenance/backbuild
/// emitter rule (`docs/specs/incremental_models.md` §"Statement emission
/// (single owner)"): it is multi-dialect and covers struct/nested/
/// nullability operations the backbuild emitters have no forms for, and
/// `smelt-state` sits below `smelt-logical`, so it cannot call into
/// `backbuild::emit`. `ddl_duckdb.rs` is the actual per-dialect renderer
/// owner; `ddl_spark.rs`/`ddl_bigquery.rs` are excluded on the same
/// per-dialect-owner basis even though their DDL shapes (backtick-quoted
/// identifiers, `ADD COLUMNS (...)`, `SET DATA TYPE`) don't match this
/// scan's DuckDB-flavored `ALTER TABLE `/`UPDATE ` shapes anyway.
const EMITTER_MODULE_EXCLUSIONS: &[&str] = &[
    "smelt-logical/src/backbuild/emit.rs",
    "smelt-state/src/ddl_duckdb.rs",
    "smelt-state/src/ddl_spark.rs",
    "smelt-state/src/ddl_bigquery.rs",
];

/// `src/`-relative directory paths excluded wholesale: the maintenance
/// emitter module, whose emitters are split across per-family submodules
/// under one directory. Excluded for the same circularity reason as the
/// files in [`EMITTER_MODULE_EXCLUSIONS`].
const EMITTER_MODULE_DIR_EXCLUSIONS: &[&str] = &["smelt-logical/src/maintenance/emit/"];

fn is_emitter_module(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    EMITTER_MODULE_EXCLUSIONS
        .iter()
        .any(|suffix| normalized.ends_with(suffix))
        || EMITTER_MODULE_DIR_EXCLUSIONS
            .iter()
            .any(|dir| normalized.contains(dir))
}

fn scan_statement_authoring_dir(dir: &Path, hits: &mut Vec<StatementAuthoringHit>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // `tests/` subdirectories hold integration tests, not
            // production code — this file (`crates/smelt-runtime/tests/`)
            // is itself outside every scanned `src/` tree.
            if path.file_name().map(|n| n == "tests").unwrap_or(false) {
                continue;
            }
            scan_statement_authoring_dir(&path, hits);
        } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
            // `tests.rs` (e.g. `smelt-backend-spark/src/tests.rs`) is a
            // unit-test module file, not production code.
            if path.file_name().map(|n| n == "tests.rs").unwrap_or(false) {
                continue;
            }
            if is_emitter_module(&path) {
                continue;
            }
            scan_statement_authoring_file(&path, hits);
        }
    }
}

/// Structural gate: `DELETE FROM`/`MERGE INTO`/`CREATE TABLE {}.{} AS`-shaped
/// statement text must not be constructed anywhere in `smelt-backend*/src`,
/// `smelt-runtime/src`, or `smelt-logical/src` production code outside the
/// two single-owner emitter modules
/// (`crates/smelt-logical/src/maintenance/emit/`,
/// `crates/smelt-logical/src/backbuild/emit.rs` —
/// [`EMITTER_MODULE_EXCLUSIONS`], excluded rather than unscanned entirely so
/// a *new* statement-shaped file dropped anywhere else in `smelt-logical`
/// is still caught). `smelt-logical` joined the scan in
/// `docs/plans/20260808-substrate-unification.md` ("emitter unification and
/// gate extension") — the no-authoring rule already applied crate-wide in
/// spec (`docs/specs/architecture.md` §"Constraints & Invariants" item 12:
/// "backends execute, never author"), this widens the structural gate to
/// match. Backends execute emitted `StatementGroup`s
/// (`Backend::execute_statement_group`); they never author
/// maintenance-statement text of their own
/// (`docs/specs/incremental_models.md` §"Statement emission (single owner)").
#[test]
fn no_maintenance_statement_authoring_outside_the_emitter() {
    let crates_dir = repo_root().join("crates");
    let mut hits = Vec::new();
    for crate_name in [
        "smelt-backend",
        "smelt-backend-duckdb",
        "smelt-backend-spark",
        "smelt-backends",
        "smelt-runtime",
        "smelt-logical",
        "smelt-state",
    ] {
        scan_statement_authoring_dir(&crates_dir.join(crate_name).join("src"), &mut hits);
    }
    assert!(
        hits.is_empty(),
        "maintenance-statement text constructed outside smelt-logical's single-owner emitters \
         (docs/specs/incremental_models.md §\"Statement emission (single owner)\") — backends must \
         execute an emitted StatementGroup, never author their own SQL text:\n{}",
        hits.iter()
            .map(|h| format!("  {}:{}: {}", h.file.display(), h.line_no, h.text))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The append-only posture probe's dispatch site
/// (`smelt_runtime::source_probes::dispatch_and_record_append_only_postures`)
/// must execute SQL byte-identical to a direct
/// `emit_append_only_posture_probe`/`emit_append_only_baseline_snapshot`
/// call over the same inputs (`docs/outcomes/20260809-probe-backed-facts/
/// outcome.md` phase 6). This drives `dispatch_and_record_append_only_
/// postures` directly against a [`RecordingBackend`] rather than the full
/// `execute_project` pipeline — the same rationale
/// `recurrence_bound_probe_and_checked_merge_come_from_the_emitters` gives:
/// this driver is the single point every append-only posture probe and
/// baseline-refresh statement flows through, so calling it directly still
/// proves *executed* SQL matches the emitter's output, without needing a
/// full staged workspace to reach this one call site twice (once via a
/// full-refresh model, once via an incremental batch).
#[tokio::test]
async fn append_only_posture_probe_and_baseline_snapshot_come_from_the_emitters() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
        conn.execute_batch(
            "CREATE SCHEMA IF NOT EXISTS raw;\n\
             CREATE TABLE raw.events (event_date DATE, payload TEXT);\n\
             INSERT INTO raw.events VALUES (DATE '2026-01-01', 'a'), (DATE '2026-01-02', 'b');",
        )
        .expect("stage raw.events");
    }
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open backend");
    let backend = RecordingBackend::new(inner);

    let parse = smelt_parser::parse("SELECT * FROM smelt.sources.raw.events");
    let refs = smelt_parser::ast::File::cast(parse.syntax())
        .map(|f| smelt_core::extract_refs(&f))
        .unwrap_or_default();
    let model_path = std::path::PathBuf::from("models/m.sql");
    let model = smelt_core::ModelFile {
        name: "m".to_string(),
        path: model_path.clone(),
        content: "SELECT * FROM smelt.sources.raw.events".to_string(),
        refs,
        parse_errors: Vec::new(),
        metadata: None,
        kind: smelt_core::ModelKind::Sql,
        model_id: smelt_core::ModelId::from_path(model_path),
        address_segments: vec!["m".to_string()],
    };
    let source = smelt_core::sources::SourceInfo {
        path: std::path::PathBuf::from("/tmp/fake.yml"),
        address_segments: vec![
            "sources".to_string(),
            "raw".to_string(),
            "events".to_string(),
        ],
        columns: vec![smelt_core::sources::SourceColumn {
            name: "payload".to_string(),
            data_type: smelt_types::DataType::Text,
            nullable: true,
            description: None,
        }],
        description: None,
        name_override: Some(smelt_core::sources::SourceNameOverride::Literal(
            "raw.events".to_string(),
        )),
        tags: vec![],
        timeseries: Some(smelt_core::config::TimeseriesConfig {
            event_time_column: "event_date_ts".to_string(),
            partition_column: "event_date".to_string(),
            granularity: smelt_core::config::Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        mutation_profile: Some(smelt_core::sources::SourceMutationProfile::from_kind(
            smelt_core::sources::MutationProfile::AppendOnly,
        )),
        source_lateness: None,
        watermark: None,
        unique_key: None,
        retention: None,
        referential_integrity: None,
    };

    let mut baselines = smelt_state::source_postures::SourcePostureStore::default();
    baselines.record(
        "raw.events",
        vec![
            smelt_state::source_postures::SourcePosturePartition {
                partition_value: "2026-01-01".to_string(),
                recorded_count: 1,
                recorded_fingerprint: "not-the-real-fingerprint".to_string(),
            },
            smelt_state::source_postures::SourcePosturePartition {
                partition_value: "2026-01-02".to_string(),
                recorded_count: 1,
                recorded_fingerprint: "irrelevant-for-the-open-partition".to_string(),
            },
        ],
    );

    let probes = smelt_runtime::source_probes::append_only_posture_probes(
        "m",
        "m creation",
        &model,
        &[source],
        &baselines,
        "dev",
        "raw",
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(probes.len(), 1);

    // Direct emitter calls over the exact same inputs the probe builder used.
    let expected_probe_sql = emit_append_only_posture_probe(
        "raw.events",
        "event_date",
        &["payload".to_string()],
        &[
            smelt_logical::maintenance::emit::AppendOnlyBaselinePartition {
                partition_value: "2026-01-01".to_string(),
                recorded_count: 1,
                recorded_fingerprint: "not-the-real-fingerprint".to_string(),
                check_fingerprint: true,
            },
            smelt_logical::maintenance::emit::AppendOnlyBaselinePartition {
                partition_value: "2026-01-02".to_string(),
                recorded_count: 1,
                recorded_fingerprint: "irrelevant-for-the-open-partition".to_string(),
                check_fingerprint: false,
            },
        ],
        MaintenanceDialect::DuckDb,
    )
    .sql;
    let (probe_sql, snapshot_sql) = match &probes[0].action {
        smelt_runtime::source_probes::SourcePostureAction::Verify {
            sql, snapshot_sql, ..
        } => (sql.clone(), snapshot_sql.clone()),
        smelt_runtime::source_probes::SourcePostureAction::Establish { .. } => {
            panic!("a recorded baseline must build a Verify action, not Establish")
        }
    };
    assert_eq!(probe_sql, expected_probe_sql);

    let expected_snapshot_sql =
        smelt_logical::maintenance::emit::emit_append_only_baseline_snapshot(
            "raw.events",
            "event_date",
            &["payload".to_string()],
            MaintenanceDialect::DuckDb,
        )
        .sql;
    assert_eq!(snapshot_sql, expected_snapshot_sql);

    // The probe fires (the recorded fingerprint for the closed partition
    // is deliberately wrong) — dispatch fails loud before any snapshot
    // executes, and the ONLY SQL actually run is the probe statement,
    // byte-identical to the direct emitter call.
    let err = smelt_runtime::source_probes::dispatch_and_record_append_only_postures(
        &backend,
        &smelt_runtime::probes::ProbePolicy::per_run(),
        &probes,
    )
    .await
    .expect_err("the mismatched closed-partition fingerprint must fail loud");
    assert!(err.to_string().contains("SourceMutationProfileViolated"));

    let executed = backend.recorded_sql();
    assert_eq!(
        executed,
        vec![expected_probe_sql.clone()],
        "the dispatch site must execute exactly the emitted probe SQL, nothing more"
    );
}

/// The mutation-happened discrimination gate
/// (`smelt_runtime::mutation_probe::gate_upstream_mutation_dispatch`) must
/// execute SQL byte-identical to a direct `emit_source_mutation_fingerprint`
/// call over the same inputs (`docs/specs/incremental_models.md` §"When a
/// mutation cell dispatches") — the statement-emission single-owner rule
/// (`CLAUDE.md` §"Maintenance-plan purity").
#[tokio::test]
async fn source_mutation_fingerprint_comes_from_the_emitter() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
        conn.execute_batch(
            "CREATE SCHEMA IF NOT EXISTS raw;\n\
             CREATE TABLE raw.dim_users (user_id INTEGER, status TEXT);\n\
             INSERT INTO raw.dim_users VALUES (1, 'active'), (2, 'inactive');",
        )
        .expect("stage raw.dim_users");
    }
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open backend");
    let backend = RecordingBackend::new(inner);

    let digest_columns = vec!["user_id".to_string(), "status".to_string()];
    let expected_sql = emit_source_mutation_fingerprint(
        "raw.dim_users",
        &digest_columns,
        MaintenanceDialect::DuckDb,
    )
    .sql;

    let (verdict, refreshed) = smelt_runtime::mutation_probe::gate_upstream_mutation_dispatch(
        &backend,
        "m",
        "raw.dim_users",
        "raw.dim_users",
        &digest_columns,
        MaintenanceDialect::DuckDb,
        None,
    )
    .await
    .expect("gate must succeed against a live backend");

    assert_eq!(
        verdict,
        smelt_runtime::mutation_probe::MutationVerdict::Dispatch,
        "no recorded baseline must always dispatch"
    );
    assert_eq!(refreshed.recorded_count, 2);
    assert_eq!(refreshed.digest_columns, digest_columns);

    let executed = backend.recorded_sql();
    assert_eq!(
        executed,
        vec![expected_sql],
        "the gate must execute exactly the emitted fingerprint SQL, nothing more"
    );

    // A second gate call against the SAME baseline (nothing changed) must
    // observe the identical fingerprint and report NoOp.
    let (verdict2, _refreshed2) = smelt_runtime::mutation_probe::gate_upstream_mutation_dispatch(
        &backend,
        "m",
        "raw.dim_users",
        "raw.dim_users",
        &digest_columns,
        MaintenanceDialect::DuckDb,
        Some(&refreshed),
    )
    .await
    .expect("gate must succeed against a live backend");
    assert_eq!(
        verdict2,
        smelt_runtime::mutation_probe::MutationVerdict::NoOp
    );
}

// =============================================================================
// State residency (`docs/outcomes/20260904-state-residency/outcome.md`
// criterion 1): the reconciliation ledger's region-recompute reset shares
// ONE backend transaction with the write it protects — proven directly
// against `DuckDbBackend::execute_write_with_bookkeeping` rather than
// through the full `execute_project` pipeline, since provoking a mid-batch
// write failure through the real pipeline has no clean seam.
// =============================================================================

/// A valid ledger reset as `pre_write_sqls`, paired with a deliberately
/// invalid write `StatementGroup`: the call must error, and `_smelt_ledger`
/// must hold no row for the region the failed write never actually wrote —
/// proving "same transaction as the maintained write", not merely "runs
/// alongside it".
#[tokio::test]
async fn ledger_reset_rolls_back_with_a_failed_write() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    let ensure_sqls = vec![smelt_state::ddl_duckdb::generate_ledger_table_ddl("main")];
    let pre_write_sqls = smelt_state::ddl_duckdb::generate_ledger_recompute_reset_sqls(
        "main",
        "rollback_model",
        "{*}",
        "2026-08-01",
        "2026-08-02",
        "self",
        "2026-08-02",
    );
    let write_group = StatementGroup {
        statements: vec![smelt_backend::MaintenanceStatement {
            sql: "INSERT INTO main.does_not_exist VALUES (1)".to_string(),
        }],
        transactional: false,
    };

    let result = backend
        .execute_write_with_bookkeeping(&ensure_sqls, &pre_write_sqls, &write_group)
        .await;
    assert!(
        result.is_err(),
        "the failed write must surface an error, not silently swallow it"
    );

    // The ensure DDL is idempotent DDL run OUTSIDE the transaction (same
    // precedent as `Backend::fold_ledger_delta`'s `ensure_sql`), so the
    // table exists even after the rollback; the query below proves it holds
    // no row, not that it's absent.
    let rows = backend
        .execute_sql("SELECT COUNT(*) FROM main._smelt_ledger WHERE model_name = 'rollback_model'")
        .await
        .expect("query ledger row count");
    let batch = rows.first().expect("COUNT returns one row");
    let count = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("COUNT column is Int64")
        .value(0);
    assert_eq!(
        count, 0,
        "a failed write must leave no reconciliation-ledger reset row behind"
    );
}

/// A non-DuckDB dialect's DeleteInsert batch write emits no `_smelt_ledger`
/// SQL at all — the skip is now driven by the run's resolved
/// `StateAvailability` (`docs/outcomes/20260904-state-residency/
/// outcome.md` phase 5), not a raw `backend.dialect() == DuckDB` check.
/// The old `RunReporter` stand-in method for this skip is retired entirely
/// (phase 6): the affected cell's own recorded `MaintenanceStateDowngraded` is the
/// user-visible channel now, surfaced by `smelt explain`
/// (`crates/smelt-cli/tests/explain_maintenance.rs`) — this test asserts
/// only the emitted-statement set, which is the half this crate owns.
/// Uses a fully mocked `Backend` (never a real connection) so the dialect
/// mismatch between the claimed `SqlDialect::SparkSQL` and no real Spark
/// engine can never itself cause a spurious failure — this test is about
/// which SQL gets BUILT, not whether it executes against a live warehouse.
#[tokio::test]
async fn ledger_reset_is_skipped_on_a_non_duckdb_dialect() {
    struct NonDuckDbBackend {
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Backend for NonDuckDbBackend {
        async fn execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
            self.calls.lock().unwrap().push(sql.to_string());
            Ok(vec![])
        }
        async fn create_table_as(
            &self,
            _schema: &str,
            _name: &str,
            sql: &str,
        ) -> Result<(), BackendError> {
            self.calls.lock().unwrap().push(sql.to_string());
            Ok(())
        }
        async fn create_view_as(
            &self,
            _schema: &str,
            _name: &str,
            _sql: &str,
        ) -> Result<(), BackendError> {
            unreachable!()
        }
        async fn drop_table_if_exists(
            &self,
            _schema: &str,
            _name: &str,
        ) -> Result<(), BackendError> {
            Ok(())
        }
        async fn drop_view_if_exists(
            &self,
            _schema: &str,
            _name: &str,
        ) -> Result<(), BackendError> {
            Ok(())
        }
        async fn get_row_count(&self, _schema: &str, _name: &str) -> Result<usize, BackendError> {
            Ok(0)
        }
        async fn get_preview(
            &self,
            _schema: &str,
            _name: &str,
            _limit: usize,
        ) -> Result<Vec<RecordBatch>, BackendError> {
            Ok(vec![])
        }
        async fn table_exists(&self, _schema: &str, _name: &str) -> Result<bool, BackendError> {
            Ok(true)
        }
        async fn ensure_schema(&self, _schema: &str) -> Result<(), BackendError> {
            Ok(())
        }
        fn dialect(&self) -> SqlDialect {
            SqlDialect::SparkSQL
        }
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities::spark()
        }
        async fn load_table(
            &self,
            _schema: &str,
            _name: &str,
            _arrow_schema: SchemaRef,
            _batches: Vec<RecordBatch>,
        ) -> Result<(), BackendError> {
            unreachable!()
        }
        async fn delete_partitions(
            &self,
            _schema: &str,
            _name: &str,
            _partition: &PartitionRange,
        ) -> Result<(), BackendError> {
            unreachable!()
        }
        async fn insert_into_from_query(
            &self,
            _schema: &str,
            _name: &str,
            _sql: &str,
        ) -> Result<(), BackendError> {
            unreachable!()
        }
        async fn insert_overwrite(
            &self,
            _schema: &str,
            _table: &str,
            _sql: &str,
            _partition: &PartitionRange,
        ) -> Result<(), BackendError> {
            unreachable!()
        }
    }

    struct NonDuckDbFactory {
        calls: Arc<Mutex<Vec<String>>>,
    }
    impl BackendFactory for NonDuckDbFactory {
        fn create<'a>(
            &'a self,
            _target_name: &'a str,
            _target_config: &'a Target,
            _project_dir: &'a Path,
        ) -> BackendFuture<'a> {
            let calls = Arc::clone(&self.calls);
            Box::pin(async move { Ok(Box::new(NonDuckDbBackend { calls }) as Box<dyn Backend>) })
        }
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    write_model(
        project_dir,
        "daily_events",
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: partition\n\
         timeseries:\n\
         \x20\x20partition_column: event_date\n\
         \x20\x20event_time_column: event_date\n\
         \x20\x20granularity: day\n\
         ---\n\
         SELECT * FROM (VALUES (DATE '2024-01-01', 10)) AS t(event_date, amount)",
    );
    // `type: spark` (`docs/outcomes/20260904-state-residency/outcome.md`
    // phase 5): the run's availability resolution reads the target's
    // *declared* dialect from `smelt.yml`
    // (`sql_dialect_for_target`/`availability_for_run`), never the mocked
    // backend's own `dialect()` claim — so this fixture's target type must
    // itself say `spark` for the ledger-less skip this test exercises to
    // actually be reached.
    let smelt_yml = "name: ledger_skip_test\nversion: 1\npaths:\n  - models\ntargets:\n  \
                      dev:\n    type: spark\n    schema: main\n\
                      default_materialization: table\ntarget: dev\n";
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();

    let config = Arc::new(Config::load(project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(project_dir, &config);

    let factory = NonDuckDbFactory {
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let calls_handle = Arc::clone(&factory.calls);
    execute_project(
        "ledger-skip-run".to_string(),
        make_request("dev", "2024-01-01", "2024-01-02"),
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &factory,
        &NO_OP_REPORTER,
        CancellationToken::new(),
    )
    .await
    .expect("a run over a non-DuckDB backend must still succeed");

    let calls = calls_handle.lock().unwrap();
    assert!(
        !calls.iter().any(|c| c.contains("_smelt_ledger")),
        "a non-DuckDB dialect must emit no ledger-reset SQL at all: {calls:?}"
    );
}
