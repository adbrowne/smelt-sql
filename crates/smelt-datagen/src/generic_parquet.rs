//! Dynamic Parquet writer for YAML-configured datasets.

use crate::config::{DatasetConfig, FkCounts};
use crate::generic::{
    generate_row, make_entity_pool, sample_pools, GenericValue, LinkedPool, RowContext,
};
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
use std::collections::HashMap;
use std::fs::{self, File};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const BATCH_SIZE: usize = 64 * 1024;

/// Write a dataset configured by `config` to Parquet.
///
/// `fk_counts` maps dataset names to their scaled row counts for `ForeignKey` resolution.
/// Returns the total number of rows written.
pub fn write_generic_dataset(
    config: &DatasetConfig,
    global_seed: u64,
    progress: Option<&(dyn Fn(usize, usize) + Sync)>,
    fk_counts: &FkCounts,
) -> Result<usize> {
    let seed = config.seed.unwrap_or(global_seed);
    let output = Path::new(&config.output);
    fs::create_dir_all(output)
        .with_context(|| format!("Failed to create output directory: {:?}", output))?;

    let schema = Arc::new(build_schema(config));

    // Build linked pools ONCE before any row or partition loop (spec §Semantics:
    // "Pools are built once per dataset and shared across all partitions").
    let linked_pools = Arc::new(build_linked_pools(config, seed, fk_counts));

    if let Some(part_cfg) = &config.partition {
        write_partitioned(
            config,
            seed,
            output,
            schema,
            part_cfg,
            progress,
            fk_counts,
            linked_pools,
        )
    } else {
        write_single(
            config,
            seed,
            output,
            schema,
            progress,
            fk_counts,
            linked_pools,
        )
    }
}

// ---------------------------------------------------------------------------
// Schema construction
// ---------------------------------------------------------------------------

fn build_schema(config: &DatasetConfig) -> Schema {
    use crate::config::GeneratorSpec;

    // Build a lookup: pool_name → pool_config (shapes[0].fields is the type oracle).
    let pool_configs: HashMap<&str, &crate::config::LinkedPoolConfig> = config
        .linked_pools
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|p| (p.name.as_str(), p))
        .collect();

    /// Resolve the Arrow type and nullability for a generator spec, handling
    /// `LinkedChoice` by looking up the referenced field's generator in the
    /// first shape of the named pool.
    fn resolve_field_type<'a>(
        gen: &'a GeneratorSpec,
        pool_configs: &HashMap<&str, &'a crate::config::LinkedPoolConfig>,
    ) -> (DataType, bool) {
        match gen {
            GeneratorSpec::LinkedChoice { pool, field } => {
                if let Some(pool_cfg) = pool_configs.get(pool.as_str()) {
                    if let Some(field_gen) = pool_cfg.shapes[0].fields.get(field.as_str()) {
                        return (field_gen.arrow_type(), field_gen.is_nullable());
                    }
                }
                // validate_config should have caught this; fall back to placeholder
                (DataType::Utf8, false)
            }
            other => (other.arrow_type(), other.is_nullable()),
        }
    }

    let mut fields: Vec<Field> = Vec::new();

    // Entity columns first. Honor Optional<...>::is_nullable() so that an
    // optional entity attribute (e.g. an FK that not every entity has)
    // round-trips as a real NULL rather than the type's zero value.
    if let Some(entity) = &config.entity {
        for col in &entity.columns {
            let (dt, nullable) = resolve_field_type(&col.generator, &pool_configs);
            fields.push(Field::new(&col.name, dt, nullable));
        }
    }

    // Regular columns
    for col in &config.columns {
        let (dt, nullable) = resolve_field_type(&col.generator, &pool_configs);
        fields.push(Field::new(&col.name, dt, nullable));
    }

    // Partition column last (always Utf8, not nullable)
    if let Some(part) = &config.partition {
        fields.push(Field::new(&part.column, DataType::Utf8, false));
    }

    Schema::new(fields)
}

