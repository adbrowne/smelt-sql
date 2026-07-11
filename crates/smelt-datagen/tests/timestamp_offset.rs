//! TDD tests for the `timestamp_offset` generator
//! (`docs/specs/datagen.md` §"Generator types").

use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::record::RowAccessor;
use smelt_datagen::config::{
    ColumnConfig, DatagenConfig, DatasetConfig, FkCounts, GeneratorSpec, PartitionConfig,
};
use smelt_datagen::generic_parquet::write_generic_dataset;
use tempfile::TempDir;

fn make_config(output: &str, offset_seconds: i64) -> DatasetConfig {
    DatasetConfig {
        name: "events".to_string(),
        output: output.to_string(),
        num_rows: 200,
        seed: Some(42),
        partition: None,
        entity: None,
        linked_pools: None,
        redelivery: None,
        columns: vec![
            ColumnConfig {
                name: "event_time".to_string(),
                generator: GeneratorSpec::Timestamp {
                    start: "2026-01-01T00:00:00".to_string(),
                    end: "2026-01-02T00:00:00".to_string(),
                },
            },
            ColumnConfig {
                name: "arrival_time".to_string(),
                generator: GeneratorSpec::TimestampOffset {
                    base: "event_time".to_string(),
                    offset_seconds: Box::new(GeneratorSpec::Constant {
                        value: serde_yaml::Value::Number(offset_seconds.into()),
                    }),
                },
            },
        ],
    }
}

/// `timestamp_offset` emits `base + offset_seconds`, as an ISO 8601 string,
/// for every row.
#[test]
fn offset_adds_seconds_to_base_column() {
    let tmp = TempDir::new().expect("tempdir");
    let output = tmp.path().to_str().unwrap().to_string();
    let config = make_config(&output, 3600);

    let count = write_generic_dataset(&config, 42, None, &FkCounts::new())
        .expect("dataset generation should succeed");
    assert_eq!(count, 200);

    let file = std::fs::File::open(tmp.path().join("data.parquet")).expect("open parquet");
    let reader = SerializedFileReader::new(file).expect("read parquet");
    let schema = reader.metadata().file_metadata().schema();
    let fields = schema.get_fields();
    let event_idx = fields
        .iter()
        .position(|f| f.name() == "event_time")
        .expect("event_time column");
    let arrival_idx = fields
        .iter()
        .position(|f| f.name() == "arrival_time")
        .expect("arrival_time column");

    let mut checked = 0;
    for row_result in reader.get_row_iter(None).expect("row iter") {
        let row = row_result.expect("row");
        let event_str = row.get_string(event_idx).expect("event_time is Utf8");
        let arrival_str = row.get_string(arrival_idx).expect("arrival_time is Utf8");
        let event_dt = chrono::NaiveDateTime::parse_from_str(event_str, "%Y-%m-%dT%H:%M:%S")
            .unwrap_or_else(|e| panic!("event_time {event_str:?} not ISO 8601: {e}"));
        let arrival_dt = chrono::NaiveDateTime::parse_from_str(arrival_str, "%Y-%m-%dT%H:%M:%S")
            .unwrap_or_else(|e| panic!("arrival_time {arrival_str:?} not ISO 8601: {e}"));
        assert_eq!(
            arrival_dt,
            event_dt + chrono::Duration::seconds(3600),
            "arrival_time must equal event_time + offset_seconds (3600)"
        );
        checked += 1;
    }
    assert!(checked > 0, "no rows were read back");
}

