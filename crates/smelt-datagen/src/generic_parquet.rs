//! Dynamic Parquet writer for YAML-configured datasets.

use crate::config::DatasetConfig;
use crate::generic::{generate_row, make_entity_pool, GenericValue};
use anyhow::{Context, Result};
use arrow::array::{ArrayRef, BooleanBuilder, Float64Builder, Int32Builder, StringBuilder};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use chrono::NaiveDate;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use rand::RngCore;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use std::fs::{self, File};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const BATCH_SIZE: usize = 64 * 1024;

/// Write a dataset configured by `config` to Parquet.
///
/// Returns the total number of rows written.
pub fn write_generic_dataset(
    config: &DatasetConfig,
    global_seed: u64,
    progress: Option<&(dyn Fn(usize, usize) + Sync)>,
) -> Result<usize> {
    let seed = config.seed.unwrap_or(global_seed);
    let output = Path::new(&config.output);
    fs::create_dir_all(output)
        .with_context(|| format!("Failed to create output directory: {:?}", output))?;

    let schema = Arc::new(build_schema(config));

    if let Some(part_cfg) = &config.partition {
        write_partitioned(config, seed, output, schema, part_cfg, progress)
    } else {
        write_single(config, seed, output, schema, progress)
    }
}

// ---------------------------------------------------------------------------
// Schema construction
// ---------------------------------------------------------------------------

fn build_schema(config: &DatasetConfig) -> Schema {
    let mut fields: Vec<Field> = Vec::new();

    // Entity columns first
    if let Some(entity) = &config.entity {
        for col in &entity.columns {
            let dt = col.generator.arrow_type();
            // Entity columns are not nullable (they come from a pre-built pool)
            fields.push(Field::new(&col.name, dt, false));
        }
    }

    // Regular columns
    for col in &config.columns {
        let dt = col.generator.arrow_type();
        let nullable = col.generator.is_nullable();
        fields.push(Field::new(&col.name, dt, nullable));
    }

    // Partition column last (always Utf8, not nullable)
    if let Some(part) = &config.partition {
        fields.push(Field::new(&part.column, DataType::Utf8, false));
    }

    Schema::new(fields)
}

// ---------------------------------------------------------------------------
// Partitioned write (one file per day, parallel)
// ---------------------------------------------------------------------------

fn write_partitioned(
    config: &DatasetConfig,
    seed: u64,
    output: &Path,
    schema: Arc<Schema>,
    part_cfg: &crate::config::PartitionConfig,
    progress: Option<&(dyn Fn(usize, usize) + Sync)>,
) -> Result<usize> {
    let start_date = NaiveDate::parse_from_str(&part_cfg.start, "%Y-%m-%d")
        .with_context(|| format!("Invalid partition start date: {}", part_cfg.start))?;

    let days = part_cfg.days;
    let rows_per_day = config.num_rows / days as usize;
    let total_rows = config.num_rows;

    // Generate per-day seeds deterministically
    let day_seeds: Vec<u64> = {
        let mut rng = ChaCha8Rng::seed_from_u64(seed.wrapping_add(1));
        (0..days).map(|_| rng.next_u64()).collect()
    };

    // Build entity pool seed (offset from day seeds)
    let entity_seed = seed.wrapping_add(2);

    // Build entity pool (shared across days, cloned cheaply via Arc inside)
    let entity_pool_arc: Option<Arc<_>> = config
        .entity
        .as_ref()
        .map(|e| Arc::new(make_entity_pool(entity_seed, config.num_rows, e)));

    let entity_col_specs: Vec<_> = config
        .entity
        .as_ref()
        .map(|e| e.columns.as_slice())
        .unwrap_or(&[])
        .to_vec();

    let total_written = AtomicUsize::new(0);

    let days_vec: Vec<_> = (0..days)
        .map(|i| {
            let date = start_date + chrono::Duration::days(i as i64);
            (date, day_seeds[i as usize])
        })
        .collect();

    days_vec
        .par_iter()
        .try_for_each(|(date, day_seed)| -> Result<()> {
            let date_str = date.to_string();
            let partition_dir = output.join(format!("{}={}", part_cfg.column, date_str));
            fs::create_dir_all(&partition_dir)
                .with_context(|| format!("Failed to create partition dir: {:?}", partition_dir))?;

            let file_path = partition_dir.join("data.parquet");
            let file = File::create(&file_path)
                .with_context(|| format!("Failed to create file: {:?}", file_path))?;

            let entity_pool = entity_pool_arc.as_deref();

            let count = write_rows_to_file(
                file,
                schema.clone(),
                *day_seed,
                rows_per_day,
                &entity_col_specs,
                entity_pool,
                &config.columns,
                Some((part_cfg.column.as_str(), date_str.as_str())),
            )?;

            let new_total = total_written.fetch_add(count, Ordering::SeqCst) + count;
            if let Some(cb) = progress {
                cb(new_total, total_rows);
            }

            Ok(())
        })?;

    Ok(total_written.load(Ordering::SeqCst))
}