/// Build the linked pools map for a dataset: pool_name → Arc<LinkedPool>.
///
/// Seed derivation (per §Semantics — Pool construction):
/// - If the pool config has an explicit `seed:`, use it.
/// - Otherwise: `dataset_seed.wrapping_add(100 + pool_index)`.
///
/// The `+100` offset avoids collision with entity-pool offsets (`+1`, `+2`).
fn build_linked_pools(
    config: &DatasetConfig,
    dataset_seed: u64,
    fk_counts: &FkCounts,
) -> HashMap<String, Arc<LinkedPool>> {
    config
        .linked_pools
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .map(|(i, pool_cfg)| {
            let seed = pool_cfg
                .seed
                .unwrap_or_else(|| dataset_seed.wrapping_add(100 + i as u64));
            let pool = Arc::new(LinkedPool::new(seed, pool_cfg, fk_counts));
            (pool_cfg.name.clone(), pool)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Partitioned write (one file per day, parallel)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn write_partitioned(
    config: &DatasetConfig,
    seed: u64,
    output: &Path,
    schema: Arc<Schema>,
    part_cfg: &crate::config::PartitionConfig,
    progress: Option<&(dyn Fn(usize, usize) + Sync)>,
    fk_counts: &FkCounts,
    linked_pools: Arc<HashMap<String, Arc<LinkedPool>>>,
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
            let base_offset = i as usize * rows_per_day;
            (date, day_seeds[i as usize], base_offset)
        })
        .collect();

    days_vec
        .par_iter()
        .try_for_each(|(date, day_seed, base_offset)| -> Result<()> {
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
                *base_offset,
                fk_counts,
                &linked_pools,
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
    fk_counts: &FkCounts,
    linked_pools: Arc<HashMap<String, Arc<LinkedPool>>>,
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
        0,
        fk_counts,
        &linked_pools,
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
    base_offset: usize,
    fk_counts: &FkCounts,
    linked_pools: &HashMap<String, Arc<LinkedPool>>,
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
            .map(|i| {
                let row_index = base_offset + written + i;
                // Sample an entity row if a pool exists
                let entity_row = entity_pool.map(|pool| {
                    let idx = (rng.next_u64() as usize) % pool.len();
                    pool.rows[idx].as_slice()
                });
                // Sample one pool-entry index per linked pool (once per row).
                let pool_samples = sample_pools(&mut rng, linked_pools);
                let ctx = RowContext {
                    row_index,
                    fk_counts,
                    pools: linked_pools,
                    pool_samples: &pool_samples,
                };
                generate_row(
                    &mut rng,
                    entity_col_specs,
                    entity_row,
                    col_specs,
                    partition_col,
                    &ctx,
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
                    GenericValue::Str(s) | GenericValue::JsonRaw(s) => builder.append_value(s),
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
            linked_pools: None,
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
        let count = write_generic_dataset(&config, 42, None, &FkCounts::new()).unwrap();
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
            linked_pools: None,
            columns: vec![ColumnConfig {
                name: "id".to_string(),
                generator: GeneratorSpec::Uuid,
            }],
        };
        let count = write_generic_dataset(&config, 42, None, &FkCounts::new()).unwrap();
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
        write_generic_dataset(&config1, 42, None, &FkCounts::new()).unwrap();
        write_generic_dataset(&config2, 42, None, &FkCounts::new()).unwrap();
        let bytes1 = std::fs::read(tmp1.path().join("data.parquet")).unwrap();
        let bytes2 = std::fs::read(tmp2.path().join("data.parquet")).unwrap();
        assert_eq!(bytes1, bytes2, "Output should be deterministic");
    }

    #[test]
    fn test_date_generator() {
        let tmp = TempDir::new().unwrap();
        let output = tmp.path().to_str().unwrap().to_string();
        let config = DatasetConfig {
            name: "test_dates".to_string(),
            output,
            num_rows: 100,
            seed: Some(42),
            partition: None,
            entity: None,
            linked_pools: None,
            columns: vec![ColumnConfig {
                name: "event_date".to_string(),
                generator: GeneratorSpec::Date {
                    start: "2020-01-01".to_string(),
                    end: "2024-12-31".to_string(),
                },
            }],
        };
        let count = write_generic_dataset(&config, 42, None, &FkCounts::new()).unwrap();
        assert_eq!(count, 100);
        assert!(tmp.path().join("data.parquet").exists());
    }

    #[test]
    fn test_timestamp_generator() {
        let tmp = TempDir::new().unwrap();
        let output = tmp.path().to_str().unwrap().to_string();
        let config = DatasetConfig {
            name: "test_timestamps".to_string(),
            output,
            num_rows: 100,
            seed: Some(42),
            partition: None,
            entity: None,
            linked_pools: None,
            columns: vec![ColumnConfig {
                name: "created_at".to_string(),
                generator: GeneratorSpec::Timestamp {
                    start: "2024-01-01T00:00:00".to_string(),
                    end: "2024-03-31T23:59:59".to_string(),
                },
            }],
        };
        let count = write_generic_dataset(&config, 42, None, &FkCounts::new()).unwrap();
        assert_eq!(count, 100);
        assert!(tmp.path().join("data.parquet").exists());
    }

    /// B11 regression: Optional<ForeignKey> must produce real NULLs in the
    /// parquet output, not 0 (the int default). Iter-3 of smelt-shop reported
    /// 45% of `customer_id` rows coming through as 0 instead of NULL when the
    /// generator was `optional { prob: 0.55, inner: foreign_key(customers) }`.
    #[test]
    fn test_optional_foreign_key_emits_nulls() {
        use arrow::array::Array;
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
        let tmp = TempDir::new().unwrap();
        let output = tmp.path().to_str().unwrap().to_string();
        let mut fk = FkCounts::new();
        fk.insert("customers".to_string(), 100);
        let config = DatasetConfig {
            name: "test_optional_fk".to_string(),
            output,
            num_rows: 1000,
            seed: Some(42),
            partition: None,
            entity: None,
            linked_pools: None,
            columns: vec![ColumnConfig {
                name: "customer_id".to_string(),
                generator: GeneratorSpec::Optional {
                    prob: 0.55,
                    inner: Box::new(GeneratorSpec::ForeignKey {
                        dataset: "customers".to_string(),
                    }),
                },
            }],
        };
        write_generic_dataset(&config, 42, None, &fk).unwrap();

        let file = File::open(tmp.path().join("data.parquet")).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap()
            .build()
            .unwrap();

        let mut total = 0usize;
        let mut nulls = 0usize;
        let mut zeros = 0usize;
        for batch in reader {
            let batch = batch.unwrap();
            let arr = batch
                .column(0)
                .as_any()
                .downcast_ref::<arrow::array::Int32Array>()
                .unwrap();
            for i in 0..arr.len() {
                total += 1;
                if arr.is_null(i) {
                    nulls += 1;
                } else if arr.value(i) == 0 {
                    zeros += 1;
                }
            }
        }

        assert_eq!(total, 1000);
        // With prob=0.55, expect ~45% NULL (~450 rows). Allow generous slack.
        assert!(
            nulls > 300,
            "Optional<ForeignKey> must emit real NULLs, got {nulls} nulls / {zeros} zeros / {total} total"
        );
        assert_eq!(
            zeros, 0,
            "Optional<ForeignKey> must not emit 0 as a stand-in for NULL, got {zeros} zeros"
        );
    }

    /// B11 (partitioned variant): same as test_optional_foreign_key_emits_nulls
    /// but with Hive partitioning, since iter-3 reported the bug on
    /// `page_events` which is a partitioned dataset.
    #[test]
    fn test_optional_foreign_key_emits_nulls_partitioned() {
        use crate::config::PartitionConfig;
        use arrow::array::Array;
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
        let tmp = TempDir::new().unwrap();
        let output = tmp.path().to_str().unwrap().to_string();
        let mut fk = FkCounts::new();
        fk.insert("customers".to_string(), 100);
        let config = DatasetConfig {
            name: "test_optional_fk_part".to_string(),
            output: output.clone(),
            num_rows: 1000,
            seed: Some(42),
            partition: Some(PartitionConfig {
                column: "event_date".to_string(),
                start: "2024-01-01".to_string(),
                days: 5,
            }),
            entity: None,
            linked_pools: None,
            columns: vec![ColumnConfig {
                name: "customer_id".to_string(),
                generator: GeneratorSpec::Optional {
                    prob: 0.55,
                    inner: Box::new(GeneratorSpec::ForeignKey {
                        dataset: "customers".to_string(),
                    }),
                },
            }],
        };
        write_generic_dataset(&config, 42, None, &fk).unwrap();

        let mut total = 0usize;
        let mut nulls = 0usize;
        let mut zeros = 0usize;
        for entry in std::fs::read_dir(&output).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path().join("data.parquet");
            if !path.exists() {
                continue;
            }
            let file = File::open(&path).unwrap();
            let reader = ParquetRecordBatchReaderBuilder::try_new(file)
                .unwrap()
                .build()
                .unwrap();
            for batch in reader {
                let batch = batch.unwrap();
                let arr = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<arrow::array::Int32Array>()
                    .unwrap();
                for i in 0..arr.len() {
                    total += 1;
                    if arr.is_null(i) {
                        nulls += 1;
                    } else if arr.value(i) == 0 {
                        zeros += 1;
                    }
                }
            }
        }

        assert_eq!(total, 1000);
        assert!(
            nulls > 300,
            "Optional<ForeignKey> (partitioned) must emit real NULLs, got {nulls} nulls / {zeros} zeros / {total} total"
        );
        assert_eq!(
            zeros, 0,
            "Optional<ForeignKey> (partitioned) must not emit 0 as a NULL stand-in, got {zeros} zeros"
        );
    }

    /// B11 (entity-column variant): an Optional generator placed under
    /// `entity.columns` must also produce real NULLs. Iter-3's `page_events`
    /// likely had `customer_id` as an entity attribute (sticky per-session),
    /// where Optional was being silently coerced to 0 because entity columns
    /// were unconditionally marked `nullable: false`.
    #[test]
    fn test_optional_entity_column_emits_nulls() {
        use crate::config::EntityConfig;
        use arrow::array::Array;
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
        let tmp = TempDir::new().unwrap();
        let output = tmp.path().to_str().unwrap().to_string();
        let mut fk = FkCounts::new();
        fk.insert("customers".to_string(), 100);
        let config = DatasetConfig {
            name: "test_optional_entity".to_string(),
            output: output.clone(),
            num_rows: 1000,
            seed: Some(42),
            partition: None,
            entity: Some(EntityConfig {
                pool_ratio: 0.1,
                columns: vec![ColumnConfig {
                    name: "customer_id".to_string(),
                    generator: GeneratorSpec::Optional {
                        prob: 0.55,
                        inner: Box::new(GeneratorSpec::ForeignKey {
                            dataset: "customers".to_string(),
                        }),
                    },
                }],
            }),
            linked_pools: None,
            columns: vec![ColumnConfig {
                name: "session_id".to_string(),
                generator: GeneratorSpec::Uuid,
            }],
        };
        write_generic_dataset(&config, 42, None, &fk).unwrap();

        let file = File::open(tmp.path().join("data.parquet")).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap()
            .build()
            .unwrap();

        let mut total = 0usize;
        let mut nulls = 0usize;
        let mut zeros = 0usize;
        for batch in reader {
            let batch = batch.unwrap();
            let arr = batch
                .column(0)
                .as_any()
                .downcast_ref::<arrow::array::Int32Array>()
                .unwrap();
            for i in 0..arr.len() {
                total += 1;
                if arr.is_null(i) {
                    nulls += 1;
                } else if arr.value(i) == 0 {
                    zeros += 1;
                }
            }
        }

        assert_eq!(total, 1000);
        assert!(
            nulls > 200,
            "Optional entity column must emit real NULLs, got {nulls} nulls / {zeros} zeros / {total} total"
        );
        assert_eq!(
            zeros, 0,
            "Optional entity column must not emit 0 as a NULL stand-in, got {zeros} zeros"
        );
    }

    #[test]
    fn test_json_object_writes_parquet_single_file() {
        use arrow::array::Array;
        use indexmap::IndexMap;
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

        let tmp = TempDir::new().unwrap();
        let output = tmp.path().to_str().unwrap().to_string();

        let mut fields = IndexMap::new();
        fields.insert(
            "kind".to_string(),
            GeneratorSpec::Constant {
                value: serde_yaml::Value::String("page_view".to_string()),
            },
        );
        fields.insert(
            "count".to_string(),
            GeneratorSpec::UniformInt { min: 1, max: 10 },
        );
        fields.insert("active".to_string(), GeneratorSpec::Bool { prob: 1.0 });

        let config = DatasetConfig {
            name: "test_json_object".to_string(),
            output,
            num_rows: 50,
            seed: Some(42),
            partition: None,
            entity: None,
            linked_pools: None,
            columns: vec![ColumnConfig {
                name: "payload".to_string(),
                generator: GeneratorSpec::JsonObject { fields },
            }],
        };
        write_generic_dataset(&config, 42, None, &FkCounts::new()).unwrap();

        let file = File::open(tmp.path().join("data.parquet")).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap()
            .build()
            .unwrap();

        let mut total = 0usize;
        for batch in reader {
            let batch = batch.unwrap();
            let arr = batch
                .column(0)
                .as_any()
                .downcast_ref::<arrow::array::StringArray>()
                .expect("payload column must be Utf8");
            for i in 0..arr.len() {
                total += 1;
                let s = arr.value(i);
                // Every row's JSON must contain all three fields in order.
                assert!(
                    s.starts_with(r#"{"kind":"page_view","count":"#),
                    "row {i}: {s}"
                );
                assert!(s.ends_with(r#","active":true}"#), "row {i}: {s}");
            }
        }
        assert_eq!(total, 50);
    }

    #[test]
    fn test_json_object_writes_parquet_partitioned() {
        use crate::config::PartitionConfig;
        use arrow::array::Array;
        use indexmap::IndexMap;
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

        let tmp = TempDir::new().unwrap();
        let output = tmp.path().to_str().unwrap().to_string();

        let mut fields = IndexMap::new();
        fields.insert(
            "n".to_string(),
            GeneratorSpec::UniformInt { min: 0, max: 100 },
        );

        let config = DatasetConfig {
            name: "test_json_part".to_string(),
            output: output.clone(),
            num_rows: 30,
            seed: Some(42),
            partition: Some(PartitionConfig {
                column: "event_date".to_string(),
                start: "2024-01-01".to_string(),
                days: 3,
            }),
            entity: None,
            linked_pools: None,
            columns: vec![ColumnConfig {
                name: "payload".to_string(),
                generator: GeneratorSpec::JsonObject { fields },
            }],
        };
        write_generic_dataset(&config, 42, None, &FkCounts::new()).unwrap();

        let mut total = 0usize;
        for entry in std::fs::read_dir(&output).unwrap() {
            let path = entry.unwrap().path().join("data.parquet");
            if !path.exists() {
                continue;
            }
            let file = File::open(&path).unwrap();
            let reader = ParquetRecordBatchReaderBuilder::try_new(file)
                .unwrap()
                .build()
                .unwrap();
            for batch in reader {
                let batch = batch.unwrap();
                let payload = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<arrow::array::StringArray>()
                    .expect("payload column must be Utf8");
                for i in 0..payload.len() {
                    total += 1;
                    let s = payload.value(i);
                    assert!(s.starts_with(r#"{"n":"#), "row {i}: {s}");
                    assert!(s.ends_with('}'), "row {i}: {s}");
                }
            }
        }
        assert_eq!(total, 30);
    }

    #[test]
    fn test_json_object_entity_column_is_sticky() {
        use crate::config::EntityConfig;
        use arrow::array::Array;
        use indexmap::IndexMap;
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
        use std::collections::HashSet;

        let tmp = TempDir::new().unwrap();
        let output = tmp.path().to_str().unwrap().to_string();

        let mut fields = IndexMap::new();
        fields.insert(
            "tier".to_string(),
            GeneratorSpec::OneOf {
                values: vec!["A".into(), "B".into(), "C".into()],
            },
        );
        fields.insert(
            "score".to_string(),
            GeneratorSpec::UniformInt { min: 0, max: 1000 },
        );

        let config = DatasetConfig {
            name: "test_json_entity".to_string(),
            output,
            num_rows: 1000,
            seed: Some(42),
            partition: None,
            entity: Some(EntityConfig {
                pool_ratio: 0.05, // 50 entities for 1000 rows
                columns: vec![ColumnConfig {
                    name: "user_attrs".to_string(),
                    generator: GeneratorSpec::JsonObject { fields },
                }],
            }),
            linked_pools: None,
            columns: vec![ColumnConfig {
                name: "event_id".to_string(),
                generator: GeneratorSpec::SequentialId,
            }],
        };
        write_generic_dataset(&config, 42, None, &FkCounts::new()).unwrap();

        let file = File::open(tmp.path().join("data.parquet")).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap()
            .build()
            .unwrap();

        let mut distinct_payloads: HashSet<String> = HashSet::new();
        for batch in reader {
            let batch = batch.unwrap();
            let arr = batch
                .column(0)
                .as_any()
                .downcast_ref::<arrow::array::StringArray>()
                .expect("user_attrs must be Utf8");
            for i in 0..arr.len() {
                distinct_payloads.insert(arr.value(i).to_string());
            }
        }
        // ≤ 50 distinct JSON values across 1000 rows proves the entity-column
        // generator was evaluated once per entity, not per row. Allow some
        // slack in case the random `tier`/`score` pair collides across
        // entities.
        assert!(
            distinct_payloads.len() <= 50,
            "entity-column json_object should be sticky; got {} distinct payloads",
            distinct_payloads.len()
        );
    }

    /// Phase 4 TDD item: a `foreign_key` sub-generator nested inside a
    /// **row-level** `json_object` field must resolve FK ranges correctly —
    /// every emitted FK id must fall in `[1, referenced_dataset_row_count]`.
    /// This exercises the row-level `apply_spec` path, where `fk_counts` is
    /// threaded in via `generate_row`. (The companion entity-column path is
    /// tracked separately in "Deferred during implementation" of the Phase 1
    /// plan — entity-pool `apply_spec` invocations carry an empty `FkCounts`,
    /// which is a pre-existing issue.)
    #[test]
    fn test_json_object_with_foreign_key_inner_generator() {
        use arrow::array::Array;
        use indexmap::IndexMap;
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

        let tmp = TempDir::new().unwrap();
        let output = tmp.path().to_str().unwrap().to_string();

        let mut fk = FkCounts::new();
        fk.insert("customers".to_string(), 50);

        let mut fields = IndexMap::new();
        fields.insert(
            "customer_id".to_string(),
            GeneratorSpec::ForeignKey {
                dataset: "customers".to_string(),
            },
        );
        fields.insert(
            "label".to_string(),
            GeneratorSpec::Constant {
                value: serde_yaml::Value::String("x".to_string()),
            },
        );

        let config = DatasetConfig {
            name: "test_json_fk".to_string(),
            output,
            num_rows: 500,
            seed: Some(42),
            partition: None,
            entity: None,
            linked_pools: None,
            columns: vec![ColumnConfig {
                name: "payload".to_string(),
                generator: GeneratorSpec::JsonObject { fields },
            }],
        };
        write_generic_dataset(&config, 42, None, &fk).unwrap();

        let file = File::open(tmp.path().join("data.parquet")).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap()
            .build()
            .unwrap();

        // The FK must resolve into [1, 50]. Track the range and distinct
        // values to prove the FK is consuming RNG (not stuck at id 1).
        let mut total = 0usize;
        let mut min_id = i64::MAX;
        let mut max_id = i64::MIN;
        let mut distinct: std::collections::HashSet<i64> = std::collections::HashSet::new();
        for batch in reader {
            let batch = batch.unwrap();
            let arr = batch
                .column(0)
                .as_any()
                .downcast_ref::<arrow::array::StringArray>()
                .expect("payload column must be Utf8");
            for i in 0..arr.len() {
                total += 1;
                let s = arr.value(i);
                // Pull "customer_id":N out of the prefix without dragging in
                // a JSON dependency just for the test.
                let prefix = r#"{"customer_id":"#;
                assert!(
                    s.starts_with(prefix),
                    "row {i} must start with {prefix}: {s}"
                );
                let rest = &s[prefix.len()..];
                let comma = rest.find(',').expect("delimiter after customer_id");
                let id: i64 = rest[..comma]
                    .parse()
                    .unwrap_or_else(|e| panic!("non-numeric FK in row {i}: {s} ({e})"));
                assert!(
                    (1..=50).contains(&id),
                    "FK out of range [1,50] in row {i}: id={id}, payload={s}"
                );
                min_id = min_id.min(id);
                max_id = max_id.max(id);
                distinct.insert(id);
            }
        }
        assert_eq!(total, 500);
        // With 50 candidate ids and 500 rows, expect broad coverage. If the
        // FK were stuck at id 1, distinct.len() would be 1.
        assert!(
            distinct.len() > 10,
            "FK appears not to consume RNG (only {} distinct ids across {} rows)",
            distinct.len(),
            total
        );
        assert!(
            min_id >= 1 && max_id <= 50,
            "FK range outside [1,50]: min={min_id}, max={max_id}"
        );
    }

    #[test]
    fn test_json_object_deterministic_partitioned() {
        use crate::config::PartitionConfig;
        use indexmap::IndexMap;

        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();

        let make_config = |output: &str| {
            let mut fields = IndexMap::new();
            fields.insert(
                "k".to_string(),
                GeneratorSpec::UniformInt { min: 0, max: 100 },
            );
            DatasetConfig {
                name: "det_json".to_string(),
                output: output.to_string(),
                num_rows: 60,
                seed: Some(7),
                partition: Some(PartitionConfig {
                    column: "event_date".to_string(),
                    start: "2024-02-01".to_string(),
                    days: 3,
                }),
                entity: None,
                linked_pools: None,
                columns: vec![ColumnConfig {
                    name: "payload".to_string(),
                    generator: GeneratorSpec::JsonObject { fields },
                }],
            }
        };
        write_generic_dataset(
            &make_config(tmp1.path().to_str().unwrap()),
            7,
            None,
            &FkCounts::new(),
        )
        .unwrap();
        write_generic_dataset(
            &make_config(tmp2.path().to_str().unwrap()),
            7,
            None,
            &FkCounts::new(),
        )
        .unwrap();

        for entry in std::fs::read_dir(tmp1.path()).unwrap() {
            let entry = entry.unwrap();
            let rel = entry.file_name();
            let p1 = entry.path().join("data.parquet");
            let p2 = tmp2.path().join(&rel).join("data.parquet");
            if !p1.exists() || !p2.exists() {
                continue;
            }
            let b1 = std::fs::read(&p1).unwrap();
            let b2 = std::fs::read(&p2).unwrap();
            assert_eq!(b1, b2, "partition {:?} must be byte-identical", rel);
        }
    }

    #[test]
    fn test_string_pattern_generator() {
        let tmp = TempDir::new().unwrap();
        let output = tmp.path().to_str().unwrap().to_string();
        let config = DatasetConfig {
            name: "test_patterns".to_string(),
            output,
            num_rows: 100,
            seed: Some(42),
            partition: None,
            entity: None,
            linked_pools: None,
            columns: vec![
                ColumnConfig {
                    name: "email".to_string(),
                    generator: GeneratorSpec::StringPattern {
                        template: "user_{sequential_id}@example.com".to_string(),
                    },
                },
                ColumnConfig {
                    name: "sku".to_string(),
                    generator: GeneratorSpec::StringPattern {
                        template: "SKU-{uniform_int:1000-9999}".to_string(),
                    },
                },
            ],
        };
        let count = write_generic_dataset(&config, 42, None, &FkCounts::new()).unwrap();
        assert_eq!(count, 100);
        assert!(tmp.path().join("data.parquet").exists());
    }

    // ── Phase 4 TDD: linked_choice Parquet round-trip ────────────────────────

    use crate::config::{EntityConfig, LinkedPoolConfig, PartitionConfig, ShapeConfig};
    use arrow::array::{Array, RecordBatchReader};
    use indexmap::IndexMap;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    /// Build a two-field FK pool config (device_id, user_id).
    fn make_two_fk_pool(
        name: &str,
        pool_size: usize,
        seed: Option<u64>,
        device_count: usize,
        user_count: usize,
    ) -> LinkedPoolConfig {
        let mut fk = FkCounts::new();
        fk.insert("devices".to_string(), device_count);
        fk.insert("users".to_string(), user_count);
        let _ = fk; // fk is for documentation; FkCounts passed separately at write time
        let mut fields = IndexMap::new();
        fields.insert(
            "device_id".to_string(),
            GeneratorSpec::ForeignKey {
                dataset: "devices".to_string(),
            },
        );
        fields.insert(
            "user_id".to_string(),
            GeneratorSpec::ForeignKey {
                dataset: "users".to_string(),
            },
        );
        LinkedPoolConfig {
            name: name.to_string(),
            pool_size,
            seed,
            shapes: vec![ShapeConfig {
                weight: 1.0,
                emit: 1,
                sticky: vec![],
                fields,
            }],
        }
    }

    fn make_fk_counts(device_count: usize, user_count: usize) -> FkCounts {
        let mut fk = FkCounts::new();
        fk.insert("devices".to_string(), device_count);
        fk.insert("users".to_string(), user_count);
        fk
    }

    /// 1. Single-file write with linked_choice: read back and verify every row's
    ///    (device_id, user_id) traces to exactly one pool entry.
    #[test]
    fn test_linked_choice_writes_parquet_single_file() {
        let tmp = TempDir::new().unwrap();
        let output = tmp.path().to_str().unwrap().to_string();
        let fk = make_fk_counts(100, 100);

        let config = DatasetConfig {
            name: "events".to_string(),
            output: output.clone(),
            num_rows: 1000,
            seed: Some(42),
            partition: None,
            entity: None,
            linked_pools: Some(vec![make_two_fk_pool("du", 100, Some(99), 100, 100)]),
            columns: vec![
                ColumnConfig {
                    name: "device_id".to_string(),
                    generator: GeneratorSpec::LinkedChoice {
                        pool: "du".to_string(),
                        field: "device_id".to_string(),
                    },
                },
                ColumnConfig {
                    name: "user_id".to_string(),
                    generator: GeneratorSpec::LinkedChoice {
                        pool: "du".to_string(),
                        field: "user_id".to_string(),
                    },
                },
            ],
        };

        let count = write_generic_dataset(&config, 42, None, &fk).unwrap();
        assert_eq!(count, 1000);

        // Rebuild the pool with the same seed to validate against.
        let pool_cfg = &config.linked_pools.as_ref().unwrap()[0];
        let pool = crate::generic::LinkedPool::new(99, pool_cfg, &fk);
        // Build a set of valid (device_id, user_id) pairs from the pool.
        let valid_pairs: std::collections::HashSet<(i32, i32)> = pool
            .rows
            .iter()
            .map(|r| {
                let d = match &r[0] {
                    crate::generic::GenericValue::Int(v) => *v,
                    _ => panic!("expected Int for device_id"),
                };
                let u = match &r[1] {
                    crate::generic::GenericValue::Int(v) => *v,
                    _ => panic!("expected Int for user_id"),
                };
                (d, u)
            })
            .collect();

        let file = File::open(tmp.path().join("data.parquet")).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap()
            .build()
            .unwrap();

        let mut total = 0usize;
        for batch in reader {
            let batch = batch.unwrap();
            let dev_arr = batch
                .column(0)
                .as_any()
                .downcast_ref::<arrow::array::Int32Array>()
                .expect("device_id must be Int32");
            let usr_arr = batch
                .column(1)
                .as_any()
                .downcast_ref::<arrow::array::Int32Array>()
                .expect("user_id must be Int32");
            for i in 0..batch.num_rows() {
                total += 1;
                let d = dev_arr.value(i);
                let u = usr_arr.value(i);
                assert!(
                    valid_pairs.contains(&(d, u)),
                    "row {total}: (device_id={d}, user_id={u}) not in pool"
                );
            }
        }
        assert_eq!(total, 1000);
    }

    /// 2. Partitioned write: pool shared; per-partition row counts sum to total.
    #[test]
    fn test_linked_choice_writes_parquet_partitioned() {
        let tmp = TempDir::new().unwrap();
        let output = tmp.path().to_str().unwrap().to_string();
        let fk = make_fk_counts(100, 100);

        let config = DatasetConfig {
            name: "events".to_string(),
            output: output.clone(),
            num_rows: 300,
            seed: Some(42),
            partition: Some(PartitionConfig {
                column: "event_date".to_string(),
                start: "2024-01-01".to_string(),
                days: 3,
            }),
            entity: None,
            linked_pools: Some(vec![make_two_fk_pool("du", 50, Some(7), 100, 100)]),
            columns: vec![
                ColumnConfig {
                    name: "device_id".to_string(),
                    generator: GeneratorSpec::LinkedChoice {
                        pool: "du".to_string(),
                        field: "device_id".to_string(),
                    },
                },
                ColumnConfig {
                    name: "user_id".to_string(),
                    generator: GeneratorSpec::LinkedChoice {
                        pool: "du".to_string(),
                        field: "user_id".to_string(),
                    },
                },
            ],
        };

        let count = write_generic_dataset(&config, 42, None, &fk).unwrap();
        assert!(count > 0);

        // Rebuild pool for validation.
        let pool_cfg = &config.linked_pools.as_ref().unwrap()[0];
        let pool = crate::generic::LinkedPool::new(7, pool_cfg, &fk);
        let valid_pairs: std::collections::HashSet<(i32, i32)> = pool
            .rows
            .iter()
            .map(|r| {
                let d = match &r[0] {
                    crate::generic::GenericValue::Int(v) => *v,
                    _ => panic!("expected Int for device_id"),
                };
                let u = match &r[1] {
                    crate::generic::GenericValue::Int(v) => *v,
                    _ => panic!("expected Int for user_id"),
                };
                (d, u)
            })
            .collect();

        let mut total = 0usize;
        let mut partition_counts: Vec<usize> = vec![];
        for entry in std::fs::read_dir(&output).unwrap() {
            let path = entry.unwrap().path().join("data.parquet");
            if !path.exists() {
                continue;
            }
            let file = File::open(&path).unwrap();
            let reader = ParquetRecordBatchReaderBuilder::try_new(file)
                .unwrap()
                .build()
                .unwrap();
            let mut part_count = 0usize;
            for batch in reader {
                let batch = batch.unwrap();
                let dev_arr = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<arrow::array::Int32Array>()
                    .expect("device_id must be Int32");
                let usr_arr = batch
                    .column(1)
                    .as_any()
                    .downcast_ref::<arrow::array::Int32Array>()
                    .expect("user_id must be Int32");
                for i in 0..batch.num_rows() {
                    let d = dev_arr.value(i);
                    let u = usr_arr.value(i);
                    assert!(
                        valid_pairs.contains(&(d, u)),
                        "(device_id={d}, user_id={u}) not in pool"
                    );
                    part_count += 1;
                }
            }
            total += part_count;
            partition_counts.push(part_count);
        }
        assert_eq!(partition_counts.len(), 3, "expected 3 partitions");
        assert_eq!(total, count, "partition row counts must sum to total");
    }

    /// 3. Schema: linked_choice columns resolve to the referenced field's type.
    ///    device_id is ForeignKey → Int32 NOT NULL; user_id is Optional<FK> → Int32 NULL.
    #[test]
    fn test_linked_choice_schema_resolves_field_type() {
        let tmp = TempDir::new().unwrap();
        let output = tmp.path().to_str().unwrap().to_string();
        let fk = make_fk_counts(100, 100);

        let mut fields = IndexMap::new();
        fields.insert(
            "device_id".to_string(),
            GeneratorSpec::ForeignKey {
                dataset: "devices".to_string(),
            },
        );
        fields.insert(
            "user_id".to_string(),
            GeneratorSpec::Optional {
                prob: 0.5,
                inner: Box::new(GeneratorSpec::ForeignKey {
                    dataset: "users".to_string(),
                }),
            },
        );
        let pool = LinkedPoolConfig {
            name: "du".to_string(),
            pool_size: 50,
            seed: Some(1),
            shapes: vec![ShapeConfig {
                weight: 1.0,
                emit: 1,
                sticky: vec![],
                fields,
            }],
        };

        let config = DatasetConfig {
            name: "events".to_string(),
            output,
            num_rows: 100,
            seed: Some(42),
            partition: None,
            entity: None,
            linked_pools: Some(vec![pool]),
            columns: vec![
                ColumnConfig {
                    name: "device_id".to_string(),
                    generator: GeneratorSpec::LinkedChoice {
                        pool: "du".to_string(),
                        field: "device_id".to_string(),
                    },
                },
                ColumnConfig {
                    name: "user_id".to_string(),
                    generator: GeneratorSpec::LinkedChoice {
                        pool: "du".to_string(),
                        field: "user_id".to_string(),
                    },
                },
            ],
        };

        write_generic_dataset(&config, 42, None, &fk).unwrap();

        let file = File::open(tmp.path().join("data.parquet")).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap()
            .build()
            .unwrap();
        let schema = reader.schema();

        let dev_field = schema.field_with_name("device_id").unwrap();
        assert_eq!(
            dev_field.data_type(),
            &DataType::Int32,
            "device_id must be Int32"
        );
        assert!(!dev_field.is_nullable(), "device_id must be NOT NULL");

        let usr_field = schema.field_with_name("user_id").unwrap();
        assert_eq!(
            usr_field.data_type(),
            &DataType::Int32,
            "user_id must be Int32"
        );
        assert!(
            usr_field.is_nullable(),
            "user_id must be nullable (Optional)"
        );
    }

    /// 4. Same row → same (device_id, user_id) tuple from pool.
    ///    Two linked_choice cols referencing the same pool must pick the same index per row.
    #[test]
    fn test_linked_choice_same_row_same_tuple() {
        let tmp = TempDir::new().unwrap();
        let output = tmp.path().to_str().unwrap().to_string();
        let fk = make_fk_counts(100, 1001); // user_id in [1001, 2000]

        // Pool: device_id in [1,100], user_id is UniformInt [1001, 2001)
        let mut fields = IndexMap::new();
        fields.insert(
            "device_id".to_string(),
            GeneratorSpec::ForeignKey {
                dataset: "devices".to_string(),
            },
        );
        fields.insert(
            "user_id".to_string(),
            GeneratorSpec::UniformInt {
                min: 1001,
                max: 2001,
            },
        );
        let pool_cfg = LinkedPoolConfig {
            name: "du".to_string(),
            pool_size: 100,
            seed: Some(77),
            shapes: vec![ShapeConfig {
                weight: 1.0,
                emit: 1,
                sticky: vec![],
                fields,
            }],
        };

        let pool = crate::generic::LinkedPool::new(77, &pool_cfg, &fk);
        let valid_pairs: std::collections::HashSet<(i32, i32)> = pool
            .rows
            .iter()
            .map(|r| {
                let d = match &r[0] {
                    crate::generic::GenericValue::Int(v) => *v,
                    _ => panic!("Int expected"),
                };
                let u = match &r[1] {
                    crate::generic::GenericValue::Int(v) => *v,
                    _ => panic!("Int expected"),
                };
                (d, u)
            })
            .collect();

        let config = DatasetConfig {
            name: "events".to_string(),
            output,
            num_rows: 500,
            seed: Some(42),
            partition: None,
            entity: None,
            linked_pools: Some(vec![pool_cfg]),
            columns: vec![
                ColumnConfig {
                    name: "device_id".to_string(),
                    generator: GeneratorSpec::LinkedChoice {
                        pool: "du".to_string(),
                        field: "device_id".to_string(),
                    },
                },
                ColumnConfig {
                    name: "user_id".to_string(),
                    generator: GeneratorSpec::LinkedChoice {
                        pool: "du".to_string(),
                        field: "user_id".to_string(),
                    },
                },
            ],
        };

        write_generic_dataset(&config, 42, None, &fk).unwrap();

        let file = File::open(tmp.path().join("data.parquet")).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap()
            .build()
            .unwrap();

        let mut total = 0usize;
        for batch in reader {
            let batch = batch.unwrap();
            let dev_arr = batch
                .column(0)
                .as_any()
                .downcast_ref::<arrow::array::Int32Array>()
                .unwrap();
            let usr_arr = batch
                .column(1)
                .as_any()
                .downcast_ref::<arrow::array::Int32Array>()
                .unwrap();
            for i in 0..batch.num_rows() {
                total += 1;
                let pair = (dev_arr.value(i), usr_arr.value(i));
                assert!(
                    valid_pairs.contains(&pair),
                    "row {total}: {pair:?} not in pool — same-pool columns must share tuple index"
                );
            }
        }
        assert_eq!(total, 500);
    }

    /// 5. Two independent pools sample independently within a row.
    #[test]
    fn test_linked_choice_different_pools_independent() {
        let tmp = TempDir::new().unwrap();
        let output = tmp.path().to_str().unwrap().to_string();

        // pool_a: (a_x, a_y) — values in [1, 10]
        let mut fields_a = IndexMap::new();
        fields_a.insert(
            "a_x".to_string(),
            GeneratorSpec::UniformInt { min: 1, max: 11 },
        );
        fields_a.insert(
            "a_y".to_string(),
            GeneratorSpec::UniformInt { min: 1, max: 11 },
        );
        let pool_a = LinkedPoolConfig {
            name: "pool_a".to_string(),
            pool_size: 20,
            seed: Some(1),
            shapes: vec![ShapeConfig {
                weight: 1.0,
                emit: 1,
                sticky: vec![],
                fields: fields_a,
            }],
        };

        // pool_b: (b_x, b_y) — values in [100, 200]
        let mut fields_b = IndexMap::new();
        fields_b.insert(
            "b_x".to_string(),
            GeneratorSpec::UniformInt { min: 100, max: 201 },
        );
        fields_b.insert(
            "b_y".to_string(),
            GeneratorSpec::UniformInt { min: 100, max: 201 },
        );
        let pool_b = LinkedPoolConfig {
            name: "pool_b".to_string(),
            pool_size: 20,
            seed: Some(2),
            shapes: vec![ShapeConfig {
                weight: 1.0,
                emit: 1,
                sticky: vec![],
                fields: fields_b,
            }],
        };

        let config = DatasetConfig {
            name: "events".to_string(),
            output,
            num_rows: 200,
            seed: Some(42),
            partition: None,
            entity: None,
            linked_pools: Some(vec![pool_a.clone(), pool_b.clone()]),
            columns: vec![
                ColumnConfig {
                    name: "a_x".to_string(),
                    generator: GeneratorSpec::LinkedChoice {
                        pool: "pool_a".to_string(),
                        field: "a_x".to_string(),
                    },
                },
                ColumnConfig {
                    name: "a_y".to_string(),
                    generator: GeneratorSpec::LinkedChoice {
                        pool: "pool_a".to_string(),
                        field: "a_y".to_string(),
                    },
                },
                ColumnConfig {
                    name: "b_x".to_string(),
                    generator: GeneratorSpec::LinkedChoice {
                        pool: "pool_b".to_string(),
                        field: "b_x".to_string(),
                    },
                },
                ColumnConfig {
                    name: "b_y".to_string(),
                    generator: GeneratorSpec::LinkedChoice {
                        pool: "pool_b".to_string(),
                        field: "b_y".to_string(),
                    },
                },
            ],
        };

        write_generic_dataset(&config, 42, None, &FkCounts::new()).unwrap();

        // Validate that a_x/a_y is a valid pool_a tuple and b_x/b_y is a valid pool_b tuple.
        let pool_a_built = crate::generic::LinkedPool::new(1, &pool_a, &FkCounts::new());
        let pool_b_built = crate::generic::LinkedPool::new(2, &pool_b, &FkCounts::new());

        let valid_a: std::collections::HashSet<(i32, i32)> = pool_a_built
            .rows
            .iter()
            .map(|r| {
                let x = match &r[0] {
                    crate::generic::GenericValue::Int(v) => *v,
                    _ => panic!("Int"),
                };
                let y = match &r[1] {
                    crate::generic::GenericValue::Int(v) => *v,
                    _ => panic!("Int"),
                };
                (x, y)
            })
            .collect();
        let valid_b: std::collections::HashSet<(i32, i32)> = pool_b_built
            .rows
            .iter()
            .map(|r| {
                let x = match &r[0] {
                    crate::generic::GenericValue::Int(v) => *v,
                    _ => panic!("Int"),
                };
                let y = match &r[1] {
                    crate::generic::GenericValue::Int(v) => *v,
                    _ => panic!("Int"),
                };
                (x, y)
            })
            .collect();

        let file = File::open(tmp.path().join("data.parquet")).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap()
            .build()
            .unwrap();

        let mut total = 0usize;
        for batch in reader {
            let batch = batch.unwrap();
            let ax = batch
                .column(0)
                .as_any()
                .downcast_ref::<arrow::array::Int32Array>()
                .unwrap();
            let ay = batch
                .column(1)
                .as_any()
                .downcast_ref::<arrow::array::Int32Array>()
                .unwrap();
            let bx = batch
                .column(2)
                .as_any()
                .downcast_ref::<arrow::array::Int32Array>()
                .unwrap();
            let by_ = batch
                .column(3)
                .as_any()
                .downcast_ref::<arrow::array::Int32Array>()
                .unwrap();
            for i in 0..batch.num_rows() {
                total += 1;
                assert!(
                    valid_a.contains(&(ax.value(i), ay.value(i))),
                    "row {total}: pool_a tuple ({},{}) not valid",
                    ax.value(i),
                    ay.value(i)
                );
                assert!(
                    valid_b.contains(&(bx.value(i), by_.value(i))),
                    "row {total}: pool_b tuple ({},{}) not valid",
                    bx.value(i),
                    by_.value(i)
                );
            }
        }
        assert_eq!(total, 200);
    }

    /// 6. Deterministic: same config + same seed → byte-identical Parquet.
    #[test]
    fn test_linked_choice_deterministic_across_runs() {
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();
        let fk = make_fk_counts(100, 100);

        let make_cfg = |output: &str| DatasetConfig {
            name: "events".to_string(),
            output: output.to_string(),
            num_rows: 200,
            seed: Some(55),
            partition: None,
            entity: None,
            linked_pools: Some(vec![make_two_fk_pool("du", 50, Some(3), 100, 100)]),
            columns: vec![
                ColumnConfig {
                    name: "device_id".to_string(),
                    generator: GeneratorSpec::LinkedChoice {
                        pool: "du".to_string(),
                        field: "device_id".to_string(),
                    },
                },
                ColumnConfig {
                    name: "user_id".to_string(),
                    generator: GeneratorSpec::LinkedChoice {
                        pool: "du".to_string(),
                        field: "user_id".to_string(),
                    },
                },
            ],
        };

        write_generic_dataset(&make_cfg(tmp1.path().to_str().unwrap()), 55, None, &fk).unwrap();
        write_generic_dataset(&make_cfg(tmp2.path().to_str().unwrap()), 55, None, &fk).unwrap();

        let b1 = std::fs::read(tmp1.path().join("data.parquet")).unwrap();
        let b2 = std::fs::read(tmp2.path().join("data.parquet")).unwrap();
        assert_eq!(b1, b2, "same seed must produce byte-identical Parquet");
    }

    /// 7. Entity columns and linked_choice columns work independently.
    #[test]
    fn test_linked_choice_with_entity_columns() {
        let tmp = TempDir::new().unwrap();
        let output = tmp.path().to_str().unwrap().to_string();
        let fk = make_fk_counts(50, 50);

        let config = DatasetConfig {
            name: "events".to_string(),
            output,
            num_rows: 500,
            seed: Some(42),
            partition: None,
            entity: Some(EntityConfig {
                pool_ratio: 0.1,
                columns: vec![ColumnConfig {
                    name: "session_id".to_string(),
                    generator: GeneratorSpec::Uuid,
                }],
            }),
            linked_pools: Some(vec![make_two_fk_pool("du", 50, Some(9), 50, 50)]),
            columns: vec![
                ColumnConfig {
                    name: "event_id".to_string(),
                    generator: GeneratorSpec::SequentialId,
                },
                ColumnConfig {
                    name: "device_id".to_string(),
                    generator: GeneratorSpec::LinkedChoice {
                        pool: "du".to_string(),
                        field: "device_id".to_string(),
                    },
                },
                ColumnConfig {
                    name: "user_id".to_string(),
                    generator: GeneratorSpec::LinkedChoice {
                        pool: "du".to_string(),
                        field: "user_id".to_string(),
                    },
                },
            ],
        };

        let count = write_generic_dataset(&config, 42, None, &fk).unwrap();
        assert_eq!(count, 500);

        // Rebuild pool for validation.
        let pool_cfg = &config.linked_pools.as_ref().unwrap()[0];
        let pool = crate::generic::LinkedPool::new(9, pool_cfg, &fk);
        let valid_pairs: std::collections::HashSet<(i32, i32)> = pool
            .rows
            .iter()
            .map(|r| {
                let d = match &r[0] {
                    crate::generic::GenericValue::Int(v) => *v,
                    _ => panic!("Int"),
                };
                let u = match &r[1] {
                    crate::generic::GenericValue::Int(v) => *v,
                    _ => panic!("Int"),
                };
                (d, u)
            })
            .collect();

        let file = File::open(tmp.path().join("data.parquet")).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap()
            .build()
            .unwrap();

        // Column layout: session_id (entity), event_id, device_id, user_id
        let mut total = 0usize;
        let mut distinct_sessions: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for batch in reader {
            let batch = batch.unwrap();
            let session_arr = batch
                .column(0)
                .as_any()
                .downcast_ref::<arrow::array::StringArray>()
                .expect("session_id must be Utf8");
            let dev_arr = batch
                .column(2)
                .as_any()
                .downcast_ref::<arrow::array::Int32Array>()
                .expect("device_id must be Int32");
            let usr_arr = batch
                .column(3)
                .as_any()
                .downcast_ref::<arrow::array::Int32Array>()
                .expect("user_id must be Int32");
            for i in 0..batch.num_rows() {
                total += 1;
                distinct_sessions.insert(session_arr.value(i).to_string());
                let pair = (dev_arr.value(i), usr_arr.value(i));
                assert!(
                    valid_pairs.contains(&pair),
                    "row {total}: {pair:?} not in pool"
                );
            }
        }
        assert_eq!(total, 500);
        // With pool_ratio=0.1, expect ~50 distinct sessions across 500 rows.
        assert!(
            distinct_sessions.len() <= 50,
            "entity sessions should be sticky; got {} distinct",
            distinct_sessions.len()
        );
    }

    /// 8. Joint distribution smoke test: the four-shape pool produces the
    ///    expected co-occurrence shape mix. Concrete assertions (kept loose
    ///    so CI is robust under RNG variance):
    ///      - anonymous (user_id is null): within ±5 pp of the 25% shape weight.
    ///      - shared-device entries (device appears in ≥2 pool rows with
    ///        different users): > 5% of the pool — empirical floor from the
    ///        10% × emit:2 shape weight, weakened to account for collisions.
    ///      - multi-device-user entries (user appears in ≥2 pool rows with
    ///        different devices): > 2% of the pool — same reasoning over the
    ///        5% × emit:2 shape weight.
    ///
    /// Single-owner is not asserted directly because it's the residual
    /// category and any drift in the other three implicitly bounds it.
    #[test]
    fn test_linked_choice_joint_distribution_smoke() {
        let pool_size = 10_000usize;
        let mut fk = FkCounts::new();
        fk.insert("devices".to_string(), 100_000);
        fk.insert("users".to_string(), 100_000);

        // Build the four-shape pool from the spec example.
        let make_fk_field = |dataset: &str| GeneratorSpec::ForeignKey {
            dataset: dataset.to_string(),
        };
        let make_opt_null_field = |dataset: &str| GeneratorSpec::Optional {
            prob: 0.0,
            inner: Box::new(GeneratorSpec::ForeignKey {
                dataset: dataset.to_string(),
            }),
        };

        let mut fields_single = IndexMap::new();
        fields_single.insert("device_id".to_string(), make_fk_field("devices"));
        fields_single.insert("user_id".to_string(), make_fk_field("users"));

        let mut fields_anon = IndexMap::new();
        fields_anon.insert("device_id".to_string(), make_fk_field("devices"));
        fields_anon.insert("user_id".to_string(), make_opt_null_field("users"));

        let mut fields_shared_dev = IndexMap::new();
        fields_shared_dev.insert("device_id".to_string(), make_fk_field("devices"));
        fields_shared_dev.insert("user_id".to_string(), make_fk_field("users"));

        let mut fields_multi_dev = IndexMap::new();
        fields_multi_dev.insert("device_id".to_string(), make_fk_field("devices"));
        fields_multi_dev.insert("user_id".to_string(), make_fk_field("users"));

        let pool_cfg = LinkedPoolConfig {
            name: "du".to_string(),
            pool_size,
            seed: Some(42),
            shapes: vec![
                ShapeConfig {
                    weight: 0.60,
                    emit: 1,
                    sticky: vec![],
                    fields: fields_single,
                },
                ShapeConfig {
                    weight: 0.25,
                    emit: 1,
                    sticky: vec![],
                    fields: fields_anon,
                },
                ShapeConfig {
                    weight: 0.10,
                    emit: 2,
                    sticky: vec!["device_id".to_string()],
                    fields: fields_shared_dev,
                },
                ShapeConfig {
                    weight: 0.05,
                    emit: 2,
                    sticky: vec!["user_id".to_string()],
                    fields: fields_multi_dev,
                },
            ],
        };

        let pool = crate::generic::LinkedPool::new(42, &pool_cfg, &fk);
        assert_eq!(pool.len(), pool_size);

        // Count categories from pool snapshot.
        // device_idx=0, user_idx=1
        let device_counts: std::collections::HashMap<i32, usize> = {
            let mut m = std::collections::HashMap::new();
            for row in &pool.rows {
                if let crate::generic::GenericValue::Int(d) = &row[0] {
                    *m.entry(*d).or_insert(0) += 1;
                }
            }
            m
        };
        let user_counts: std::collections::HashMap<Option<i32>, usize> = {
            let mut m = std::collections::HashMap::new();
            for row in &pool.rows {
                let key = match &row[1] {
                    crate::generic::GenericValue::Int(u) => Some(*u),
                    crate::generic::GenericValue::Null => None,
                    _ => panic!("unexpected variant for user_id"),
                };
                *m.entry(key).or_insert(0) += 1;
            }
            m
        };

        // anonymous = rows where user_id IS NULL
        let anon_count = user_counts.get(&None).copied().unwrap_or(0);
        // shared device = devices that appear ≥2 times
        let shared_device_count: usize = device_counts.values().filter(|&&c| c >= 2).sum();
        // multi-device user = users that appear ≥2 times
        let non_null_user_counts: std::collections::HashMap<i32, usize> = user_counts
            .iter()
            .filter_map(|(k, v)| k.map(|u| (u, *v)))
            .collect();
        let multi_dev_user_count: usize = non_null_user_counts.values().filter(|&&c| c >= 2).sum();

        // Spec target ratios (approximate; shapes produce pool entries in these proportions):
        // - anonymous: ~25%
        // - shared-device entries: ~10% of pool (2 per draw × 10% weight)
        // - multi-device-user entries: ~5% of pool (2 per draw × 5% weight)
        // Allow ±5pp slack because small pool and emit:2 cause some variance.
        let anon_ratio = anon_count as f64 / pool_size as f64;
        let shared_ratio = shared_device_count as f64 / pool_size as f64;
        let multi_ratio = multi_dev_user_count as f64 / pool_size as f64;

        assert!(
            (anon_ratio - 0.25).abs() < 0.05,
            "anonymous ratio {anon_ratio:.3} not within 5pp of 0.25"
        );
        assert!(
            shared_ratio > 0.05,
            "shared-device ratio {shared_ratio:.3} should be > 5%"
        );
        assert!(
            multi_ratio > 0.02,
            "multi-device-user ratio {multi_ratio:.3} should be > 2%"
        );
    }
}
