//! TDD tests for `build_source_timeseries_map` (Phase 2 / BUG-072).
//!
//! These are RED before the helper is extracted and source YAML timeseries
//! entries are merged into the map.

use smelt_core::config::{Granularity, TimeseriesConfig};
use smelt_core::graph::DependencyGraph;
use smelt_core::SourceInfo;
use smelt_runtime::build_source_timeseries_map;

fn day_ts(partition_column: &str) -> TimeseriesConfig {
    TimeseriesConfig {
        event_time_column: format!("{}_ts", partition_column),
        partition_column: partition_column.to_string(),
        granularity: Granularity::Day,
        week_start: None,
        assert_monotonic: false,
    }
}

fn make_source(segments: Vec<&str>, ts: Option<TimeseriesConfig>) -> SourceInfo {
    SourceInfo {
        path: std::path::PathBuf::from("/tmp/fake.yml"),
        address_segments: segments.into_iter().map(String::from).collect(),
        columns: vec![],
        description: None,
        name_override: None,
        tags: vec![],
        timeseries: ts,
        mutation_profile: None,
        source_lateness: None,
        watermark: None,
        unique_key: None,
        retention: None,
        referential_integrity: None,
    }
}

// ── Test 1: source YAML timeseries appears in map ─────────────────────────────
//
// RED today: `build_source_timeseries_map` does not exist yet.
// After Phase 2 it must be `pub` and include SourceInfo timeseries entries.

#[test]
fn source_yaml_timeseries_in_map() {
    let graph = DependencyGraph::build(vec![], None).expect("empty graph");
    let source = make_source(vec!["sources", "raw", "events"], Some(day_ts("event_date")));
    let map = build_source_timeseries_map(&graph, &[source]);
    let entry = map
        .get("smelt.sources.raw.events")
        .expect("source YAML timeseries must appear in map at its smelt path");
    assert_eq!(entry.partition_column, "event_date");
}

// ── Test 2: source without timeseries does not appear in map ─────────────────

#[test]
fn source_without_timeseries_not_in_map() {
    let graph = DependencyGraph::build(vec![], None).expect("empty graph");
    let source = make_source(vec!["sources", "raw", "users"], None);
    let map = build_source_timeseries_map(&graph, &[source]);
    assert!(
        !map.contains_key("smelt.sources.raw.users"),
        "source without timeseries must not appear in map"
    );
}

// ── Test 3: model frontmatter timeseries is preserved (regression guard) ──────
//
// Phase 2 must not drop the existing model-frontmatter behaviour.

#[test]
fn model_frontmatter_timeseries_preserved() {
    use smelt_core::{ModelId, ModelKind, ModelMetadata};

    let sql =
        "---\ntimeseries:\n  event_time_column: event_date\n  partition_column: event_date\n  granularity: day\n---\nSELECT 1";
    let td = tempfile::tempdir().unwrap();
    let path = td.path().join("events.sql");
    std::fs::write(&path, sql).unwrap();

    let model = smelt_core::ModelFile {
        name: "events".to_string(),
        path: path.clone(),
        content: sql.to_string(),
        refs: vec![],
        parse_errors: vec![],
        metadata: Some(Box::new(ModelMetadata {
            timeseries: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            ..Default::default()
        })),
        kind: ModelKind::Sql,
        model_id: ModelId::from_path(path),
        address_segments: vec!["events".to_string()],
    };

    let graph = DependencyGraph::build(vec![model], None).expect("graph with one model");
    let map = build_source_timeseries_map(&graph, &[]);
    let entry = map
        .get("smelt.events")
        .expect("model frontmatter timeseries must still appear in map after Phase 2");
    assert_eq!(entry.partition_column, "event_date");
}
