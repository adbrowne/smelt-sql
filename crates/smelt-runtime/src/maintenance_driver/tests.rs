use super::*;
use anyhow::Result;
use async_trait::async_trait;
use smelt_backend::{Backend, BackendError};
use smelt_core::config::Granularity;
use smelt_dialect::BackendCapabilities;
use smelt_dialect::SqlDialect;
use smelt_logical::analysis::join_shape::ContributionVerdict;
use smelt_logical::analysis::source_bounds::BoundResult;
use smelt_logical::analysis::source_bounds::Seconds;
use smelt_logical::maintenance::choice::WriteSuppression;
use smelt_logical::maintenance::derive::SourceReferentialIntegrity;
use smelt_logical::maintenance::emit::{MaintenanceDialect, TargetSlicePredicate};
use smelt_logical::maintenance::locality::LocalitySlice;
use smelt_logical::maintenance::{
    PartitionLocal, PlanCell, RowPreservation, ScanClamp, SkeletonSourceClosure, SourceFacts,
    Technique, Trigger,
};
use smelt_state::reconciliation::Grade;
use std::collections::HashSet;
use std::sync::Mutex;

/// A retry policy that never retries — these unit tests exercise the
/// driver against `RecordingBackend`, a synchronous test double, so
/// there is no `ExecuteRequest`/run reporter to derive a policy from
/// (`docs/plans/20260719-prod-w2-operability.md` Phase 6). Retry
/// behaviour itself is covered end-to-end by `tests/retry.rs`.
const NO_OP_REPORTER: crate::reporter::NoOpReporter = crate::reporter::NoOpReporter;
fn no_retry_policy() -> crate::execute::RetryPolicy<'static> {
    crate::execute::RetryPolicy {
        retry_max: 0,
        base_backoff_ms: 0,
        run_id: "maintenance-driver-unit-test",
        model_name: "maintenance-driver-unit-test",
        reporter: &NO_OP_REPORTER,
    }
}

// ── 27e: resolve_live_external_delta_restriction_facts ────────────────
// (`docs/outcomes/20260815-definition-delta-migrate/phases/27e-plan.md`)

