use super::*;

/// A `write: keyed`/`keyed_conditional` pin on a backend that cannot run
/// `MERGE` at all must refuse the run before any write — the pin selects
/// the `MERGE` mechanism explicitly, so the driver must never silently
/// substitute the merge-less staged-candidate mechanism instead (`docs/
/// outcomes/20260815-definition-delta-migrate/phases/27g-plan.md`).
#[tokio::test]
async fn keyed_pin_on_a_merge_less_backend_refuses_before_any_write() {
    use smelt_logical::maintenance::choice::WriteSuppression;

    struct MergeLessBackend {
        calls: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Backend for MergeLessBackend {
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
            // Existing target — reaches the merge/write-mechanism branch,
            // not the first-run create.
            Ok(true)
        }
        async fn ensure_schema(&self, _schema: &str) -> Result<(), BackendError> {
            Ok(())
        }
        fn dialect(&self) -> SqlDialect {
            SqlDialect::SparkSQL
        }
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities {
                supports_merge: false,
                ..BackendCapabilities::spark()
            }
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
            _partitions: &PartitionRange,
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

    let backend = MergeLessBackend {
        calls: Mutex::new(Vec::new()),
    };
    let classification = CumulativeClassification {
        unique_key: vec!["device_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "event_count".to_string(),
            per_partition_agg: "COUNT".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Sum,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.events".to_string(),
            timeseries: None,
        },
    };
    let steps = driving_steps(
        "2024-01-01",
        "2024-01-02",
        &smelt_core::config::Granularity::Day,
    )
    .expect("steps");
    let suppression = WriteSuppression::Unconditional {
        why: "test exercises the keyed pin refusal, not suppression".to_string(),
    };
    let pin = smelt_logical::maintenance::lookup_write_pattern("keyed").expect("registered");

    let result = run_windowed_keyed_maintenance(
        &backend,
        "device_daily",
        "main",
        "device_daily",
        &steps,
        &classification,
        None,
        &suppression,
        Some(pin),
        |step| {
            Ok(format!(
                "SELECT device_id, COUNT(*) AS event_count FROM events WHERE d = '{}' GROUP BY \
                 device_id",
                step.partition_value
            ))
        },
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
    )
    .await;

    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("device_daily"),
        "error must name the model: {err}"
    );
    assert!(err.contains("keyed"), "error must name the pin: {err}");
    assert!(
        backend.calls.lock().unwrap().is_empty(),
        "no write statement must be issued once the pin refuses: {:?}",
        backend.calls.lock().unwrap()
    );
}

