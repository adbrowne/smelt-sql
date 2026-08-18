#![cfg(feature = "duckdb")]
//! Cross-partition equivalence harness for cumulative aggregate models.
//!
//! Asserts the formal contract from `docs/specs/cumulative_aggregate.md`
//! §"Cross-partition equivalence":
//!
//!   For any set of source partitions S = {D₁, …, Dₙ} and any ordering π over S:
//!     cumulative_aggregate_run(model, π(S))
//!       == full_refresh(model, source.where(partition ∈ S))
//!
//! In particular, processing partitions in **forward**, **reverse**, and
//! **shuffled** order all converge to the same final state.
//!
//! The harness builds a tiny synthetic source table directly in DuckDB and
//! drives `build_cumulative_merge_sql` (the rule's emitted SQL) over each
//! partition. This exercises the load-bearing property — commutative-
//! associative combine — without standing up the full CLI orchestration.
//! Each aggregator family from the v1 allowlist is exercised in turn.

use arrow::array::{BooleanArray, Int32Array, Int64Array};
use arrow::record_batch::RecordBatch;
use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Granularity, TimeseriesConfig};
use smelt_logical::maintenance::choice::WriteSuppression;
use smelt_logical::maintenance::emit::MaintenanceDialect;
use smelt_planner::{
    AggregatorColumn, CrossPartitionCombiner, CumulativeClassification, DrivingSource,
};
use smelt_runtime::build_cumulative_merge_sql;
use tempfile::TempDir;

/// This harness asserts fold-order equivalence, independent of
/// write-suppression admission — every call site below passes the plain
/// unconditional matched arm.
fn unconditional() -> WriteSuppression {
    WriteSuppression::Unconditional {
        why: "test asserts fold-order equivalence, not suppression".to_string(),
    }
}

/// One row in the synthetic source table.
#[derive(Clone, Copy, Debug)]
struct Row {
    event_date: &'static str,
    device_id: i32,
    user_id: i32,
    flag: bool,
    score: i32,
}

const FIXTURE: &[Row] = &[
    // Partition 2026-01-01
    Row {
        event_date: "2026-01-01",
        device_id: 1,
        user_id: 100,
        flag: true,
        score: 5,
    },
    Row {
        event_date: "2026-01-01",
        device_id: 1,
        user_id: 100,
        flag: true,
        score: 3,
    },
    Row {
        event_date: "2026-01-01",
        device_id: 2,
        user_id: 200,
        flag: false,
        score: 8,
    },
    // Partition 2026-01-02
    Row {
        event_date: "2026-01-02",
        device_id: 1,
        user_id: 100,
        flag: false,
        score: 7,
    },
    Row {
        event_date: "2026-01-02",
        device_id: 2,
        user_id: 200,
        flag: true,
        score: 1,
    },
    Row {
        event_date: "2026-01-02",
        device_id: 3,
        user_id: 300,
        flag: true,
        score: 9,
    },
    // Partition 2026-01-03
    Row {
        event_date: "2026-01-03",
        device_id: 1,
        user_id: 100,
        flag: true,
        score: 2,
    },
    Row {
        event_date: "2026-01-03",
        device_id: 2,
        user_id: 200,
        flag: true,
        score: 6,
    },
    Row {
        event_date: "2026-01-03",
        device_id: 3,
        user_id: 300,
        flag: false,
        score: 4,
    },
];

const PARTITIONS: &[&str] = &["2026-01-01", "2026-01-02", "2026-01-03"];

async fn setup_source(backend: &DuckDbBackend) {
    backend
        .execute_sql(
            "CREATE TABLE main.events (event_date DATE, device_id INT, user_id INT, \
             flag BOOLEAN, score INT)",
        )
        .await
        .expect("create events");
    let values: Vec<String> = FIXTURE
        .iter()
        .map(|r| {
            format!(
                "(DATE '{}', {}, {}, {}, {})",
                r.event_date, r.device_id, r.user_id, r.flag, r.score
            )
        })
        .collect();
    backend
        .execute_sql(&format!(
            "INSERT INTO main.events VALUES {}",
            values.join(", ")
        ))
        .await
        .expect("insert fixture rows");
}

fn classification(aggregator_columns: Vec<AggregatorColumn>) -> CumulativeClassification {
    CumulativeClassification {
        unique_key: vec!["device_id".to_string(), "user_id".to_string()],
        aggregator_columns,
        driving_source: DrivingSource {
            name: "smelt.main.events".to_string(),
            timeseries: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
        },
    }
}