/// The real `examples/timeseries/daily_events_enriched.sql` shape's SQL
/// body (frontmatter stripped) — the SAME fixture
/// `crates/smelt-runtime/tests/technique_lowering.rs`'s
/// `external_source_point_lookup_recompute` module reads, so this unit
/// test can never silently drift from what that end-to-end suite pins.
fn external_facts_model_sql_body() -> (smelt_core::ModelMetadata, String) {
    let project_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
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

fn external_facts_source_facts(users_unique_key: Vec<String>) -> Vec<SourceFacts> {
    vec![
        SourceFacts {
            name: "raw.events".to_string(),
            mutation: smelt_logical::maintenance::MutationProfile::AppendOnly,
            partition_col: None,
            unique_key: vec!["event_id".to_string()],
            allow_full_scan: false,
        },
        SourceFacts {
            name: "raw.users".to_string(),
            mutation: smelt_logical::maintenance::MutationProfile::MutableSnapshot,
            partition_col: None,
            unique_key: users_unique_key,
            allow_full_scan: true,
        },
    ]
}

fn external_facts_explicitly_mutable() -> HashSet<String> {
    std::iter::once("raw.users".to_string()).collect()
}

fn external_facts_source_ri() -> SourceReferentialIntegrity {
    let mut ri = SourceReferentialIntegrity::new();
    ri.insert("raw.users".to_string(), vec!["user_id".to_string()]);
    ri
}

/// A declared-RI closure (P1 `Closed`) plus a single-column declared
/// `unique_key` resolves external delta-restriction facts for the
/// `{user_name}` cell on `raw.users` — the golden path this whole
/// mechanism exists for.
#[test]
fn external_facts_resolve_for_a_declared_mutable_dimension() {
    let (metadata, sql_body) = external_facts_model_sql_body();
    let sources = external_facts_source_facts(vec!["user_id".to_string()]);
    let explicitly_mutable = external_facts_explicitly_mutable();
    let facts = resolve_live_external_delta_restriction_facts(
        &sql_body,
        "daily_events_enriched",
        &metadata,
        &sources,
        &explicitly_mutable,
        &external_facts_source_ri(),
        true,
        &smelt_logical::maintenance::availability::StateAvailability::all(),
    )
    .expect("resolver does not refuse")
    .expect("facts resolve for a declared-RI mutable dimension");
    assert_eq!(facts.source_name, "raw.users");
    assert_eq!(facts.restrict_column.as_deref(), Some("user_id"));
    assert_eq!(
        facts.skeleton_source_closure,
        Some(SkeletonSourceClosure::Closed {
            row_preservation: RowPreservation::DeclaredReferentialIntegrity {
                source: "raw.users".to_string()
            }
        })
    );
}

/// A composite `unique_key` on the driving source makes
/// `enrichment_restrict_column` return `None` — this phase's semi-join
/// restriction is single-column only — so the resolver returns `None`
/// (falling back to the widened scan) rather than a facts value with no
/// restrict column.
#[test]
fn external_facts_refuse_a_composite_dimension_key() {
    let (metadata, sql_body) = external_facts_model_sql_body();
    let sources = external_facts_source_facts(vec!["user_id".to_string(), "region".to_string()]);
    let explicitly_mutable = external_facts_explicitly_mutable();
    let facts = resolve_live_external_delta_restriction_facts(
        &sql_body,
        "daily_events_enriched",
        &metadata,
        &sources,
        &explicitly_mutable,
        &external_facts_source_ri(),
        true,
        &smelt_logical::maintenance::availability::StateAvailability::all(),
    )
    .expect("resolver does not refuse");
    assert!(
        facts.is_none(),
        "a composite unique_key must fall back to the widened scan, got {facts:?}"
    );
}

/// `supports_fingerprint_sidecar: false` refuses external delta
/// restriction outright — the fallback is capability-driven, not a
/// property re-derived from the closure/key facts alone.
#[test]
fn external_facts_refuse_without_the_sidecar_capability() {
    let (metadata, sql_body) = external_facts_model_sql_body();
    let sources = external_facts_source_facts(vec!["user_id".to_string()]);
    let explicitly_mutable = external_facts_explicitly_mutable();
    let facts = resolve_live_external_delta_restriction_facts(
        &sql_body,
        "daily_events_enriched",
        &metadata,
        &sources,
        &explicitly_mutable,
        &external_facts_source_ri(),
        false,
        &smelt_logical::maintenance::availability::StateAvailability::all(),
    )
    .expect("resolver does not refuse");
    assert!(
        facts.is_none(),
        "supports_fingerprint_sidecar: false must refuse, got {facts:?}"
    );
}

/// The rejection test at the `maintenance_driver` call site: for
/// `MaintenanceDialect::BigQuery` the emitted affected-keys relation is
/// never a `FROM (VALUES …)` table-value constructor, and for
/// DuckDB/Spark it is byte-identical (both route through the shared
/// `smelt_core::build_row_set_table` owner).
#[test]
fn repair_keys_literal_select_bigquery_is_not_a_values_constructor() {
    let keys = vec!["a".to_string(), "b".to_string()];
    let select = repair_keys_literal_select(&keys, MaintenanceDialect::BigQuery);
    assert!(
        !select.contains("VALUES"),
        "BigQuery has no table-value constructor, got: {select}"
    );
    assert!(select.contains("UNION ALL"));
}

#[test]
fn repair_keys_literal_select_duckdb_and_spark_are_byte_identical() {
    let keys = vec!["a".to_string(), "b".to_string()];
    let duckdb = repair_keys_literal_select(&keys, MaintenanceDialect::DuckDb);
    let spark = repair_keys_literal_select(&keys, MaintenanceDialect::Spark);
    assert_eq!(duckdb, spark);
    assert_eq!(
        duckdb,
        "SELECT * FROM (VALUES ('a'), ('b')) AS __smelt_repair_group_keys(delta_key)"
    );
}

#[test]
fn repair_keys_literal_select_empty_keys_is_dialect_independent() {
    let empty: Vec<String> = vec![];
    for dialect in [
        MaintenanceDialect::DuckDb,
        MaintenanceDialect::Spark,
        MaintenanceDialect::BigQuery,
    ] {
        let select = repair_keys_literal_select(&empty, dialect);
        assert_eq!(
            select,
            "SELECT CAST(NULL AS VARCHAR) AS delta_key WHERE FALSE"
        );
    }
}

#[test]
fn driving_steps_day_granularity_in_temporal_order() {
    let steps = driving_steps("2024-01-01", "2024-01-04", &Granularity::Day).unwrap();
    let values: Vec<&str> = steps.iter().map(|s| s.partition_value.as_str()).collect();
    assert_eq!(values, vec!["2024-01-01", "2024-01-02", "2024-01-03"]);
    assert_eq!(steps[0].range.start, "2024-01-01");
    assert_eq!(steps[0].range.end, "2024-01-02");
}

#[test]
fn driving_steps_week_granularity() {
    let steps = driving_steps("2024-01-01", "2024-01-15", &Granularity::Week).unwrap();
    let values: Vec<&str> = steps.iter().map(|s| s.partition_value.as_str()).collect();
    assert_eq!(values, vec!["2024-01-01", "2024-01-08"]);
    assert_eq!(steps[0].range.end, "2024-01-08");
}

#[test]
fn driving_steps_rejects_unsupported_granularity() {
    let err = driving_steps("2024-01-01", "2024-02-01", &Granularity::Month).unwrap_err();
    assert!(err.to_string().contains("day and week"));
}

#[test]
fn driving_steps_rejects_empty_window() {
    assert!(driving_steps("2024-01-05", "2024-01-01", &Granularity::Day).is_err());
}

/// The plain unconditional matched arm — the pre-Phase-C6 default for
/// tests below that don't exercise suppression itself.
fn unconditional_suppression() -> WriteSuppression {
    WriteSuppression::Unconditional {
        why: "test rule does not exercise write suppression".to_string(),
    }
}

/// A rule whose combiner set is never monoid-safe — the driver must
/// refuse the whole run rather than merge approximately.
struct AlwaysRefuses;

impl WindowedKeyedRule for AlwaysRefuses {
    fn refuse(&self) -> Option<String> {
        Some("non-monoid combiner (e.g. MEDIAN) cannot be merged".to_string())
    }
    fn merge_sql(
        &self,
        _schema: &str,
        _table: &str,
        _delta_sql: &str,
        _slice: Option<&TargetSlicePredicate>,
        _suppression: &WriteSuppression,
        _dialect: MaintenanceDialect,
    ) -> String {
        unreachable!("merge_sql must not be called once refuse() fires")
    }
}

/// An in-memory fake backend that records every call it receives so the
/// driver's classify → step → pushdown → create-or-merge sequencing can
/// be exercised without a real database.
struct RecordingBackend {
    table_exists: Mutex<bool>,
    calls: Mutex<Vec<String>>,
    dialect: SqlDialect,
}

impl Default for RecordingBackend {
    fn default() -> Self {
        RecordingBackend {
            table_exists: Mutex::new(false),
            calls: Mutex::new(Vec::new()),
            dialect: SqlDialect::DuckDB,
        }
    }
}

#[async_trait]
impl Backend for RecordingBackend {
    async fn execute_sql(&self, sql: &str) -> Result<Vec<arrow::array::RecordBatch>, BackendError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("execute_sql: {}", sql));
        // The `CREATE TABLE … AS` text now arrives here (via the
        // default `execute_statement_group` fallback, since this
        // driver no longer calls `Backend::create_table_as` for this
        // family) rather than through the dedicated `create_table_as`
        // method — flip the same flag a real backend's live
        // `table_exists` query would reflect after running it.
        if sql.starts_with("CREATE TABLE") {
            *self.table_exists.lock().unwrap() = true;
        }
        Ok(vec![])
    }
    async fn create_table_as(
        &self,
        _schema: &str,
        _name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("create_table_as: {}", sql));
        *self.table_exists.lock().unwrap() = true;
        Ok(())
    }
    async fn create_view_as(
        &self,
        _schema: &str,
        _name: &str,
        _sql: &str,
    ) -> Result<(), BackendError> {
        unreachable!("driver does not create views")
    }
    async fn drop_table_if_exists(&self, _schema: &str, _name: &str) -> Result<(), BackendError> {
        Ok(())
    }
    async fn drop_view_if_exists(&self, _schema: &str, _name: &str) -> Result<(), BackendError> {
        Ok(())
    }
    async fn get_row_count(&self, _schema: &str, _name: &str) -> Result<usize, BackendError> {
        Ok(self.calls.lock().unwrap().len())
    }
    async fn get_preview(
        &self,
        _schema: &str,
        _name: &str,
        _limit: usize,
    ) -> Result<Vec<arrow::array::RecordBatch>, BackendError> {
        Ok(vec![])
    }
    async fn table_exists(&self, _schema: &str, _name: &str) -> Result<bool, BackendError> {
        Ok(*self.table_exists.lock().unwrap())
    }
    async fn ensure_schema(&self, _schema: &str) -> Result<(), BackendError> {
        Ok(())
    }
    fn dialect(&self) -> SqlDialect {
        self.dialect
    }
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::duckdb()
    }
    async fn load_table(
        &self,
        _schema: &str,
        _name: &str,
        _arrow_schema: arrow::datatypes::SchemaRef,
        _batches: Vec<arrow::array::RecordBatch>,
    ) -> Result<(), BackendError> {
        unreachable!("driver does not load tables")
    }
    async fn delete_partitions(
        &self,
        _schema: &str,
        _name: &str,
        _partition: &smelt_backend::PartitionRange,
    ) -> Result<(), BackendError> {
        unreachable!("driver does not delete partitions")
    }
    async fn insert_into_from_query(
        &self,
        _schema: &str,
        _name: &str,
        _sql: &str,
    ) -> Result<(), BackendError> {
        unreachable!("driver does not insert-into")
    }
    async fn merge_into(
        &self,
        _schema: &str,
        _table: &str,
        _source_sql: &str,
        _unique_key: &[String],
        _columns: &[String],
    ) -> Result<(), BackendError> {
        unreachable!("driver merges via execute_sql, not native merge_into")
    }
    async fn insert_overwrite(
        &self,
        _schema: &str,
        _table: &str,
        _sql: &str,
        _partition: &smelt_backend::PartitionRange,
    ) -> Result<(), BackendError> {
        unreachable!("driver does not insert-overwrite")
    }
}