/// The `smelt explain` `KeyedFold` preview for a state-bearing model (`AVG`,
/// `docs/outcomes/20260809-rung2-state-shapes` row 7) must carry the same
/// state-column folds as the executed `MERGE` — both now go through the
/// same single-owner `expand_aggregator_column_folds`
/// (`smelt_logical::maintenance::emit`, row 7's "single-owner statement
/// rule" move) and the same pre-compile `state_augmented_projection` step,
/// so they can never diverge.
#[tokio::test]
async fn keyed_fold_preview_matches_executed_statement_for_state_bearing_model() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    write_model(
        project_dir,
        "events",
        "---\n\
         materialization: table\n\
         timeseries:\n\
         \x20\x20partition_column: event_date\n\
         \x20\x20event_time_column: event_date\n\
         \x20\x20granularity: day\n\
         ---\n\
         SELECT * FROM (VALUES \
         (DATE '2024-01-01', 1, 10.0), \
         (DATE '2024-01-02', 1, 20.0), \
         (DATE '2024-01-02', 2, 30.0)) \
         AS t(event_date, device_id, amount)",
    );
    write_model(
        project_dir,
        "device_avg_amount",
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         ---\n\
         SELECT device_id, AVG(amount) AS avg_amount \
         FROM smelt.events GROUP BY device_id",
    );

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: keyed_avg_statement_parity_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    let config = Arc::new(Config::load(project_dir).expect("load config"));

    let (db, graph) = build_db_and_graph(project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };

    // One window covering both driving-source partitions: step 1
    // (2024-01-01) hits the first-run CREATE arm; step 2 (2024-01-02) hits
    // the MERGE arm — the one this test inspects.
    let request = make_request("dev", "2024-01-01", "2024-01-03");
    let outcome = execute_project(
        "keyed-avg-statement-parity-run".to_string(),
        request,
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("execute_project (keyed, state-bearing)");
    assert!(
        outcome.models.contains_key("device_avg_amount"),
        "device_avg_amount must have run: {:?}",
        outcome.models.keys().collect::<Vec<_>>()
    );

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    // `AVG`'s hidden state is the first *additive* state this mechanism
    // admits (`docs/outcomes/20260809-rung2-state-shapes` row 7), so the
    // cell now grades `Grade::Additive` and routes through the
    // reconciliation-ledger path (`maintenance_driver::run_windowed_keyed_
    // maintenance`'s ledger-interleaved arm) — its statements go through
    // `Backend::execute_sql` directly, not `execute_statement_group`, so
    // this test reads `recorded_sql`, not `recorded_groups` (unlike the
    // `Idempotent`-graded `MIN`/`MAX` cells `keyed_fold_statements_come_
    // from_the_emitter` above inspects).
    let sql_log = backend.recorded_sql();
    let executed_merge_sql = sql_log
        .iter()
        .find(|sql| sql.starts_with("MERGE INTO main.device_avg_amount"))
        .cloned()
        .unwrap_or_else(|| panic!("no executed MERGE statement found: {sql_log:?}"));
    assert!(
        executed_merge_sql
            .contains("avg_amount__sum = target.avg_amount__sum + delta.avg_amount__sum")
            && executed_merge_sql
                .contains("avg_amount__count = target.avg_amount__count + delta.avg_amount__count"),
        "expected the executed MERGE to fold the hidden sum/count state additively: \
         {executed_merge_sql}"
    );

    // Now build the `smelt explain` `KeyedFold` preview for the same model
    // and assert it carries the identical state-column fold expressions.
    let sql_models =
        smelt_core::ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone())
            .discover_models()
            .expect("discover_models");
    let model = sql_models
        .iter()
        .find(|m| m.canonical_path() == "device_avg_amount")
        .expect("device_avg_amount model discovered");
    let metadata = model
        .metadata
        .as_deref()
        .expect("device_avg_amount declares frontmatter");
    let stripped_sql = smelt_parser::strip_frontmatter(&model.content).to_string();
    let sources = vec![smelt_logical::maintenance::SourceFacts {
        name: "events".to_string(),
        mutation: smelt_logical::maintenance::MutationProfile::AppendOnly,
        partition_col: Some("event_date".to_string()),
        unique_key: vec![],
        allow_full_scan: true,
    }];
    let plan_result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        &stripped_sql,
        "device_avg_amount",
        metadata,
        &sources,
        &std::collections::HashSet::new(),
        None,
        &[],
        &[],
        &std::collections::BTreeMap::new(),
        None,
        None,
        &[],
    )
    .expect("device_avg_amount must derive a maintenance plan");
    let cell = plan_result
        .plan
        .cells
        .iter()
        .find(|c| c.technique == smelt_logical::maintenance::Technique::KeyedFold)
        .expect("device_avg_amount must admit a KeyedFold cell");

    let registry = smelt_runtime::CompilerRegistry::new(&config, &config.targets);
    let resolver = registry
        .get("dev")
        .build_ephemeral_resolver(&[], "main")
        .expect("no ephemerals");
    let graph_locked = graph.lock().await;
    let source_timeseries = smelt_runtime::build_source_timeseries_map(&graph_locked, &[]);
    drop(graph_locked);

    let plan_cell_diagnostics = smelt_runtime::diagnostics::build_plan_cell_diagnostics(
        cell,
        model,
        "main",
        "dev",
        &registry,
        &resolver,
        MaintenanceDialect::DuckDb,
        &[],
        &source_timeseries,
        &plan_result.column_groups,
    );
    let preview = plan_cell_diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == smelt_logical::maintenance::Technique::KeyedFold)
        .expect("a KeyedFold preview must always be present");
    let preview_sql = preview
        .statements
        .first()
        .expect("the KeyedFold preview must render a statement")
        .sql
        .clone();

    for fragment in [
        "avg_amount__sum = target.avg_amount__sum + delta.avg_amount__sum",
        "avg_amount__count = target.avg_amount__count + delta.avg_amount__count",
        "avg_amount = (target.avg_amount__sum + delta.avg_amount__sum) / \
         (target.avg_amount__count + delta.avg_amount__count)",
    ] {
        assert!(
            preview_sql.contains(fragment) && executed_merge_sql.contains(fragment),
            "preview and executed statement must carry the identical state-column fold \
             `{fragment}` — preview: {preview_sql}\nexecuted: {executed_merge_sql}"
        );
    }

    // Phase 27a (`docs/outcomes/20260815-definition-delta-migrate/phases/
    // 27a-plan.md`): the preview's own change-suppressed matched-arm guard
    // must be byte-identical to what the live run actually executed — never
    // a preview that renders the unconditional arm while the live run
    // suppressed, or vice versa. `avg_amount` is a plain `AVG` (registry-
    // backed, P3 `Comparable`) over a proven `Key` row identity, so both
    // resolve `WriteSuppression::Suppressed` here.
    let guard = "target.avg_amount IS DISTINCT FROM \
                 ((target.avg_amount__sum + delta.avg_amount__sum) / \
                 (target.avg_amount__count + delta.avg_amount__count))";
    assert!(
        preview_sql.contains(guard) && executed_merge_sql.contains(guard),
        "preview and executed statement must carry the identical change-suppressed guard — \
         preview: {preview_sql}\nexecuted: {executed_merge_sql}"
    );
}

