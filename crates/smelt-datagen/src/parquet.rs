//! Parquet writer with Hive-style partitioning.

use crate::session::{generate_day_seeds, DayGenerator, Session, VisitorPool};
use anyhow::{Context, Result};
use arrow::array::{ArrayRef, Int32Array, StringBuilder};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use chrono::NaiveDate;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use rayon::prelude::*;
use std::fs::{self, File};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Schema for session records (without session_date, which is the partition key).
fn session_schema() -> Schema {
    Schema::new(vec![
        Field::new("visitor_id", DataType::Utf8, false),
        Field::new("session_id", DataType::Utf8, false),
        Field::new("platform", DataType::Utf8, false),
        Field::new("visit_source", DataType::Utf8, false),
        Field::new("visit_campaign", DataType::Utf8, true),
        Field::new("widget_views", DataType::Int32, false),
        Field::new("product_views", DataType::Int32, false),
        Field::new("product_category", DataType::Utf8, false),
        Field::new("product_revenue", DataType::Int32, false),
        Field::new("product_purchase_count", DataType::Int32, false),
        Field::new("country", DataType::Utf8, false),
        Field::new("user_status", DataType::Utf8, false),
    ])
}

/// Write sessions for a single day to a Hive-partitioned Parquet file.
pub fn write_day_to_parquet(
    output_dir: &Path,
    date: NaiveDate,
    sessions: &[Session],
) -> Result<usize> {
    if sessions.is_empty() {
        return Ok(0);
    }

    // Create partition directory: output_dir/session_date=YYYY-MM-DD/
    let partition_dir = output_dir.join(format!("session_date={}", date));
    fs::create_dir_all(&partition_dir)
        .with_context(|| format!("Failed to create partition directory: {:?}", partition_dir))?;

    let file_path = partition_dir.join("data.parquet");
    let file = File::create(&file_path)
        .with_context(|| format!("Failed to create parquet file: {:?}", file_path))?;

    // Convert sessions to Arrow arrays
    let schema = Arc::new(session_schema());
    let batch = sessions_to_record_batch(sessions, &schema)?;

    // Write to Parquet
    let props = WriterProperties::builder()
        .set_compression(parquet::basic::Compression::SNAPPY)
        .build();

    let mut writer = ArrowWriter::try_new(file, schema, Some(props))
        .context("Failed to create Parquet writer")?;

    writer
        .write(&batch)
        .context("Failed to write record batch")?;
    writer.close().context("Failed to close Parquet writer")?;

    Ok(sessions.len())
}

fn sessions_to_record_batch(sessions: &[Session], schema: &Arc<Schema>) -> Result<RecordBatch> {
    let mut visitor_ids = StringBuilder::new();
    let mut session_ids = StringBuilder::new();
    let mut platforms = StringBuilder::new();
    let mut visit_sources = StringBuilder::new();
    let mut visit_campaigns = StringBuilder::new();
    let mut widget_views: Vec<i32> = Vec::with_capacity(sessions.len());
    let mut product_views: Vec<i32> = Vec::with_capacity(sessions.len());
    let mut product_categories = StringBuilder::new();
    let mut product_revenues: Vec<i32> = Vec::with_capacity(sessions.len());
    let mut product_purchase_counts: Vec<i32> = Vec::with_capacity(sessions.len());
    let mut countries = StringBuilder::new();
    let mut user_statuses = StringBuilder::new();

    for session in sessions {
        visitor_ids.append_value(session.visitor_id.to_string());
        session_ids.append_value(session.session_id.to_string());
        platforms.append_value(session.platform.as_str());
        visit_sources.append_value(session.visit_source.as_str());
        match &session.visit_campaign {
            Some(c) => visit_campaigns.append_value(c),
            None => visit_campaigns.append_null(),
        }
        widget_views.push(session.widget_views);
        product_views.push(session.product_views);
        product_categories.append_value(session.product_category.as_str());
        product_revenues.push(session.product_revenue);
        product_purchase_counts.push(session.product_purchase_count);
        countries.append_value(&session.country);
        user_statuses.append_value(session.user_status.as_str());
    }

    let columns: Vec<ArrayRef> = vec![
        Arc::new(visitor_ids.finish()),
        Arc::new(session_ids.finish()),
        Arc::new(platforms.finish()),
        Arc::new(visit_sources.finish()),
        Arc::new(visit_campaigns.finish()),
        Arc::new(Int32Array::from(widget_views)),
        Arc::new(Int32Array::from(product_views)),
        Arc::new(product_categories.finish()),
        Arc::new(Int32Array::from(product_revenues)),
        Arc::new(Int32Array::from(product_purchase_counts)),
        Arc::new(countries.finish()),
        Arc::new(user_statuses.finish()),
    ];

    RecordBatch::try_new(schema.clone(), columns).context("Failed to create record batch")
}