// ---------------------------------------------------------------------------
// Single-file write (no partition)
// ---------------------------------------------------------------------------

fn write_single(
    config: &DatasetConfig,
    seed: u64,
    output: &Path,
    schema: Arc<Schema>,
    progress: Option<&(dyn Fn(usize, usize) + Sync)>,
) -> Result<usize> {
    let entity_pool = config
        .entity
        .as_ref()
        .map(|e| make_entity_pool(seed.wrapping_add(1), config.num_rows, e));

    let entity_col_specs: Vec<_> = config
        .entity
        .as_ref()
        .map(|e| e.columns.as_slice())
        .unwrap_or(&[])
        .to_vec();

    let file_path = output.join("data.parquet");
    let file = File::create(&file_path)
        .with_context(|| format!("Failed to create file: {:?}", file_path))?;

    let count = write_rows_to_file(
        file,
        schema,
        seed,
        config.num_rows,
        &entity_col_specs,
        entity_pool.as_ref(),
        &config.columns,
        None,
    )?;

    if let Some(cb) = progress {
        cb(count, count);
    }

    Ok(count)
}

// ---------------------------------------------------------------------------
// Core row writing loop
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn write_rows_to_file(
    file: File,
    schema: Arc<Schema>,
    seed: u64,
    num_rows: usize,
    entity_col_specs: &[crate::config::ColumnConfig],
    entity_pool: Option<&crate::generic::EntityPool>,
    col_specs: &[crate::config::ColumnConfig],
    partition_col: Option<(&str, &str)>,
) -> Result<usize> {
    let props = WriterProperties::builder()
        .set_compression(parquet::basic::Compression::SNAPPY)
        .build();

    let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(props))
        .context("Failed to create Parquet writer")?;

    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut written = 0;

    while written < num_rows {
        let batch_size = BATCH_SIZE.min(num_rows - written);

        // Collect rows for this batch
        let rows: Vec<Vec<(String, GenericValue)>> = (0..batch_size)
            .map(|_| {
                // Sample an entity row if a pool exists
                let entity_row = entity_pool.map(|pool| {
                    let idx = (rng.next_u64() as usize) % pool.len();
                    pool.rows[idx].as_slice()
                });
                generate_row(
                    &mut rng,
                    entity_col_specs,
                    entity_row,
                    col_specs,
                    partition_col,
                )
            })
            .collect();

        let batch = rows_to_record_batch(&rows, &schema)?;
        writer.write(&batch).context("Failed to write batch")?;
        written += batch_size;
    }

    writer.close().context("Failed to close Parquet writer")?;
    Ok(written)
}

// ---------------------------------------------------------------------------
// Convert rows → RecordBatch
// ---------------------------------------------------------------------------

fn rows_to_record_batch(
    rows: &[Vec<(String, GenericValue)>],
    schema: &Arc<Schema>,
) -> Result<RecordBatch> {
    if rows.is_empty() {
        return Ok(RecordBatch::new_empty(schema.clone()));
    }

    let num_cols = rows[0].len();
    let mut columns: Vec<ArrayRef> = Vec::with_capacity(num_cols);

    for col_idx in 0..num_cols {
        let field = &schema.field(col_idx);
        let array = build_column(rows, col_idx, field.data_type(), field.is_nullable())?;
        columns.push(array);
    }

    RecordBatch::try_new(schema.clone(), columns).context("Failed to create RecordBatch")
}

