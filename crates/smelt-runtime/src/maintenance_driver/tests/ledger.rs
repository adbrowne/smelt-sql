//! Ledger-family tests for the windowed-keyed-maintenance driver: the
//! re-run-tolerant merge-ledger record, and the additive fold's
//! never-fold-twice refusal (`docs/specs/incremental_models.md` §Constraints
//! "Never fold a delta already reflected in the state").
//!
//! Split out of the driver's main unit-test module when that file outgrew its
//! line budget. It shares that module's fakes (`RecordingBackend`, `SumRule`, the no-retry policy)
//! rather than growing a second set, so a change to the driver seam shows up
//! in both files at once.

use super::{no_retry_policy, unconditional_suppression, RecordingBackend, SumRule};
use crate::maintenance_driver::*;
use smelt_backend::Backend;
use smelt_core::config::Granularity;
use smelt_dialect::SqlDialect;
use smelt_logical::maintenance::choice::WriteSuppression;
use smelt_logical::maintenance::emit::{MaintenanceDialect, TargetSlicePredicate};
use smelt_state::reconciliation::Grade;
use std::sync::Mutex;

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

/// MP12: a dialect with no never-fold-twice realisation
/// (`realises_reconciliation_ledger`, derived from the availability layer)
/// must fail loudly rather than being handed ledger SQL that cannot refuse a
/// repeat there (`CLAUDE.md` §"Fail-loud discipline"). Spark is the permanent
/// case: Delta has no cross-table transaction, so the refusal has no sound
/// realisation at all.
///
/// Renamed from `additive_grade_on_non_duckdb_backend_fails_loud`: "non-DuckDB"
/// stopped being the condition once BigQuery grew a realisation of its own.
#[tokio::test]
async fn additive_grade_on_a_dialect_with_no_refusal_fails_loud() {
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

/// BigQuery now realises the reconciliation ledger, so the dialect guard above
/// must **not** fire for it — but the first step's action is a
/// `CREATE TABLE … AS`, and BigQuery cannot hold permanent-entity DDL inside
/// the transaction that carries the ledger record. That is a *narrower*
/// condition than "the wrong dialect", spelled as the capability it actually
/// is, and it refuses before any SQL is issued with the `--full-refresh`
/// remedy named.
#[tokio::test]
async fn additive_grade_refuses_a_first_run_ddl_action_where_transactions_cannot_hold_ddl() {
    let backend = RecordingBackend {
        dialect: SqlDialect::BigQuery,
        ..Default::default()
    };
    assert!(
        !backend.capabilities().supports_transactional_ddl,
        "this test's premise is BigQuery's inability to hold DDL in a transaction"
    );
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

    let message = format!("{err:#}");
    assert!(
        message.contains("CREATE TABLE"),
        "the refusal must name the construct it cannot fold: {message}"
    );
    assert!(
        message.contains("--full-refresh"),
        "the refusal must name the remedy: {message}"
    );
    assert!(
        !message.contains("never-fold-twice ledger (never"),
        "this must NOT be the dialect guard's message: {message}"
    );
    assert!(
        backend.calls.lock().unwrap().is_empty(),
        "no SQL must be issued once the refusal fires: {:?}",
        backend.calls.lock().unwrap()
    );
}

/// …and with the target already materialised, every step's action is a merge,
/// so the same BigQuery cell folds through `Backend::fold_ledger_delta`
/// normally. Without this, the test above would be satisfied by BigQuery
/// refusing every additive fold — a false realisation of the row.
#[tokio::test]
async fn additive_grade_on_bigquery_folds_once_the_target_exists() {
    let backend = RecordingBackend {
        dialect: SqlDialect::BigQuery,
        table_exists: Mutex::new(true),
        ..Default::default()
    };
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
    .expect("BigQuery realises the reconciliation ledger");

    let calls = backend.calls.lock().unwrap();
    assert!(
        calls.iter().any(|c| c.contains("`main._smelt_ledger`")),
        "the ledger statements must be GoogleSQL-spelled: {:?}",
        calls
    );
    assert!(
        calls
            .iter()
            .any(|c| c.contains("WHEN NOT MATCHED THEN INSERT")),
        "BigQuery's fold record is the conditional MERGE, not a plain INSERT: {:?}",
        calls
    );
}