/// A monoid `SUM`-style rule: always safe, merges via a fixed template.
struct SumRule;

impl WindowedKeyedRule for SumRule {
    fn refuse(&self) -> Option<String> {
        None
    }
    fn merge_sql(
        &self,
        schema: &str,
        table: &str,
        delta_sql: &str,
        _slice: Option<&TargetSlicePredicate>,
        _suppression: &WriteSuppression,
        _dialect: MaintenanceDialect,
    ) -> String {
        format!("MERGE INTO {}.{} USING ({})", schema, table, delta_sql)
    }
}

/// Same as [`SumRule`] but opts into `Grade::Additive` ledger grading
/// (MP12) — exercises the driver's never-fold-twice wiring without a
/// real backend.
struct SumRuleAdditive;

impl WindowedKeyedRule for SumRuleAdditive {
    fn refuse(&self) -> Option<String> {
        None
    }
    fn merge_sql(
        &self,
        schema: &str,
        table: &str,
        delta_sql: &str,
        _slice: Option<&TargetSlicePredicate>,
        _suppression: &WriteSuppression,
        _dialect: MaintenanceDialect,
    ) -> String {
        format!("MERGE INTO {}.{} USING ({})", schema, table, delta_sql)
    }
    fn ledger_grade(&self) -> Grade {
        Grade::Additive
    }
    fn ledger_input(&self) -> &str {
        "smelt.events"
    }
}