/// Cumulative state we compare across orderings.
/// (device_id, user_id, event_count, any_flag, max_score, min_score)
#[derive(Debug, Clone, PartialEq, Eq)]
struct StateRow {
    device_id: i32,
    user_id: i32,
    event_count: i64,
    any_flag: bool,
    max_score: i32,
    min_score: i32,
}

/// SELECT template that produces the per-partition delta. `{partition}` is
/// substituted with the partition value.
const DELTA_TEMPLATE: &str = "SELECT device_id, user_id, \
    COUNT(*) AS event_count, \
    BOOL_OR(flag) AS any_flag, \
    MAX(score) AS max_score, \
    MIN(score) AS min_score \
    FROM main.events \
    WHERE event_date = DATE '{partition}' \
    GROUP BY device_id, user_id";

fn full_classification() -> CumulativeClassification {
    classification(vec![
        AggregatorColumn {
            output_name: "event_count".to_string(),
            per_partition_agg: "COUNT".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Sum,
            state: None,
        },
        AggregatorColumn {
            output_name: "any_flag".to_string(),
            per_partition_agg: "BOOL_OR".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::BoolOr,
            state: None,
        },
        AggregatorColumn {
            output_name: "max_score".to_string(),
            per_partition_agg: "MAX".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Max,
            state: None,
        },
        AggregatorColumn {
            output_name: "min_score".to_string(),
            per_partition_agg: "MIN".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Min,
            state: None,
        },
    ])
}

/// Run the cumulative loop for the given partition order, returning the final
/// state of `main.{table}` as a sorted Vec of StateRows.
async fn run_cumulative_in_order(
    table: &str,
    classification: &CumulativeClassification,
    order: &[&str],
) -> Vec<StateRow> {
    let temp_dir = TempDir::new().expect("tempdir");
    let db_path = temp_dir.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main").await.expect("backend");
    setup_source(&backend).await;

    for (idx, partition) in order.iter().enumerate() {
        let delta_sql = DELTA_TEMPLATE.replace("{partition}", partition);
        if idx == 0 {
            backend
                .execute_sql(&format!("CREATE TABLE main.{} AS {}", table, delta_sql))
                .await
                .unwrap_or_else(|e| panic!("create_table_as on {}: {}", partition, e));
        } else {
            let merge_sql = build_cumulative_merge_sql(
                "main",
                table,
                &delta_sql,
                classification,
                None,
                &unconditional(),
                MaintenanceDialect::DuckDb,
            );
            backend
                .execute_sql(&merge_sql)
                .await
                .unwrap_or_else(|e| panic!("merge on {}: {}\nSQL: {}", partition, e, merge_sql));
        }
    }

    let result = backend
        .execute_sql(&format!(
            "SELECT device_id, user_id, event_count, any_flag, max_score, min_score \
             FROM main.{} ORDER BY device_id, user_id",
            table
        ))
        .await
        .unwrap_or_else(|e| panic!("select final state: {}", e));

    extract_rows(&result)
}

/// Run a single full-refresh SELECT over all partitions, returning the same
/// shape as `run_cumulative_in_order`.
async fn run_full_refresh() -> Vec<StateRow> {
    let temp_dir = TempDir::new().expect("tempdir");
    let db_path = temp_dir.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main").await.expect("backend");
    setup_source(&backend).await;

    let dates = PARTITIONS
        .iter()
        .map(|d| format!("DATE '{}'", d))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT device_id, user_id, \
                COUNT(*) AS event_count, \
                BOOL_OR(flag) AS any_flag, \
                MAX(score) AS max_score, \
                MIN(score) AS min_score \
         FROM main.events \
         WHERE event_date IN ({}) \
         GROUP BY device_id, user_id \
         ORDER BY device_id, user_id",
        dates
    );
    let result = backend.execute_sql(&sql).await.expect("full refresh");
    extract_rows(&result)
}

