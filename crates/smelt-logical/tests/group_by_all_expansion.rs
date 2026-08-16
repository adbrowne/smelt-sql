//! `GROUP BY ALL` (DuckDB) must be analyzed with its real grouping keys — the
//! non-aggregate select items — everywhere smelt-logical consumes grouping
//! information, so a `GROUP BY ALL` model is analyzed identically to its
//! explicit `GROUP BY <non-aggregate items>` twin.
//!
//! Regression coverage for two defects: the AST walk path
//! (`resolve_scope_group_by`) previously returned an empty key set for
//! `GROUP BY ALL` (the walk could not distinguish it from no grouping), and
//! the text-scan path (`extract_group_by_from_text`, via `analyze_select`)
//! previously returned a phantom `["ALL"]` key that fed cumulative/incremental
//! rules a wrong `unique_key` and spurious `KeyedUnknownCombiner`s.

use smelt_core::config::{Granularity, TimeseriesConfig};
use smelt_logical::analysis::analyze_select;
use smelt_logical::analysis::walk::{enumerate_scopes, Scope, ScopeKind};
use smelt_logical::rules::cumulative::{classify_cumulative, KeyedDiagnostic, SourceTimeseriesMap};

fn group_by_scope(sql: &str) -> Option<Scope> {
    enumerate_scopes(sql)
        .expect("enumerable")
        .scopes
        .into_iter()
        .find(|s| s.kind == ScopeKind::GroupBy)
}

// ===== Walk / analysis twin equivalence =====

#[test]
fn walk_group_by_all_matches_explicit_twin() {
    let all = group_by_scope("SELECT k, SUM(v) AS s FROM t GROUP BY ALL");
    let explicit = group_by_scope("SELECT k, SUM(v) AS s FROM t GROUP BY k");

    assert!(
        all.is_some(),
        "GROUP BY ALL must produce a GroupBy scope, not be lost as no-grouping"
    );
    assert_eq!(
        all.as_ref().map(|s| &s.keys),
        explicit.as_ref().map(|s| &s.keys),
        "GROUP BY ALL scope keys must match the explicit GROUP BY k twin"
    );
    assert_eq!(all.expect("group by scope").keys, vec!["k".to_string()]);
}

#[test]
fn walk_group_by_all_excludes_window_function_select_item() {
    // A bare window-function projection is neither an aggregate nor a plain
    // grouping expression; DuckDB never treats it as a GROUP BY ALL key.
    let all = group_by_scope(
        "SELECT k, SUM(v) AS s, ROW_NUMBER() OVER (ORDER BY k) AS rn FROM t GROUP BY ALL",
    );
    let explicit = group_by_scope(
        "SELECT k, SUM(v) AS s, ROW_NUMBER() OVER (ORDER BY k) AS rn FROM t GROUP BY k",
    );

    assert!(
        all.is_some(),
        "GROUP BY ALL must still produce a GroupBy scope"
    );
    assert_eq!(
        all.as_ref().map(|s| &s.keys),
        explicit.as_ref().map(|s| &s.keys),
        "GROUP BY ALL scope keys must match the explicit GROUP BY k twin"
    );
    assert_eq!(
        all.expect("group by scope").keys,
        vec!["k".to_string()],
        "the window-function select item must not become a grouping key"
    );
}

#[test]
fn walk_group_by_all_multi_key_matches_explicit_twin() {
    let all = group_by_scope("SELECT a, b, COUNT(*) AS n FROM t GROUP BY ALL");
    let explicit = group_by_scope("SELECT a, b, COUNT(*) AS n FROM t GROUP BY a, b");
    assert_eq!(
        all.as_ref().map(|s| &s.keys),
        explicit.as_ref().map(|s| &s.keys)
    );
    assert_eq!(
        all.expect("group by scope").keys,
        vec!["a".to_string(), "b".to_string()]
    );
}

#[test]
fn walk_group_by_all_aggregates_only_is_single_group_no_phantom_key() {
    // No non-aggregate select items: GROUP BY ALL degenerates to a single
    // group. There must be no phantom "ALL" key anywhere.
    let scopes = enumerate_scopes("SELECT SUM(v) AS s FROM t GROUP BY ALL")
        .expect("enumerable")
        .scopes;
    assert!(
        scopes.iter().all(|s| !s.keys.iter().any(|k| k == "ALL")),
        "no scope may carry a phantom ALL key: {scopes:?}"
    );
    // Single-group semantics: no non-aggregate grouping key => no GroupBy
    // scope (identical to a plain aggregate without GROUP BY).
    assert!(
        !scopes.iter().any(|s| s.kind == ScopeKind::GroupBy),
        "aggregates-only GROUP BY ALL is single-group, no keyed GroupBy scope: {scopes:?}"
    );
}