/// Write sessions to Hive-partitioned Parquet files with parallel generation.
pub fn write_sessions_to_parquet(
    output_dir: &Path,
    seed: u64,
    num_sessions: usize,
    num_days: u32,
    start_date: NaiveDate,
    progress_callback: Option<&(dyn Fn(usize, usize) + Sync)>,
) -> Result<usize> {
    // Create output directory
    fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create output directory: {:?}", output_dir))?;

    // Step 1: Generate shared visitor pool (deterministic from seed)
    let visitor_pool = VisitorPool::new(seed, num_sessions);

    // Step 2: Pre-compute per-day seeds (deterministic from seed)
    let day_seeds = generate_day_seeds(seed, num_days);

    // Step 3: Calculate sessions per day
    let sessions_per_day = num_sessions / num_days as usize;

    // Step 4: Build list of (date, seed) pairs
    let days: Vec<_> = (0..num_days)
        .map(|i| {
            let date = start_date + chrono::Duration::days(i as i64);
            (date, day_seeds[i as usize])
        })
        .collect();

    // Step 5: Parallel generation and writing
    let total_written = AtomicUsize::new(0);

    days.par_iter()
        .try_for_each(|(date, day_seed)| -> Result<()> {
            // Generate sessions for this day
            let generator =
                DayGenerator::new(visitor_pool.clone(), *day_seed, *date, sessions_per_day);
            let sessions = generator.generate();

            // Write to parquet
            let count = write_day_to_parquet(output_dir, *date, &sessions)?;

            // Update progress
            let new_total = total_written.fetch_add(count, Ordering::SeqCst) + count;
            if let Some(cb) = progress_callback {
                cb(new_total, num_sessions);
            }

            Ok(())
        })?;

    Ok(total_written.load(Ordering::SeqCst))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_write_parquet_creates_partitions() {
        let temp_dir = TempDir::new().unwrap();
        let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

        let count =
            write_sessions_to_parquet(temp_dir.path(), 42, 1000, 5, start_date, None).unwrap();

        assert!(count > 0);

        // Verify partition directories exist
        for i in 0..5 {
            let date = start_date + chrono::Duration::days(i);
            let partition_dir = temp_dir.path().join(format!("session_date={}", date));
            assert!(
                partition_dir.exists(),
                "Partition {:?} should exist",
                partition_dir
            );
            assert!(partition_dir.join("data.parquet").exists());
        }
    }

    #[test]
    fn test_deterministic_parallel_output() {
        let temp_dir1 = TempDir::new().unwrap();
        let temp_dir2 = TempDir::new().unwrap();
        let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

        // Run twice with same seed
        write_sessions_to_parquet(temp_dir1.path(), 42, 1000, 5, start_date, None).unwrap();
        write_sessions_to_parquet(temp_dir2.path(), 42, 1000, 5, start_date, None).unwrap();

        // Compare file contents for each partition
        for i in 0..5 {
            let date = start_date + chrono::Duration::days(i);
            let file1 = temp_dir1
                .path()
                .join(format!("session_date={}", date))
                .join("data.parquet");
            let file2 = temp_dir2
                .path()
                .join(format!("session_date={}", date))
                .join("data.parquet");

            // Read and compare
            let bytes1 = std::fs::read(&file1).unwrap();
            let bytes2 = std::fs::read(&file2).unwrap();
            assert_eq!(bytes1, bytes2, "Files for {} should be identical", date);
        }
    }
}
