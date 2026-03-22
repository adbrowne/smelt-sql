//! Tests for interval tracking: gap detection, coverage recording, and auto mode.
//!
//! These tests verify the state infrastructure that tracks which time ranges
//! have been processed, enabling gap detection and automatic incremental runs.

use super::*;
use smelt_state::file_store::FileStore;
use smelt_state::intervals::{compute_model_hash, IntervalStore, ModelIntervals};

#[test]
fn test_gap_detection_with_single_gap() {
    let mut mi = ModelIntervals::new("hash1".into());
    mi.record_interval("2024-12-25", "2024-12-27");
    mi.record_interval("2024-12-29", "2024-12-31");

    let gaps = mi.find_gaps("2024-12-25", "2024-12-31");
    assert_eq!(gaps.len(), 1);
    assert_eq!(
        gaps[0].start,
        chrono::NaiveDate::from_ymd_opt(2024, 12, 27).unwrap()
    );
    assert_eq!(
        gaps[0].end,
        chrono::NaiveDate::from_ymd_opt(2024, 12, 29).unwrap()
    );
}

#[test]
fn test_gap_detection_with_multiple_gaps() {
    let mut mi = ModelIntervals::new("hash1".into());
    mi.record_interval("2024-12-25", "2024-12-26");
    mi.record_interval("2024-12-27", "2024-12-28");
    mi.record_interval("2024-12-29", "2024-12-30");

    let gaps = mi.find_gaps("2024-12-25", "2024-12-31");
    assert_eq!(gaps.len(), 3); // 26-27, 28-29, 30-31
}

#[test]
fn test_no_gaps_when_fully_covered() {
    let mut mi = ModelIntervals::new("hash1".into());
    mi.record_interval("2024-12-25", "2024-12-31");

    let gaps = mi.find_gaps("2024-12-25", "2024-12-31");
    assert!(gaps.is_empty());
}

#[test]
fn test_interval_merge_adjacent() {
    let mut mi = ModelIntervals::new("hash1".into());
    mi.record_interval("2024-12-25", "2024-12-27");
    mi.record_interval("2024-12-27", "2024-12-29");

    assert_eq!(mi.covered_intervals.len(), 1);
    assert_eq!(mi.covered_intervals[0].start, "2024-12-25");
    assert_eq!(mi.covered_intervals[0].end, "2024-12-29");
}

#[test]
fn test_interval_merge_overlapping() {
    let mut mi = ModelIntervals::new("hash1".into());
    mi.record_interval("2024-12-25", "2024-12-28");
    mi.record_interval("2024-12-27", "2024-12-30");

    assert_eq!(mi.covered_intervals.len(), 1);
    assert_eq!(mi.covered_intervals[0].start, "2024-12-25");
    assert_eq!(mi.covered_intervals[0].end, "2024-12-30");
}

#[test]
fn test_model_hash_invalidation_clears_intervals() {
    let mut store = IntervalStore::default();
    {
        let intervals = store.get_or_create("model_a", "hash_v1");
        intervals.record_interval("2024-12-25", "2024-12-31");
    }
    assert_eq!(store.get("model_a").unwrap().covered_intervals.len(), 1);

    // New hash → all intervals cleared
    let intervals = store.get_or_create("model_a", "hash_v2");
    assert!(intervals.covered_intervals.is_empty());
}

#[test]
fn test_interval_store_persistence() {
    let dir = tempfile::TempDir::new().unwrap();
    let file_store = FileStore::new(dir.path());

    // Save
    let mut store = IntervalStore::default();
    let intervals = store.get_or_create("daily_revenue", "sha256:abc123");
    intervals.record_interval("2024-12-25", "2024-12-28");
    intervals.record_interval("2024-12-28", "2024-12-31");
    file_store.save_intervals(&store).unwrap();

    // Load
    let loaded = file_store.load_intervals().unwrap();
    let model = loaded.get("daily_revenue").unwrap();
    assert_eq!(model.covered_intervals.len(), 1); // merged into one
    assert_eq!(model.covered_intervals[0].start, "2024-12-25");
    assert_eq!(model.covered_intervals[0].end, "2024-12-31");
}

#[test]
fn test_compute_model_hash_deterministic() {
    let sql = "SELECT SUM(amount) FROM transactions GROUP BY date";
    let hash1 = compute_model_hash(sql);
    let hash2 = compute_model_hash(sql);
    assert_eq!(hash1, hash2);
}

#[test]
fn test_compute_model_hash_changes_with_sql() {
    let hash1 = compute_model_hash("SELECT SUM(amount) FROM transactions GROUP BY date");
    let hash2 = compute_model_hash("SELECT AVG(amount) FROM transactions GROUP BY date");
    assert_ne!(hash1, hash2);
}

#[tokio::test]
async fn test_incremental_run_updates_intervals() -> Result<()> {
    let (dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    let file_store = FileStore::new(dir.path());
    let model_sql = DAILY_REVENUE_SQL;
    let model_hash = compute_model_hash(model_sql);

    // Run incremental for Dec 25-27
    run_incremental_sequence(
        &backend,
        "inc_interval_test",
        &daily_revenue_filtered,
        &[TestTimeRange { start: "2024-12-25".into(), end: "2024-12-27".into() }],
        IncrementalStrategy::DeleteInsert,
        "revenue_date",
        &[],
    )
    .await?;

    // Record the interval
    let mut store = file_store.load_intervals()?;
    let intervals = store.get_or_create("inc_interval_test", &model_hash);
    intervals.record_interval("2024-12-25", "2024-12-27");
    file_store.save_intervals(&store)?;

    // Check gaps — Dec 27-30 should be a gap
    let loaded = file_store.load_intervals()?;
    let model = loaded.get("inc_interval_test").unwrap();
    let gaps = model.find_gaps("2024-12-25", "2024-12-30");
    assert_eq!(gaps.len(), 1);
    assert_eq!(
        gaps[0].start,
        chrono::NaiveDate::from_ymd_opt(2024, 12, 27).unwrap()
    );

    // Run incremental for Dec 27-30 and update intervals
    run_incremental_sequence(
        &backend,
        "inc_interval_test",
        &daily_revenue_filtered,
        &[TestTimeRange { start: "2024-12-27".into(), end: "2024-12-30".into() }],
        IncrementalStrategy::DeleteInsert,
        "revenue_date",
        &[],
    )
    .await?;

    let mut store = file_store.load_intervals()?;
    let intervals = store.get_or_create("inc_interval_test", &model_hash);
    intervals.record_interval("2024-12-27", "2024-12-30");
    file_store.save_intervals(&store)?;

    // No more gaps
    let loaded = file_store.load_intervals()?;
    let model = loaded.get("inc_interval_test").unwrap();
    let gaps = model.find_gaps("2024-12-25", "2024-12-30");
    assert!(gaps.is_empty());

    // Verify data correctness
    run_full_refresh(&backend, "baseline_interval", DAILY_REVENUE_SQL).await?;
    assert_tables_equal(&backend, "baseline_interval", "inc_interval_test").await?;

    Ok(())
}