fn build_column(
    rows: &[Vec<(String, GenericValue)>],
    col_idx: usize,
    data_type: &DataType,
    nullable: bool,
) -> Result<ArrayRef> {
    match data_type {
        DataType::Utf8 => {
            let mut builder = StringBuilder::new();
            for row in rows {
                match &row[col_idx].1 {
                    GenericValue::Str(s) => builder.append_value(s),
                    GenericValue::Null if nullable => builder.append_null(),
                    GenericValue::Int(i) => builder.append_value(i.to_string()),
                    other => builder.append_value(format!("{:?}", other)),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::Int32 => {
            let mut builder = Int32Builder::new();
            for row in rows {
                match &row[col_idx].1 {
                    GenericValue::Int(i) => builder.append_value(*i),
                    GenericValue::Null if nullable => builder.append_null(),
                    _ => builder.append_value(0),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::Float64 => {
            let mut builder = Float64Builder::new();
            for row in rows {
                match &row[col_idx].1 {
                    GenericValue::Float(f) => builder.append_value(*f),
                    GenericValue::Int(i) => builder.append_value(*i as f64),
                    GenericValue::Null if nullable => builder.append_null(),
                    _ => builder.append_value(0.0),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::Boolean => {
            let mut builder = BooleanBuilder::new();
            for row in rows {
                match &row[col_idx].1 {
                    GenericValue::Bool(b) => builder.append_value(*b),
                    GenericValue::Null if nullable => builder.append_null(),
                    _ => builder.append_value(false),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        dt => anyhow::bail!("Unsupported Arrow data type: {:?}", dt),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ColumnConfig, GeneratorSpec};
    use tempfile::TempDir;

    fn make_simple_config(output: &str, num_rows: usize) -> DatasetConfig {
        DatasetConfig {
            name: "test".to_string(),
            output: output.to_string(),
            num_rows,
            seed: Some(42),
            partition: None,
            entity: None,
            columns: vec![
                ColumnConfig {
                    name: "id".to_string(),
                    generator: GeneratorSpec::Uuid,
                },
                ColumnConfig {
                    name: "value".to_string(),
                    generator: GeneratorSpec::UniformInt { min: 1, max: 100 },
                },
            ],
        }
    }

    #[test]
    fn test_write_single_file() {
        let tmp = TempDir::new().unwrap();
        let output = tmp.path().to_str().unwrap().to_string();
        let config = make_simple_config(&output, 1000);
        let count = write_generic_dataset(&config, 42, None).unwrap();
        assert_eq!(count, 1000);
        assert!(tmp.path().join("data.parquet").exists());
    }

    #[test]
    fn test_write_partitioned() {
        use crate::config::PartitionConfig;
        let tmp = TempDir::new().unwrap();
        let output = tmp.path().to_str().unwrap().to_string();
        let config = DatasetConfig {
            name: "test".to_string(),
            output: output.clone(),
            num_rows: 300,
            seed: Some(42),
            partition: Some(PartitionConfig {
                column: "event_date".to_string(),
                start: "2024-01-01".to_string(),
                days: 3,
            }),
            entity: None,
            columns: vec![ColumnConfig {
                name: "id".to_string(),
                generator: GeneratorSpec::Uuid,
            }],
        };
        let count = write_generic_dataset(&config, 42, None).unwrap();
        assert!(count > 0);
        // Check partition dirs exist
        for i in 0..3 {
            let date = NaiveDate::from_ymd_opt(2024, 1, 1 + i).unwrap();
            let dir = tmp.path().join(format!("event_date={}", date));
            assert!(dir.exists(), "Partition dir {:?} should exist", dir);
            assert!(dir.join("data.parquet").exists());
        }
    }

    #[test]
    fn test_deterministic_output() {
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();
        let config1 = make_simple_config(tmp1.path().to_str().unwrap(), 500);
        let config2 = make_simple_config(tmp2.path().to_str().unwrap(), 500);
        write_generic_dataset(&config1, 42, None).unwrap();
        write_generic_dataset(&config2, 42, None).unwrap();
        let bytes1 = std::fs::read(tmp1.path().join("data.parquet")).unwrap();
        let bytes2 = std::fs::read(tmp2.path().join("data.parquet")).unwrap();
        assert_eq!(bytes1, bytes2, "Output should be deterministic");
    }
}
