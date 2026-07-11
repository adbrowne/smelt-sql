//! TDD tests for the `redelivery:` block
//! (`docs/specs/datagen.md` §"Redelivery (duplicate emission)").

use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::record::RowAccessor;
use smelt_datagen::config::{
    ColumnConfig, DatasetConfig, FkCounts, GeneratorSpec, RedeliveryConfig,
};
use smelt_datagen::generic_parquet::write_generic_dataset;
use std::collections::HashMap;
use std::path::Path;
use tempfile::TempDir;

fn make_config(output: &str, redelivery: Option<RedeliveryConfig>) -> DatasetConfig {
    DatasetConfig {
        name: "events".to_string(),
        output: output.to_string(),
        num_rows: 1000,
        seed: Some(7),
        partition: None,
        entity: None,
        linked_pools: None,
        redelivery,
        columns: vec![
            ColumnConfig {
                name: "event_id".to_string(),
                generator: GeneratorSpec::SequentialId,
            },
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
                        value: serde_yaml::Value::Number(0.into()),
                    }),
                },
            },
        ],
    }
}

fn make_redelivery() -> RedeliveryConfig {
    RedeliveryConfig {
        fraction: 0.02,
        arrival_column: "arrival_time".to_string(),
        delay_seconds: GeneratorSpec::UniformInt {
            min: 3600,
            max: 7201, // [min, max) → inclusive of 7200
        },
    }
}

fn read_rows(path: &Path) -> Vec<(i32, String, String)> {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("open {path:?}: {e}"));
    let reader =
        SerializedFileReader::new(file).unwrap_or_else(|e| panic!("read parquet {path:?}: {e}"));
    let schema = reader.metadata().file_metadata().schema();
    let fields = schema.get_fields();
    let id_idx = fields
        .iter()
        .position(|f| f.name() == "event_id")
        .expect("event_id column");
    let et_idx = fields
        .iter()
        .position(|f| f.name() == "event_time")
        .expect("event_time column");
    let at_idx = fields
        .iter()
        .position(|f| f.name() == "arrival_time")
        .expect("arrival_time column");

    reader
        .get_row_iter(None)
        .expect("row iter")
        .map(|r| {
            let row = r.expect("row");
            (
                row.get_int(id_idx).expect("event_id must be non-null"),
                row.get_string(et_idx)
                    .expect("event_time must be non-null")
                    .clone(),
                row.get_string(at_idx)
                    .expect("arrival_time must be non-null")
                    .clone(),
            )
        })
        .collect()
}

/// Redelivered rows are byte-identical to their originals — same
/// identifiers, same occurrence timestamps — except `arrival_column`, which
/// is shifted forward by a per-duplicate draw from `delay_seconds`. Count
/// equals `round(fraction × num_rows)`, and duplicates are appended after the
/// primary rows.
#[test]
fn duplicates_identical_except_arrival_column() {
    let tmp = TempDir::new().expect("tempdir");
    let output = tmp.path().to_str().unwrap().to_string();
    let config = make_config(&output, Some(make_redelivery()));

    let count =
        write_generic_dataset(&config, 7, None, &FkCounts::new()).expect("dataset generation");
    assert_eq!(
        count, 1020,
        "primary (1000) + round(0.02 * 1000) = 20 duplicates"
    );

    let rows = read_rows(&tmp.path().join("data.parquet"));
    assert_eq!(rows.len(), 1020);

    let primary = &rows[0..1000];
    let duplicates = &rows[1000..1020];

    let primary_by_id: HashMap<i32, &(i32, String, String)> =
        primary.iter().map(|r| (r.0, r)).collect();

    for dup in duplicates {
        let orig = primary_by_id
            .get(&dup.0)
            .unwrap_or_else(|| panic!("duplicate event_id {} has no matching primary row", dup.0));
        assert_eq!(orig.1, dup.1, "event_time must be identical on redelivery");
        assert_ne!(
            orig.2, dup.2,
            "arrival_time must be shifted forward on redelivery"
        );

        let orig_arrival =
            chrono::NaiveDateTime::parse_from_str(&orig.2, "%Y-%m-%dT%H:%M:%S").unwrap();
        let dup_arrival =
            chrono::NaiveDateTime::parse_from_str(&dup.2, "%Y-%m-%dT%H:%M:%S").unwrap();
        let delta = (dup_arrival - orig_arrival).num_seconds();
        assert!(
            (3600..=7200).contains(&delta),
            "redelivery delay {delta}s out of the configured [3600, 7200] range"
        );
    }
}

/// Toggling `redelivery:` must leave the primary rows byte-identical: the
/// selection/delay draws use a dedicated RNG stream seeded
/// `dataset_seed.wrapping_add(200)`, never the primary row stream.
#[test]
fn redelivery_does_not_perturb_primary_rows() {
    let tmp_off = TempDir::new().expect("tempdir");
    let output_off = tmp_off.path().to_str().unwrap().to_string();
    let config_off = make_config(&output_off, None);
    let count_off = write_generic_dataset(&config_off, 7, None, &FkCounts::new())
        .expect("generation (no redelivery)");
    assert_eq!(count_off, 1000);
    let rows_off = read_rows(&tmp_off.path().join("data.parquet"));

    let tmp_on = TempDir::new().expect("tempdir");
    let output_on = tmp_on.path().to_str().unwrap().to_string();
    let config_on = make_config(&output_on, Some(make_redelivery()));
    let count_on = write_generic_dataset(&config_on, 7, None, &FkCounts::new())
        .expect("generation (redelivery)");
    assert!(count_on > count_off);
    let rows_on = read_rows(&tmp_on.path().join("data.parquet"));

    assert_eq!(rows_off.len(), 1000);
    let primary_on = &rows_on[0..1000];
    assert_eq!(
        &rows_off, primary_on,
        "toggling redelivery must not perturb the primary rows (dedicated +200 RNG stream)"
    );
}