/// The slice-predicated keyed-fold family: a `refresh: keyed` model that
/// also declares its own `timeseries:` block, admitted through key temporal
/// locality's route 1 (key-embedded — `partition_column` is itself a
/// `unique_key` column, `docs/specs/incremental_shapes.md` §"Key temporal
/// locality (the time-partitioned output)"; `docs/plans/20260715-composed-
/// axes-conditional-maintenance.md` Phase A2). The established
/// [`smelt_logical::maintenance::locality::LocalitySlice`] licenses a
/// `target.<partition_column> BETWEEN ...` predicate on the `MERGE`'s `ON`
/// clause (`emit_keyed_fold`'s `slice` parameter) — this is
/// `keyed_fold_statements_come_from_the_emitter` above with one addition (the
/// keyed model's own `timeseries:` block), proving the *slice-carrying*
/// MERGE `execute_project` actually runs is still byte-identical to a direct
/// `emit_keyed_fold` call with that same slice, not merely slice-shaped
/// text.
///
/// `MAX` (not `SUM`) keeps the cell `Grade::Idempotent`
/// (`WindowedKeyedRule::ledger_grade`), so the step routes through
/// `Backend::execute_statement_group` — the funnel this test's
/// `RecordingBackend` records — rather than the ledger-interleaved additive
/// path, matching `keyed_fold_statements_come_from_the_emitter`'s own choice
/// of combiner.
///
/// This is also the doubly-predicated statement-parity leg
/// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase
/// C6): `max_amount` is Comparable (a registry-backed deterministic
/// aggregate) over the proven `{device_id, event_date}` key, so a real run
/// now resolves `WriteSuppression::Suppressed`
/// (`resolve_cumulative_write_suppression`, wired into `execute_cumulative_
/// aggregate`) — the executed `MERGE` carries **both** the slice predicate
/// on the `ON` clause's target read AND the `IS DISTINCT FROM` suppression
/// arm on the matched clause, byte-identical to a direct `emit_keyed_fold_
/// suppressed` call with the same slice.
#[tokio::test]
async fn keyed_fold_slice_predicated_merge_statements_come_from_the_emitter() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();

    std::fs::write(
        project_dir.join("models/sources/events.yml"),
        "description: statement-parity locality source.\n\
         mutation_profile: append_only\n\
         timeseries:\n\
         \x20\x20event_time_column: event_date\n\
         \x20\x20partition_column: event_date\n\
         \x20\x20granularity: day\n\
         columns:\n\
         \x20\x20- name: device_id\n\
         \x20\x20\x20\x20type: INTEGER\n\
         \x20\x20- name: event_date\n\
         \x20\x20\x20\x20type: DATE\n\
         \x20\x20- name: amount\n\
         \x20\x20\x20\x20type: DOUBLE\n",
    )
    .unwrap();

    // Route 1 (key-embedded): `event_date` is both the model's own
    // `timeseries.partition_column` and a `unique_key` column (GROUP BY 1,
    // 2) — the same composed shape `crates/smelt-runtime/tests/
    // locality_route1_slice_pruning.rs` exercises end-to-end (result
    // equivalence + slice-shape assertions). This test's own contribution is
    // the statement-parity leg: byte-identity against a direct emitter
    // call, the missing coverage this phase's review flagged.
    write_model(
        project_dir,
        "device_daily",
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         timeseries:\n\
         \x20\x20event_time_column: event_date\n\
         \x20\x20partition_column: event_date\n\
         \x20\x20granularity: day\n\
         maintenance:\n\
         \x20\x20scan_bounds:\n\
         \x20\x20\x20\x20per_source:\n\
         \x20\x20\x20\x20\x20\x20events:\n\
         \x20\x20\x20\x20\x20\x20\x20\x20allow_full_scan: true\n\
         ---\n\
         SELECT device_id, event_date, MAX(amount) AS max_amount \
         FROM smelt.sources.events GROUP BY device_id, event_date",
    );

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: keyed_slice_statement_parity_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    {
        let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
        conn.execute_batch(
            "CREATE SCHEMA IF NOT EXISTS main;\n\
             CREATE TABLE main.sources_events (device_id INTEGER, event_date DATE, amount DOUBLE);\n\
             INSERT INTO main.sources_events VALUES \
             (1, DATE '2024-01-01', 10.0), \
             (1, DATE '2024-01-02', 20.0), \
             (2, DATE '2024-01-02', 5.0);",
        )
        .expect("seed source table");
    }

    let config = Arc::new(Config::load(project_dir).expect("load config"));

    // Window 1: day 1 alone — first-run CREATE TABLE ... AS, no MERGE yet.
    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
        let factory = RecordingBackendFactory {
            db_path: db_path.clone(),
            backend: Arc::clone(&backend_slot),
        };
        execute_project(
            "keyed-slice-statement-parity-run-1".to_string(),
            make_request("dev", "2024-01-01", "2024-01-02"),
            Arc::clone(&config),
            graph,
            db,
            project_dir,
            &factory,
            &smelt_runtime::NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("window 1 (create) must run");
    }

    // Window 2: day 2 alone — a single MERGE step carrying the locality
    // slice (zero margin, since the model's SQL has no lookback construct:
    // the slice is exactly this step's own date).
    let (db, graph) = build_db_and_graph(project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    execute_project(
        "keyed-slice-statement-parity-run-2".to_string(),
        make_request("dev", "2024-01-02", "2024-01-03"),
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("window 2 (slice-predicated merge) must run");

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    assert_eq!(
        groups.len(),
        1,
        "window 2 covers exactly one day-step, one MERGE group: {:?}",
        groups
    );

    let merge_sql = &groups[0].statements[0].sql;
    assert_eq!(groups[0].statements.len(), 1);

    let key = vec!["device_id".to_string(), "event_date".to_string()];
    let folds = vec![(
        "max_amount".to_string(),
        "GREATEST(target.max_amount, delta.max_amount)".to_string(),
    )];
    let slice = TargetSlicePredicate::Range {
        partition_column: "event_date".to_string(),
        lower: "2024-01-02".to_string(),
        upper: "2024-01-02".to_string(),
    };

    let prefix = "MERGE INTO main.device_daily AS target USING (";
    let suffix = ") AS delta ON target.device_id = delta.device_id AND \
                  target.event_date = delta.event_date AND \
                  target.event_date BETWEEN '2024-01-02' AND '2024-01-02' \
                  WHEN MATCHED AND (target.max_amount IS DISTINCT FROM (GREATEST(target.\
                  max_amount, delta.max_amount))) THEN UPDATE SET \
                  max_amount = GREATEST(target.max_amount, delta.max_amount) \
                  WHEN NOT MATCHED THEN INSERT *";
    assert!(
        merge_sql.starts_with(prefix) && merge_sql.ends_with(suffix),
        "unexpected slice-predicated merge statement: {merge_sql}"
    );
    assert!(
        merge_sql.contains("BETWEEN") && merge_sql.contains("IS DISTINCT FROM"),
        "the composed model's suppressed merge must carry BOTH the slice predicate and the \
         suppression arm: {merge_sql}"
    );
    let delta_select = &merge_sql[prefix.len()..merge_sql.len() - suffix.len()];

    let expected = emit_keyed_fold_suppressed(
        "main.device_daily",
        &key,
        &folds,
        delta_select,
        Some(&slice),
        &["max_amount".to_string()],
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, &groups[0],
        "executed slice-predicated MERGE group must be byte-identical to a direct emitter call \
         over the same table/key/folds/slice/delta_select"
    );

    // Result-equivalence: the CREATE (window 1) + slice-predicated MERGE
    // (window 2) statements the run actually executed must leave
    // `device_daily` multiset-equal to a full refresh of the model's own
    // aggregation over every seeded row.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT * FROM main.device_daily",
            "SELECT device_id, event_date, MAX(amount) AS max_amount \
             FROM main.sources_events GROUP BY device_id, event_date"
        )
        .await,
        "the CREATE+slice-predicated-MERGE statements execute_project actually ran must \
         reproduce a full refresh"
    );
}