fn extract_rows(batches: &[RecordBatch]) -> Vec<StateRow> {
    let mut rows = Vec::new();
    for batch in batches {
        let device_id = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .expect("device_id Int32");
        let user_id = batch
            .column(1)
            .as_any()
            .downcast_ref::<Int32Array>()
            .expect("user_id Int32");
        let event_count = batch
            .column(2)
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("event_count Int64");
        let any_flag = batch
            .column(3)
            .as_any()
            .downcast_ref::<BooleanArray>()
            .expect("any_flag Bool");
        let max_score = batch
            .column(4)
            .as_any()
            .downcast_ref::<Int32Array>()
            .expect("max_score Int32");
        let min_score = batch
            .column(5)
            .as_any()
            .downcast_ref::<Int32Array>()
            .expect("min_score Int32");
        for i in 0..batch.num_rows() {
            rows.push(StateRow {
                device_id: device_id.value(i),
                user_id: user_id.value(i),
                event_count: event_count.value(i),
                any_flag: any_flag.value(i),
                max_score: max_score.value(i),
                min_score: min_score.value(i),
            });
        }
    }
    rows
}

#[tokio::test]
async fn forward_ordering_matches_full_refresh() {
    let cls = full_classification();
    let actual = run_cumulative_in_order("forward", &cls, PARTITIONS).await;
    let expected = run_full_refresh().await;
    assert_eq!(
        actual, expected,
        "Forward partition order must produce the same cumulative state as a full refresh"
    );
}

#[tokio::test]
async fn reverse_ordering_matches_full_refresh() {
    let cls = full_classification();
    let mut order: Vec<&str> = PARTITIONS.to_vec();
    order.reverse();
    let actual = run_cumulative_in_order("reverse", &cls, &order).await;
    let expected = run_full_refresh().await;
    assert_eq!(
        actual, expected,
        "Reverse partition order must produce the same cumulative state as a full refresh \
         — the cross-partition equivalence contract is order-independent"
    );
}

#[tokio::test]
async fn shuffled_ordering_matches_full_refresh() {
    let cls = full_classification();
    let order = ["2026-01-02", "2026-01-01", "2026-01-03"];
    let actual = run_cumulative_in_order("shuffled", &cls, &order).await;
    let expected = run_full_refresh().await;
    assert_eq!(
        actual, expected,
        "Shuffled partition order must produce the same cumulative state as a full refresh"
    );
}

#[tokio::test]
async fn forward_and_reverse_orderings_agree() {
    let cls = full_classification();
    let forward = run_cumulative_in_order("fwd", &cls, PARTITIONS).await;
    let mut reversed: Vec<&str> = PARTITIONS.to_vec();
    reversed.reverse();
    let reverse = run_cumulative_in_order("rev", &cls, &reversed).await;
    assert_eq!(
        forward, reverse,
        "Forward and reverse partition orders must converge to identical state"
    );
}

#[tokio::test]
async fn two_independent_forward_runs_are_deterministic() {
    let cls = full_classification();
    let once = run_cumulative_in_order("once", &cls, PARTITIONS).await;
    let twice = run_cumulative_in_order("twice", &cls, PARTITIONS).await;
    assert_eq!(
        once, twice,
        "Two independent forward runs over the same input must produce identical state"
    );
}

#[tokio::test]
async fn bool_and_combiner_round_trips() {
    // BOOL_AND isn't exercised by the main full_classification (which uses
    // BOOL_OR via the `any_flag` column). Cover it in a focused fixture to
    // verify the AND combiner.
    let temp_dir = TempDir::new().expect("tempdir");
    let db_path = temp_dir.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main").await.expect("backend");
    setup_source(&backend).await;

    let cls = classification(vec![AggregatorColumn {
        output_name: "all_flag".to_string(),
        per_partition_agg: "BOOL_AND".to_string(),
        cross_partition_combiner: CrossPartitionCombiner::BoolAnd,
        state: None,
    }]);

    let delta_p1 = "SELECT device_id, user_id, BOOL_AND(flag) AS all_flag \
                    FROM main.events WHERE event_date = DATE '2026-01-01' \
                    GROUP BY device_id, user_id";
    backend
        .execute_sql(&format!("CREATE TABLE main.bool_and AS {}", delta_p1))
        .await
        .unwrap();

    let delta_p2 = "SELECT device_id, user_id, BOOL_AND(flag) AS all_flag \
                    FROM main.events WHERE event_date = DATE '2026-01-02' \
                    GROUP BY device_id, user_id";
    let merge_sql = build_cumulative_merge_sql(
        "main",
        "bool_and",
        delta_p2,
        &cls,
        None,
        &unconditional(),
        MaintenanceDialect::DuckDb,
    );
    backend.execute_sql(&merge_sql).await.unwrap();

    // For device 1, user 100:
    //   Partition 1: flags [true, true] -> BOOL_AND = true
    //   Partition 2: flags [false]      -> BOOL_AND = false
    //   Combined:    BOOL_AND(true, false) = false
    let result = backend
        .execute_sql(
            "SELECT all_flag FROM main.bool_and \
             WHERE device_id = 1 AND user_id = 100",
        )
        .await
        .unwrap();
    let all_flag = result[0]
        .column(0)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .expect("all_flag Bool")
        .value(0);
    assert!(
        !all_flag,
        "BOOL_AND combiner: true AND false should be false"
    );
}