#[tokio::test]
async fn refuses_before_any_backend_call() {
    let backend = RecordingBackend::default();
    let steps = driving_steps("2024-01-01", "2024-01-03", &Granularity::Day).unwrap();
    let result = run_windowed_keyed_maintenance(
        &backend,
        "model.under.test",
        "main",
        "t",
        &steps,
        &AlwaysRefuses,
        None,
        &unconditional_suppression(),
        None,
        |step| {
            Ok(format!(
                "SELECT * FROM src WHERE d = '{}'",
                step.partition_value
            ))
        },
        &no_retry_policy(),
        &crate::probes::ProbePolicy::per_run(),
    )
    .await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("non-monoid combiner"));
    assert!(backend.calls.lock().unwrap().is_empty());
}

/// A `write: staged_candidate` pin over an `Unconditional` verdict has no
/// sound realisation (`resolve_keyed_write_mechanism`'s own doc comment:
/// the staged-candidate emitter has no unconditional shape) — the
/// resulting `ChoiceRefusal` must propagate out of
/// `run_windowed_keyed_maintenance` as an error naming the model and the
/// pin, refused before any backend call, exactly like the combiner-safety
/// refusal above (`docs/outcomes/20260815-definition-delta-migrate/
/// phases/27g-plan.md`).
#[tokio::test]
async fn staged_candidate_pin_over_an_unconditional_cell_refuses() {
    let backend = RecordingBackend::default();
    let steps = driving_steps("2024-01-01", "2024-01-02", &Granularity::Day).unwrap();
    let pin = smelt_logical::maintenance::lookup_write_pattern("staged_candidate").unwrap();
    let result = run_windowed_keyed_maintenance(
        &backend,
        "model.under.test",
        "main",
        "t",
        &steps,
        &SumRule,
        None,
        &unconditional_suppression(),
        Some(pin),
        |step| {
            Ok(format!(
                "SELECT * FROM src WHERE d = '{}'",
                step.partition_value
            ))
        },
        &no_retry_policy(),
        &crate::probes::ProbePolicy::per_run(),
    )
    .await;
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("model.under.test"),
        "error must name the model: {err}"
    );
    assert!(
        err.contains("staged_candidate"),
        "error must name the pin: {err}"
    );
    assert!(backend.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn sequences_create_then_merge_across_partitions_in_temporal_order() {
    let backend = RecordingBackend::default();
    let steps = driving_steps("2024-01-01", "2024-01-04", &Granularity::Day).unwrap();
    run_windowed_keyed_maintenance(
        &backend,
        "model.under.test",
        "main",
        "t",
        &steps,
        &SumRule,
        None,
        &unconditional_suppression(),
        None,
        |step| {
            Ok(format!(
                "SELECT * FROM src WHERE d = '{}'",
                step.partition_value
            ))
        },
        &no_retry_policy(),
        &crate::probes::ProbePolicy::per_run(),
    )
    .await
    .unwrap();

    let calls = backend.calls.lock().unwrap();
    // Each of the 3 steps now also writes a re-run-tolerance bookkeeping
    // record into the merge ledger (`Backend::execute_write_with_
    // bookkeeping`'s default fallback: one `execute_sql` for the
    // ledger's idempotent ensure DDL, one for the `ON CONFLICT DO
    // NOTHING` upsert, then the create/merge statement itself) —
    // `SumRule`'s default `ledger_grade()` is `Grade::Idempotent`, and
    // `RecordingBackend`'s dialect is DuckDB (`docs/specs/
    // incremental_shapes.md` §"The transactional frontier write (merge
    // ledger)").
    assert_eq!(calls.len(), 9);
    // The first-run CREATE now comes from `emit_create_table_as`,
    // executed via `execute_statement_group` (its default sequential
    // fallback routes through `execute_sql`, since `RecordingBackend`
    // does not override `execute_statement_group`) — no more
    // `Backend::create_table_as` call for this family
    // (`docs/specs/incremental_models.md` §"Statement emission (single
    // owner)").
    assert!(calls[0].starts_with("execute_sql: CREATE TABLE IF NOT EXISTS main._smelt_ledger"));
    assert!(calls[1].starts_with("execute_sql: INSERT INTO main._smelt_ledger"));
    assert!(calls[1].contains("ON CONFLICT DO NOTHING"));
    assert!(calls[2].starts_with("execute_sql: CREATE TABLE main.t AS"));
    assert!(calls[2].contains("2024-01-01"));
    assert!(calls[3].starts_with("execute_sql: CREATE TABLE IF NOT EXISTS main._smelt_ledger"));
    assert!(calls[4].starts_with("execute_sql: INSERT INTO main._smelt_ledger"));
    assert!(calls[4].contains("ON CONFLICT DO NOTHING"));
    assert!(calls[5].starts_with("execute_sql: MERGE INTO main.t"));
    assert!(calls[5].contains("2024-01-02"));
    assert!(calls[6].starts_with("execute_sql: CREATE TABLE IF NOT EXISTS main._smelt_ledger"));
    assert!(calls[7].starts_with("execute_sql: INSERT INTO main._smelt_ledger"));
    assert!(calls[7].contains("ON CONFLICT DO NOTHING"));
    assert!(calls[8].starts_with("execute_sql: MERGE INTO main.t"));
    assert!(calls[8].contains("2024-01-03"));
}

/// A re-run-tolerant (`Grade::Idempotent`) keyed model on a dialect with
/// no merge-ledger substrate (Spark) skips the bookkeeping record but
/// still succeeds — this is bookkeeping, not a correctness gate. The
/// omission is no longer surfaced via the old `RunReporter` stand-in
/// method (retired — `docs/outcomes/20260904-state-residency/
/// outcome.md` phase 6): the affected cell's own recorded
/// `state_downgrade` is now the user-visible channel, surfaced by
/// `smelt explain` (`crates/smelt-cli/tests/explain_maintenance.rs`).
/// This test proves only the mechanical half the driver itself owns:
/// no `RunReporter` event fires and no ledger statement is issued.
#[tokio::test]
async fn keyed_ledger_skip_reports_no_reporter_event() {
    let backend = RecordingBackend {
        dialect: SqlDialect::SparkSQL,
        ..Default::default()
    };
    let retry = no_retry_policy();
    let steps = driving_steps("2024-01-01", "2024-01-02", &Granularity::Day).unwrap();
    run_windowed_keyed_maintenance(
        &backend,
        "model.under.test",
        "main",
        "t",
        &steps,
        &SumRule,
        None,
        &unconditional_suppression(),
        None,
        |step| {
            Ok(format!(
                "SELECT * FROM src WHERE d = '{}'",
                step.partition_value
            ))
        },
        &retry,
        &crate::probes::ProbePolicy::per_run(),
    )
    .await
    .expect("a skipped ledger record must not fail the run");

    let calls = backend.calls.lock().unwrap();
    assert!(
        !calls.iter().any(|c| c.contains("_smelt_ledger")),
        "no ledger statement must be issued on a ledger-less dialect: {:?}",
        calls
    );
}

/// The negative direction of the test above: on DuckDB (which has the
/// ledger substrate) the bookkeeping record is written.
#[tokio::test]
async fn idempotent_ledger_on_duckdb_writes_the_record() {
    let backend = RecordingBackend::default();
    let retry = no_retry_policy();
    let steps = driving_steps("2024-01-01", "2024-01-02", &Granularity::Day).unwrap();
    run_windowed_keyed_maintenance(
        &backend,
        "model.under.test",
        "main",
        "t",
        &steps,
        &SumRule,
        None,
        &unconditional_suppression(),
        None,
        |step| {
            Ok(format!(
                "SELECT * FROM src WHERE d = '{}'",
                step.partition_value
            ))
        },
        &retry,
        &crate::probes::ProbePolicy::per_run(),
    )
    .await
    .unwrap();

    let calls = backend.calls.lock().unwrap();
    assert!(
        calls.iter().any(|c| c.contains("_smelt_ledger")),
        "the ledger record must be written on DuckDB: {:?}",
        calls
    );
}

/// MP12: an `Additive`-graded rule routes every step's create-or-merge
/// action through `Backend::fold_ledger_delta` instead of the plain
/// `create_table_as`/`execute_sql` path — the never-fold-twice wiring
/// is reached even without a real database (`RecordingBackend` falls
/// back to `fold_ledger_delta`'s generic default, which itself calls
/// `execute_sql` for the ledger DDL/DML and the fold action).
#[tokio::test]
async fn additive_grade_routes_through_ledger_fold() {
    let backend = RecordingBackend::default();
    let steps = driving_steps("2024-01-01", "2024-01-02", &Granularity::Day).unwrap();
    run_windowed_keyed_maintenance(
        &backend,
        "model.under.test",
        "main",
        "t",
        &steps,
        &SumRuleAdditive,
        None,
        &unconditional_suppression(),
        None,
        |step| {
            Ok(format!(
                "SELECT * FROM src WHERE d = '{}'",
                step.partition_value
            ))
        },
        &no_retry_policy(),
        &crate::probes::ProbePolicy::per_run(),
    )
    .await
    .unwrap();

    let calls = backend.calls.lock().unwrap();
    // The default `fold_ledger_delta` fallback issues ensure + exists +
    // insert + action, all via `execute_sql` — never `create_table_as`,
    // since the ledger-guarded action string carries its own `CREATE
    // TABLE ... AS` text for the create branch.
    assert!(
        calls.iter().any(|c| c.contains("_smelt_ledger")),
        "the ledger table DDL/DML must be issued: {:?}",
        calls
    );
    assert!(
        calls.iter().any(|c| c.contains("CREATE TABLE main.t AS")),
        "the create branch's action must run through the ledger fold: {:?}",
        calls
    );
}

/// MP12: the ledger DDL/DML is DuckDB-flavored SQL
/// (`smelt_state::ddl_duckdb`). An `Additive`-graded rule on a non-DuckDB
/// backend must fail loudly instead of handing that backend SQL it
/// cannot run (`CLAUDE.md` §"Fail-loud discipline").
#[tokio::test]
async fn additive_grade_on_non_duckdb_backend_fails_loud() {
    let backend = RecordingBackend {
        dialect: SqlDialect::SparkSQL,
        ..Default::default()
    };
    let steps = driving_steps("2024-01-01", "2024-01-02", &Granularity::Day).unwrap();
    let err = run_windowed_keyed_maintenance(
        &backend,
        "model.under.test",
        "main",
        "t",
        &steps,
        &SumRuleAdditive,
        None,
        &unconditional_suppression(),
        None,
        |step| {
            Ok(format!(
                "SELECT * FROM src WHERE d = '{}'",
                step.partition_value
            ))
        },
        &no_retry_policy(),
        &crate::probes::ProbePolicy::per_run(),
    )
    .await
    .unwrap_err();

    assert!(
        backend.calls.lock().unwrap().is_empty(),
        "no SQL must be issued once the dialect guard refuses"
    );
    let message = format!("{err:#}");
    assert!(
        message.contains("Spark SQL"),
        "error must name the unsupported dialect: {message}"
    );
}

/// A rule that records the slice predicate it receives from the driver —
/// used to prove route 2 (key-determined) locality threads a
/// `LocalitySlice::DeltaValues` through to `merge_sql` as a
/// `TargetSlicePredicate::DeltaValues` over the step's *own* delta
/// relation, never a margin-based range
/// (`docs/specs/incremental_shapes.md` §"Key temporal locality", route
/// 2: "the slice is the delta's own partition values — exact
/// regardless of key age").
struct CapturingRule {
    captured: Mutex<Vec<Option<TargetSlicePredicate>>>,
}

impl WindowedKeyedRule for CapturingRule {
    fn refuse(&self) -> Option<String> {
        None
    }
    fn merge_sql(
        &self,
        schema: &str,
        table: &str,
        delta_sql: &str,
        slice: Option<&TargetSlicePredicate>,
        _suppression: &WriteSuppression,
        _dialect: MaintenanceDialect,
    ) -> String {
        self.captured.lock().unwrap().push(slice.cloned());
        format!("MERGE INTO {}.{} USING ({})", schema, table, delta_sql)
    }
}

#[tokio::test]
async fn route2_locality_threads_delta_values_slice_over_the_steps_own_delta() {
    let backend = RecordingBackend::default();
    // Three day-steps: the first creates the table (no `merge_sql` call
    // at all — the create branch owns that step); the remaining two
    // exercise `merge_sql` and are the ones this test inspects.
    let steps = driving_steps("2024-01-01", "2024-01-04", &Granularity::Day).unwrap();
    let rule = CapturingRule {
        captured: Mutex::new(Vec::new()),
    };
    let locality = LocalitySlice::DeltaValues {
        partition_column: "first_seen_at".to_string(),
    };
    run_windowed_keyed_maintenance(
        &backend,
        "model.under.test",
        "main",
        "t",
        &steps,
        &rule,
        Some(&locality),
        &unconditional_suppression(),
        None,
        |step| {
            Ok(format!(
                "SELECT id, first_seen_at FROM src WHERE d = '{}'",
                step.partition_value
            ))
        },
        &no_retry_policy(),
        &crate::probes::ProbePolicy::per_run(),
    )
    .await
    .unwrap();

    let captured = rule.captured.lock().unwrap();
    assert_eq!(
        captured.len(),
        2,
        "the two merge steps must each capture a slice: {:?}",
        captured
    );
    for (idx, slice) in captured.iter().enumerate() {
        match slice.as_ref().expect("route 2 must thread a slice") {
            TargetSlicePredicate::DeltaValues {
                partition_column,
                delta_select,
            } => {
                assert_eq!(partition_column, "first_seen_at");
                // The delta relation threaded through is exactly this
                // step's own compiled delta — never widened, never a
                // caller-precomputed range.
                let expected_date = if idx == 0 { "2024-01-02" } else { "2024-01-03" };
                assert!(
                    delta_select.contains(expected_date),
                    "step {idx} delta_select must be its own step's delta, got: \
                     {delta_select}"
                );
            }
            other => panic!(
                "step {idx}: route 2 must derive a DeltaValues predicate, not a \
                              Window (margin-based) one: {other:?}"
            ),
        }
    }
}

fn yes_cell(scan: ScanClamp) -> PlanCell {
    PlanCell {
        group: "{status}".to_string(),
        trigger: Trigger::UpstreamMutation {
            source: "dim".to_string(),
        },
        corner: smelt_logical::maintenance::Corner::ColumnMerge,
        technique: Technique::ColumnScopedMerge,
        partition_local: PartitionLocal::Yes,
        scans: vec![scan],
        ledger_catch_up: false,
        row_identity: smelt_logical::maintenance::RowIdentityVerdict {
            identity: smelt_logical::maintenance::RowIdentity::WholeRow,
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: std::collections::BTreeMap::new(),
        key_scope: None,
        state_downgrade: None,
    }
}

fn no_cell() -> PlanCell {
    PlanCell {
        group: "{status}".to_string(),
        trigger: Trigger::UpstreamMutation {
            source: "dim".to_string(),
        },
        corner: smelt_logical::maintenance::Corner::ColumnMerge,
        technique: Technique::ColumnScopedMerge,
        partition_local: PartitionLocal::No {
            source: "dim".to_string(),
            why: "unclocked".to_string(),
        },
        scans: vec![],
        ledger_catch_up: false,
        row_identity: smelt_logical::maintenance::RowIdentityVerdict {
            identity: smelt_logical::maintenance::RowIdentity::WholeRow,
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: std::collections::BTreeMap::new(),
        key_scope: None,
        state_downgrade: None,
    }
}

fn dim_scan() -> ScanClamp {
    ScanClamp {
        source: "dim".to_string(),
        column: "changed_at".to_string(),
        before: Seconds::ZERO,
        after: Seconds::hours(24),
        write_footprint: None,
    }
}

#[test]
fn decide_dispatch_full_for_partition_local_no() {
    let dispatch = decide_column_merge_dispatch(
        &no_cell(),
        "dim",
        true,
        true,
        &ContributionVerdict::Monotone,
    )
    .expect("PartitionLocal::No + table exists + unique_key must dispatch Full");
    assert_eq!(dispatch, ColumnMergeDispatch::Full);
}

#[test]
fn decide_dispatch_clamped_for_partition_local_yes_with_monotone_contribution() {
    let cell = yes_cell(dim_scan());
    let dispatch =
        decide_column_merge_dispatch(&cell, "dim", true, true, &ContributionVerdict::Monotone)
            .expect(
                "PartitionLocal::Yes + matching scan + monotone contribution must dispatch Clamped",
            );
    assert_eq!(dispatch, ColumnMergeDispatch::Clamped(dim_scan()));
}

#[test]
fn decide_dispatch_none_when_table_missing() {
    assert_eq!(
        decide_column_merge_dispatch(
            &no_cell(),
            "dim",
            false,
            true,
            &ContributionVerdict::Monotone
        ),
        None,
        "a missing target table must fall back to the safe default, never error"
    );
    assert_eq!(
        decide_column_merge_dispatch(
            &yes_cell(dim_scan()),
            "dim",
            false,
            true,
            &ContributionVerdict::Monotone
        ),
        None
    );
}

#[test]
fn decide_dispatch_none_when_unique_key_undeclared() {
    assert_eq!(
        decide_column_merge_dispatch(
            &no_cell(),
            "dim",
            true,
            false,
            &ContributionVerdict::Monotone
        ),
        None
    );
}

#[test]
fn decide_dispatch_none_when_contribution_not_monotone() {
    let cell = yes_cell(dim_scan());
    let refused = ContributionVerdict::Refused("join fans out".to_string());
    assert_eq!(
        decide_column_merge_dispatch(&cell, "dim", true, true, &refused),
        None,
        "a non-monotone contribution must never dispatch Clamped — the whole point of the \
         proof is to refuse a fanned-out join, not merge it approximately"
    );
}

#[test]
fn decide_dispatch_none_when_no_scan_matches_source() {
    // The plan's only scan is for a DIFFERENT source than the one the
    // caller resolved live — must never dispatch on a mismatched scan.
    let mut cell = yes_cell(dim_scan());
    cell.scans[0].source = "other_source".to_string();
    assert_eq!(
        decide_column_merge_dispatch(&cell, "dim", true, true, &ContributionVerdict::Monotone),
        None
    );
}

#[test]
fn widen_horizon_never_narrows_the_batch_window() {
    let scan = dim_scan(); // after = 24h
    let narrower_batch = Seconds::hours(6);
    let bound = widen_horizon_for_batch(&scan, narrower_batch);
    assert_eq!(
        bound,
        BoundResult::Bounded {
            source_partition_col: "changed_at".to_string(),
            before: Seconds::ZERO,
            after: Seconds::hours(24),
        },
        "a batch narrower than the derived scan margin must keep the derived margin"
    );

    let wider_batch = Seconds::days(3);
    let bound = widen_horizon_for_batch(&scan, wider_batch);
    assert_eq!(
        bound,
        BoundResult::Bounded {
            source_partition_col: "changed_at".to_string(),
            before: Seconds::ZERO,
            after: Seconds::days(3),
        },
        "a batch wider than the derived scan margin must widen to the batch width, never \
         silently drop the batch's earlier rows from the merge"
    );
}

#[test]
fn dimension_join_contribution_refuses_with_no_declared_unique_key() {
    let sql = "SELECT e.id, d.status FROM smelt.sources.events e \
                JOIN smelt.sources.dim d ON e.dim_id = d.id";
    let verdict = dimension_join_contribution(sql, "dim", &[]);
    assert!(
        !verdict.is_monotone(),
        "no declared unique_key must refuse, never optimistically assume monotone"
    );
}

#[test]
fn dimension_join_contribution_proves_monotone_via_declared_unique_key() {
    let sql = "SELECT e.id, d.status FROM smelt.sources.events e \
                JOIN smelt.sources.dim d ON e.dim_id = d.id";
    let verdict = dimension_join_contribution(sql, "dim", &["id".to_string()]);
    assert!(
        verdict.is_monotone(),
        "an equi-join on the dimension's declared unique_key must prove one-to-one: \
         {verdict:?}"
    );
}

#[test]
fn dimension_join_contribution_refuses_a_fan_out_join() {
    let sql = "SELECT e.id, d.status FROM smelt.sources.events e \
                JOIN smelt.sources.dim d ON e.dim_id = d.category";
    let verdict = dimension_join_contribution(sql, "dim", &["id".to_string()]);
    assert!(
        !verdict.is_monotone(),
        "the join equates on `category`, not the declared unique_key `id` — this must \
         refuse, never assume one-to-one: {verdict:?}"
    );
}