/// `timestamp_offset` with `base:` naming the dataset's partition column
/// anchors to midnight of that row's own partition date, plus
/// `offset_seconds` — so `DATE(event_time)` always equals the partition
/// value, even though the partition column is injected after regular
/// columns are generated (`docs/specs/datagen.md` §"Generator types" /
/// §"Partitioning").
#[test]
fn partition_anchored_base_uses_partition_date() {
    let tmp = TempDir::new().expect("tempdir");
    let output = tmp.path().to_str().unwrap().to_string();

    let config = DatasetConfig {
        name: "events".to_string(),
        output,
        num_rows: 200,
        seed: Some(42),
        partition: Some(PartitionConfig {
            column: "event_date".to_string(),
            start: "2026-01-01".to_string(),
            days: 5,
        }),
        entity: None,
        linked_pools: None,
        redelivery: None,
        columns: vec![ColumnConfig {
            name: "event_time".to_string(),
            generator: GeneratorSpec::TimestampOffset {
                base: "event_date".to_string(),
                offset_seconds: Box::new(GeneratorSpec::UniformInt { min: 0, max: 86399 }),
            },
        }],
    };

    let count = write_generic_dataset(&config, 42, None, &FkCounts::new())
        .expect("dataset generation should succeed");
    assert_eq!(count, 200);

    let mut checked = 0;
    for entry in std::fs::read_dir(tmp.path()).expect("read output dir") {
        let dir = entry.expect("dir entry").path();
        if !dir.is_dir() {
            continue;
        }
        let name = dir.file_name().unwrap().to_str().unwrap().to_string();
        let date_str = name
            .strip_prefix("event_date=")
            .unwrap_or_else(|| panic!("unexpected partition dir name: {name}"));
        let partition_date = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
            .unwrap_or_else(|e| panic!("partition dir {date_str:?} not YYYY-MM-DD: {e}"));

        let file = std::fs::File::open(dir.join("data.parquet")).expect("open parquet");
        let reader = SerializedFileReader::new(file).expect("read parquet");
        let schema = reader.metadata().file_metadata().schema();
        let fields = schema.get_fields();
        let event_idx = fields
            .iter()
            .position(|f| f.name() == "event_time")
            .expect("event_time column");

        for row_result in reader.get_row_iter(None).expect("row iter") {
            let row = row_result.expect("row");
            let event_str = row.get_string(event_idx).expect("event_time is Utf8");
            let event_dt = chrono::NaiveDateTime::parse_from_str(event_str, "%Y-%m-%dT%H:%M:%S")
                .unwrap_or_else(|e| panic!("event_time {event_str:?} not ISO 8601: {e}"));
            assert_eq!(
                event_dt.date(),
                partition_date,
                "DATE(event_time) ({}) must equal the partition date ({date_str})",
                event_dt.date(),
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "no rows were read back");
}

/// A `timestamp_offset` whose `base:` names a later column, an undeclared
/// column, or a non-timestamp-producing column is a configuration error —
/// never silently accepted (fail-loud discipline, `CLAUDE.md`).
#[test]
fn base_must_be_earlier_timestamp_column() {
    use smelt_datagen::config::validate_config;

    // Case 1: base column is declared AFTER the timestamp_offset column.
    let later_ref_config = DatagenConfig {
        seed: Some(42),
        scale_factor: None,
        datasets: vec![DatasetConfig {
            name: "events".to_string(),
            output: "unused".to_string(),
            num_rows: 10,
            seed: Some(42),
            partition: None,
            entity: None,
            linked_pools: None,
            redelivery: None,
            columns: vec![
                ColumnConfig {
                    name: "arrival_time".to_string(),
                    generator: GeneratorSpec::TimestampOffset {
                        base: "event_time".to_string(),
                        offset_seconds: Box::new(GeneratorSpec::Constant {
                            value: serde_yaml::Value::Number(0.into()),
                        }),
                    },
                },
                ColumnConfig {
                    name: "event_time".to_string(),
                    generator: GeneratorSpec::Timestamp {
                        start: "2026-01-01T00:00:00".to_string(),
                        end: "2026-01-02T00:00:00".to_string(),
                    },
                },
            ],
        }],
    };
    let err = validate_config(&later_ref_config)
        .expect_err("timestamp_offset referencing a later column must be a config error");
    assert!(
        err.contains("event_time"),
        "error should name the offending base column; got: {err}"
    );

    // Case 2: base column exists but is not a timestamp-producing generator.
    let non_timestamp_config = DatagenConfig {
        seed: Some(42),
        scale_factor: None,
        datasets: vec![DatasetConfig {
            name: "events".to_string(),
            output: "unused".to_string(),
            num_rows: 10,
            seed: Some(42),
            partition: None,
            entity: None,
            linked_pools: None,
            redelivery: None,
            columns: vec![
                ColumnConfig {
                    name: "event_id".to_string(),
                    generator: GeneratorSpec::SequentialId,
                },
                ColumnConfig {
                    name: "arrival_time".to_string(),
                    generator: GeneratorSpec::TimestampOffset {
                        base: "event_id".to_string(),
                        offset_seconds: Box::new(GeneratorSpec::Constant {
                            value: serde_yaml::Value::Number(0.into()),
                        }),
                    },
                },
            ],
        }],
    };
    let err = validate_config(&non_timestamp_config)
        .expect_err("timestamp_offset base must be a timestamp-producing generator");
    assert!(
        err.contains("event_id"),
        "error should name the offending base column; got: {err}"
    );

    // Case 3: base column does not exist at all.
    let missing_config = DatagenConfig {
        seed: Some(42),
        scale_factor: None,
        datasets: vec![DatasetConfig {
            name: "events".to_string(),
            output: "unused".to_string(),
            num_rows: 10,
            seed: Some(42),
            partition: None,
            entity: None,
            linked_pools: None,
            redelivery: None,
            columns: vec![ColumnConfig {
                name: "arrival_time".to_string(),
                generator: GeneratorSpec::TimestampOffset {
                    base: "does_not_exist".to_string(),
                    offset_seconds: Box::new(GeneratorSpec::Constant {
                        value: serde_yaml::Value::Number(0.into()),
                    }),
                },
            }],
        }],
    };
    let err = validate_config(&missing_config)
        .expect_err("timestamp_offset base naming an undeclared column must be a config error");
    assert!(
        err.contains("does_not_exist"),
        "error should name the missing base column; got: {err}"
    );
}