#[tokio::test]
async fn bitwise_combiners_round_trip() {
    // The numeric bitwise combiners (BIT_AND / BIT_OR / BIT_XOR) aren't
    // exercised by the main full_classification. Build a tiny focused
    // fixture that combines two partitions and verifies each.
    let temp_dir = TempDir::new().expect("tempdir");
    let db_path = temp_dir.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main").await.expect("backend");
    backend
        .execute_sql("CREATE TABLE main.bits (event_date DATE, key INT, bits INT)")
        .await
        .unwrap();
    backend
        .execute_sql(
            "INSERT INTO main.bits VALUES \
             (DATE '2026-01-01', 1, 12), \
             (DATE '2026-01-02', 1, 10)",
        )
        .await
        .unwrap();

    let cls = CumulativeClassification {
        unique_key: vec!["key".to_string()],
        aggregator_columns: vec![
            AggregatorColumn {
                output_name: "all_bits".to_string(),
                per_partition_agg: "BIT_AND".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::BitAnd,
                state: None,
            },
            AggregatorColumn {
                output_name: "any_bits".to_string(),
                per_partition_agg: "BIT_OR".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::BitOr,
                state: None,
            },
            AggregatorColumn {
                output_name: "xor_bits".to_string(),
                per_partition_agg: "BIT_XOR".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::BitXor,
                state: None,
            },
        ],
        driving_source: DrivingSource {
            name: "smelt.main.bits".to_string(),
            timeseries: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
        },
    };

    let delta_p1 = "SELECT key, BIT_AND(bits) AS all_bits, BIT_OR(bits) AS any_bits, \
                           BIT_XOR(bits) AS xor_bits \
                    FROM main.bits WHERE event_date = DATE '2026-01-01' GROUP BY key";
    backend
        .execute_sql(&format!("CREATE TABLE main.bits_agg AS {}", delta_p1))
        .await
        .unwrap();

    let delta_p2 = "SELECT key, BIT_AND(bits) AS all_bits, BIT_OR(bits) AS any_bits, \
                           BIT_XOR(bits) AS xor_bits \
                    FROM main.bits WHERE event_date = DATE '2026-01-02' GROUP BY key";
    let merge_sql = build_cumulative_merge_sql(
        "main",
        "bits_agg",
        delta_p2,
        &cls,
        None,
        &unconditional(),
        MaintenanceDialect::DuckDb,
    );
    backend.execute_sql(&merge_sql).await.unwrap();

    // For key 1:
    //   Partition 1: BIT_AND/OR/XOR of [12]      = 12, 12, 12
    //   Partition 2: BIT_AND/OR/XOR of [10]      = 10, 10, 10
    //   Combined:   AND = 12 & 10 = 8;  OR = 12 | 10 = 14;  XOR = 12 ^ 10 = 6
    let result = backend
        .execute_sql("SELECT all_bits, any_bits, xor_bits FROM main.bits_agg WHERE key = 1")
        .await
        .unwrap();
    let all_bits = result[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int32Array>()
        .expect("all_bits Int32")
        .value(0);
    let any_bits = result[0]
        .column(1)
        .as_any()
        .downcast_ref::<Int32Array>()
        .expect("any_bits Int32")
        .value(0);
    let xor_bits = result[0]
        .column(2)
        .as_any()
        .downcast_ref::<Int32Array>()
        .expect("xor_bits Int32")
        .value(0);
    assert_eq!(all_bits, 8, "BIT_AND: 12 & 10 = 8");
    assert_eq!(any_bits, 14, "BIT_OR: 12 | 10 = 14");
    assert_eq!(xor_bits, 6, "BIT_XOR: 12 ^ 10 = 6");
}