/// Statement-parity leg for the **checked route-3** (recurrence-bounded,
/// declared `r`) merge (`docs/specs/incremental_shapes.md` §"Key temporal
/// locality", route 3; `docs/plans/20260715-composed-axes-conditional-
/// maintenance.md` Phase A4): the out-of-slice match probe and the merge
/// itself are each byte-identical to a direct call of their single-owner
/// emitters (`emit_recurrence_bound_probe`, `emit_keyed_fold`).
///
/// Driven directly through `maintenance_driver::run_windowed_keyed_
/// maintenance` (not the full `execute_project` pipeline): route 3's
/// flagship shape needs an extremal-fold (`MIN`/`MAX`) partition column,
/// which trips the *unrelated* NOT-NULL diagnostic `execute_project`'s
/// pre-execution gate enforces regardless of locality admission — the
/// same pre-existing blocker `docs/specs/incremental_models.md` §Known
/// Divergences documents for route 2's own real-fixture coverage. Calling
/// the driver directly still proves the actual SQL a run executes matches
/// the emitters, the parity gate's whole point; it does not touch the
/// (separately tracked) nullability gap.
#[tokio::test]
async fn recurrence_bound_probe_and_checked_merge_come_from_the_emitters() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
        conn.execute_batch(
            "CREATE SCHEMA IF NOT EXISTS main;\n\
             CREATE TABLE main.raw_events (event_id INTEGER, event_ts TIMESTAMP, event_date DATE);",
        )
        .expect("create raw_events");
    }
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open backend");
    let backend = RecordingBackend::new(inner);

    let classification = CumulativeClassification {
        unique_key: vec!["event_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "last_seen_date".to_string(),
            per_partition_agg: "MAX".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Max,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.sources.raw.events".to_string(),
            timeseries: Some(smelt_core::config::TimeseriesConfig {
                event_time_column: "event_ts".to_string(),
                partition_column: "event_date".to_string(),
                granularity: smelt_core::config::Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
        },
    };
    let slice = LocalitySlice::RecurrenceBounded {
        partition_column: "last_seen_date".to_string(),
        margin_before: smelt_logical::analysis::source_bounds::Seconds::days(3),
        margin_after: smelt_logical::analysis::source_bounds::Seconds::ZERO,
        r: smelt_logical::analysis::source_bounds::Seconds::days(3),
    };
    let compile_step = |step: &smelt_runtime::maintenance_driver::MaintenanceStep| {
        Ok(format!(
            "SELECT event_id, MAX(event_date) AS last_seen_date FROM main.raw_events \
             WHERE event_date = '{}' GROUP BY event_id",
            step.partition_value
        ))
    };

    backend
        .execute_sql(
            "INSERT INTO main.raw_events VALUES (1, TIMESTAMP '2026-02-01 00:00:00', DATE \
             '2026-02-01')",
        )
        .await
        .expect("insert day 1");
    let create_steps = driving_steps(
        "2026-02-01",
        "2026-02-02",
        &smelt_core::config::Granularity::Day,
    )
    .expect("steps");
    run_windowed_keyed_maintenance(
        &backend,
        "events_last_seen",
        "main",
        "events_last_seen",
        &create_steps,
        &classification,
        Some(&slice),
        &smelt_logical::maintenance::choice::WriteSuppression::Unconditional {
            why: "test asserts the unconditional checked-merge shape".to_string(),
        },
        None,
        compile_step,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
    )
    .await
    .expect("day 1 create must succeed");

    // Day 2: an in-bound redelivery — the probe must run, find no
    // violation, and the merge must apply.
    backend
        .execute_sql(
            "INSERT INTO main.raw_events VALUES (1, TIMESTAMP '2026-02-02 00:00:00', DATE \
             '2026-02-02')",
        )
        .await
        .expect("insert day 2");
    let steps = driving_steps(
        "2026-02-02",
        "2026-02-03",
        &smelt_core::config::Granularity::Day,
    )
    .expect("steps");
    run_windowed_keyed_maintenance(
        &backend,
        "events_last_seen",
        "main",
        "events_last_seen",
        &steps,
        &classification,
        Some(&slice),
        &smelt_logical::maintenance::choice::WriteSuppression::Unconditional {
            why: "test asserts the unconditional checked-merge shape".to_string(),
        },
        None,
        compile_step,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
    )
    .await
    .expect("in-bound redelivery must merge cleanly");

    // The probe: byte-identical to a direct `emit_recurrence_bound_probe`
    // call over this step's own delta SELECT and slice lower bound
    // (2026-02-02 widened backward by r=3 days → 2026-01-30).
    let executed = backend.recorded_sql();
    let probe_sql = executed
        .iter()
        .find(|s| s.contains("__recurrence_violations"))
        .expect("the checked route must execute the out-of-slice match probe");
    let delta_select = "SELECT event_id, MAX(event_date) AS last_seen_date FROM main.raw_events \
                         WHERE event_date = '2026-02-02' GROUP BY event_id";
    let expected_probe = emit_recurrence_bound_probe(
        "main.events_last_seen",
        &["event_id".to_string()],
        "last_seen_date",
        delta_select,
        "2026-01-30",
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        probe_sql, &expected_probe.sql,
        "executed probe must be byte-identical to a direct emitter call"
    );

    // The merge: byte-identical to a direct `emit_keyed_fold` call with the
    // same `Range` predicate the checked route resolves to (same shape as
    // route 1's window).
    let groups = backend.recorded_groups();
    let merge_group = groups
        .iter()
        .find(|g| g.statements[0].sql.starts_with("MERGE INTO"))
        .expect("the merge action must have executed via execute_statement_group");
    let range_slice = TargetSlicePredicate::Range {
        partition_column: "last_seen_date".to_string(),
        lower: "2026-01-30".to_string(),
        upper: "2026-02-02".to_string(),
    };
    let expected_merge = emit_keyed_fold(
        "main.events_last_seen",
        &["event_id".to_string()],
        &[(
            "last_seen_date".to_string(),
            "GREATEST(target.last_seen_date, delta.last_seen_date)".to_string(),
        )],
        delta_select,
        Some(&range_slice),
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        merge_group, &expected_merge,
        "executed checked-merge group must be byte-identical to a direct emitter call"
    );
}