// ===== analyze_select (text-scan path) twin equivalence =====

#[test]
fn analyze_select_group_by_all_expands_to_non_aggregate_items() {
    let all = analyze_select("SELECT k, SUM(v) AS s FROM t GROUP BY ALL").expect("parse");
    let explicit = analyze_select("SELECT k, SUM(v) AS s FROM t GROUP BY k").expect("parse");
    assert_eq!(all.group_by_exprs, explicit.group_by_exprs);
    assert_eq!(all.group_by_exprs, vec!["k".to_string()]);
    assert!(
        !all.group_by_exprs.iter().any(|e| e == "ALL"),
        "no phantom ALL key: {:?}",
        all.group_by_exprs
    );
}

// ===== Cumulative rule twin equivalence =====

fn ts(col: &str) -> TimeseriesConfig {
    TimeseriesConfig {
        event_time_column: col.to_string(),
        partition_column: col.to_string(),
        granularity: Granularity::Day,
        week_start: None,
        assert_monotonic: false,
    }
}

fn events_source_map() -> SourceTimeseriesMap {
    let mut m = SourceTimeseriesMap::new();
    m.insert("smelt.silver.events_parsed".to_string(), ts("event_date"));
    m
}

#[test]
fn cumulative_group_by_all_derives_same_unique_key_no_spurious_combiner() {
    let refs = vec!["smelt.silver.events_parsed".to_string()];

    let explicit_sql = "SELECT device_id, user_id, COUNT(*) AS event_count \
        FROM smelt.silver.events_parsed WHERE user_id IS NOT NULL \
        GROUP BY device_id, user_id";
    let all_sql = "SELECT device_id, user_id, COUNT(*) AS event_count \
        FROM smelt.silver.events_parsed WHERE user_id IS NOT NULL \
        GROUP BY ALL";

    let explicit = classify_cumulative(explicit_sql, &refs, &events_source_map(), false, &[])
        .expect("explicit GROUP BY twin must classify");
    let all = classify_cumulative(all_sql, &refs, &events_source_map(), false, &[])
        .expect("GROUP BY ALL twin must classify identically — no phantom ALL key");

    assert_eq!(
        all.unique_key, explicit.unique_key,
        "GROUP BY ALL unique_key must match explicit twin"
    );
    assert_eq!(
        all.unique_key,
        vec!["device_id".to_string(), "user_id".to_string()],
        "unique_key must be the real keys, not [\"ALL\"]"
    );
    assert!(
        !all.unique_key.iter().any(|k| k == "ALL"),
        "no phantom ALL key in unique_key: {:?}",
        all.unique_key
    );
}

#[test]
fn cumulative_group_by_all_no_spurious_unknown_combiner() {
    // Directly guard against the spurious KeyedUnknownCombiner the phantom
    // ["ALL"] key produced: with ALL expanded, the key columns are recognised
    // as keys, not flagged as unknown combiners.
    let refs = vec!["smelt.silver.events_parsed".to_string()];
    let sql = "SELECT device_id, user_id, COUNT(*) AS event_count \
        FROM smelt.silver.events_parsed WHERE user_id IS NOT NULL \
        GROUP BY ALL";
    match classify_cumulative(sql, &refs, &events_source_map(), false, &[]) {
        Ok(_) => {}
        Err(diags) => {
            assert!(
                !diags
                    .iter()
                    .any(|d| matches!(d, KeyedDiagnostic::KeyedUnknownCombiner { .. })),
                "GROUP BY ALL must not emit spurious KeyedUnknownCombiner: {diags:?}"
            );
        }
    }
}

// ===== Real fixture characterization =====

#[test]
fn timeseries_user_activity_fixture_expands_group_by_all() {
    // examples/timeseries/models/user_activity.sql uses GROUP BY ALL with four
    // non-aggregate items and one aggregate (COUNT). The analysis must expand
    // to the four non-aggregate expressions with no phantom ALL key.
    let sql = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/timeseries/models/user_activity.sql"
    ))
    .expect("fixture readable");
    let analysis = analyze_select(&sql).expect("fixture parses");
    assert!(
        !analysis.group_by_exprs.iter().any(|e| e == "ALL"),
        "no phantom ALL key in fixture: {:?}",
        analysis.group_by_exprs
    );
    assert_eq!(
        analysis.group_by_exprs.len(),
        4,
        "four non-aggregate select items expand as grouping keys: {:?}",
        analysis.group_by_exprs
    );
    assert!(
        analysis
            .group_by_exprs
            .iter()
            .any(|e| e.contains("TRY_CAST")),
        "the TRY_CAST non-aggregate projection is a grouping key: {:?}",
        analysis.group_by_exprs
    );
}
