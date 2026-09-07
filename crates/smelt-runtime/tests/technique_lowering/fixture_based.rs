use super::*;

/// Real fixture: `examples/timeseries/models/daily_events_enriched.sql`
/// (fact `raw.events` × dimension `raw.users`, the latter declared
/// `mutation_profile: mutable_snapshot`) reads `raw.users` in its `ON
/// e.user_id = u.user_id` join predicate — a row-admission read, so the
/// `{user_name}` group is membership-sensitive to `raw.users`
/// (`docs/plans/20260808-membership-sensitivity.md` Phase 1), not merely
/// value-sensitive. This derives the SAME `MaintenancePlan` `smelt explain`
/// reports (`smelt-db::maintenance_plan_report`), reading the model +
/// source YAML straight off disk with no Salsa layer, and asserts the
/// `{user_name}` group's `UpstreamMutation { source: "raw.users" }` cell is
/// admitted with `Technique::DeleteInsert` (`Corner::RecomputeRegion`), never
/// `Technique::ColumnScopedMerge` — a membership-sensitive group "must be
/// repaired by a technique that can create and delete rows"
/// (`incremental_models.md` §"The plan matrix"), which a column-scoped
/// `MERGE` cannot do. `example_diagnostics` (`crates/smelt-cli/tests/`) is
/// the standing gate that this fixture carries no diagnostics.
#[test]
fn real_fixture_examples_timeseries_admits_membership_recompute_cell() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let model_path = project_dir.join("models/daily_events_enriched.sql");
    let text = std::fs::read_to_string(&model_path).expect("read daily_events_enriched.sql");

    let smelt_core::FileMetadata::Single {
        metadata,
        sql_offset,
    } = smelt_core::extract_file_metadata(&text).expect("parse frontmatter")
    else {
        panic!("daily_events_enriched.sql must be a single-model file");
    };
    let sql_body = &text[sql_offset..];

    let config = smelt_core::Config::load(&project_dir).expect("load smelt.yml");
    let source_infos = smelt_core::discover_source_infos(&project_dir, &config.paths);

    let refs = smelt_logical::collect_path_refs(sql_body);
    let source_refs: Vec<(String, Option<smelt_core::SourceInfo>)> = refs
        .iter()
        .filter_map(|r| {
            let stripped = r.strip_prefix("smelt.")?;
            let segs: Vec<String> = stripped.split('.').map(String::from).collect();
            let info = source_infos
                .iter()
                .find(|s| s.address_segments == segs)?
                .clone();
            let bare = stripped.strip_prefix("sources.").unwrap_or(stripped);
            Some((bare.to_string(), Some(info)))
        })
        .collect();

    let model_scan_bounds = metadata
        .maintenance
        .as_ref()
        .and_then(|m| m.scan_bounds.as_ref());
    let (sources, _scan_bounds_warnings) =
        smelt_db::queries::maintenance::build_source_facts(&source_refs, model_scan_bounds, None);
    let explicitly_mutable: std::collections::HashSet<String> = source_refs
        .iter()
        .filter(|(_, info)| {
            info.as_ref().is_some_and(|i| {
                i.mutation_profile
                    .as_ref()
                    .is_some_and(|m| m.kind == smelt_core::sources::MutationProfile::Mutable)
            })
        })
        .map(|(name, _)| name.clone())
        .collect();

    let result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        sql_body,
        "daily_events_enriched",
        &metadata,
        &sources,
        &explicitly_mutable,
        None,
        &[],
        &[],
        &std::collections::BTreeMap::new(),
        None,
        None,
        &[],
    )
    .expect("daily_events_enriched has a maintenance plan (refresh: incremental + grain set)");

    assert!(
        result.plan.refusals.is_empty(),
        "expected no admission refusals: {:?}",
        result.plan.refusals
    );

    let mutation_trigger = Trigger::UpstreamMutation {
        source: "raw.users".to_string(),
    };
    // Membership sensitivity is a row-admission property of the WHOLE join,
    // so every column group the fact+dimension join admits (not only
    // `{user_name}`) now carries its own `UpstreamMutation { raw.users }`
    // cell — `MaintenancePlan::cell_for`'s "first matching trigger" lookup
    // is only safe when a trigger has a single admitted cell, which no
    // longer holds here. Find the `{user_name}` cell explicitly rather than
    // relying on `cell_for`'s arbitrary first-match order (a real, pre-
    // existing `cell_for` API limitation this phase's derivation exposes,
    // out of this phase's critical files — `crates/smelt-logical/src/
    // maintenance/mod.rs` is not touched by Phase 2 — flagged for a
    // follow-up rather than silently worked around).
    let cell = result
        .plan
        .cells
        .iter()
        .find(|c| c.trigger == mutation_trigger && c.group == "{user_name}")
        .unwrap_or_else(|| {
            panic!(
                "no {{user_name}} cell admitted for {mutation_trigger:?}: {:#?}",
                result.plan
            )
        });
    assert_eq!(
        cell.technique,
        Technique::DeleteInsert,
        "a membership-sensitive dimension-mutation cell must admit the recompute family, never \
         column-scoped MERGE"
    );
    assert_eq!(cell.corner, Corner::RecomputeRegion);
}

/// Real fixture, the `PartitionLocal::Yes` corner:
/// `examples/timeseries/models/daily_events_status.sql` (fact `raw.events` ×
/// a CLOCKED, mutable dimension `raw.user_status`, joined on an explicit
/// `changed_at BETWEEN event_timestamp - INTERVAL '1 day' AND
/// event_timestamp + INTERVAL '1 day'` predicate) derives a genuine
/// `ScanClamp` for `raw.user_status` — unlike `daily_events_enriched.sql`'s
/// unclocked `raw.users`, which only ever derives the accepted-full-scan
/// corner (`PartitionLocal::No`).
///
/// Obtains the plan from the production wrapper
/// (`smelt_db::queries::maintenance::derive_model_maintenance_plan`), which
/// now derives its trigger list via the pure, clock-blind
/// `smelt_logical::maintenance::derive::derive_triggers`
/// (`docs/outcomes/20260815-definition-delta-migrate` phase 19) — a clocked
/// explicitly-mutable source gets an `UpstreamMutation` cell exactly like an
/// unclocked one, so this corner is reachable through the real production
/// path, not only by hand-building a fuller trigger list than the wrapper
/// constructs.
#[test]
fn real_fixture_daily_events_status_admits_partition_local_yes_cell() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let model_path = project_dir.join("models/daily_events_status.sql");
    let text = std::fs::read_to_string(&model_path).expect("read daily_events_status.sql");

    let smelt_core::FileMetadata::Single {
        metadata,
        sql_offset,
    } = smelt_core::extract_file_metadata(&text).expect("parse frontmatter")
    else {
        panic!("daily_events_status.sql must be a single-model file");
    };
    let sql_body = &text[sql_offset..];

    let config = smelt_core::Config::load(&project_dir).expect("load smelt.yml");
    let source_infos = smelt_core::discover_source_infos(&project_dir, &config.paths);

    let refs = smelt_logical::collect_path_refs(sql_body);
    let source_refs: Vec<(String, Option<smelt_core::SourceInfo>)> = refs
        .iter()
        .filter_map(|r| {
            let stripped = r.strip_prefix("smelt.")?;
            let segs: Vec<String> = stripped.split('.').map(String::from).collect();
            let info = source_infos
                .iter()
                .find(|s| s.address_segments == segs)?
                .clone();
            let bare = stripped.strip_prefix("sources.").unwrap_or(stripped);
            Some((bare.to_string(), Some(info)))
        })
        .collect();

    let model_scan_bounds = metadata
        .maintenance
        .as_ref()
        .and_then(|m| m.scan_bounds.as_ref());
    let (sources, _scan_bounds_warnings) =
        smelt_db::queries::maintenance::build_source_facts(&source_refs, model_scan_bounds, None);
    let explicitly_mutable: std::collections::HashSet<String> = source_refs
        .iter()
        .filter(|(_, info)| {
            info.as_ref().is_some_and(|i| {
                i.mutation_profile
                    .as_ref()
                    .is_some_and(|m| m.kind == smelt_core::sources::MutationProfile::Mutable)
            })
        })
        .map(|(name, _)| name.clone())
        .collect();

    let result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        sql_body,
        "daily_events_status",
        &metadata,
        &sources,
        &explicitly_mutable,
        None,
        &[],
        &[],
        &std::collections::BTreeMap::new(),
        None,
        None,
        &[],
    )
    .expect("daily_events_status has a maintenance plan (refresh: incremental + grain set)");
    let plan = result.plan;

    assert!(
        plan.refusals.is_empty(),
        "expected no admission refusals: {:?}",
        plan.refusals
    );

    let mutation_trigger = Trigger::UpstreamMutation {
        source: "raw.user_status".to_string(),
    };
    // See `real_fixture_examples_timeseries_admits_membership_recompute_cell`'s
    // comment above: membership sensitivity now spreads the same trigger
    // over every admitted column group, so `cell_for`'s first-match lookup
    // is not safe here either — find the `{status}` cell explicitly.
    let cell = plan
        .cells
        .iter()
        .find(|c| c.trigger == mutation_trigger && c.group == "{status}")
        .unwrap_or_else(|| {
            panic!("no {{status}} cell admitted for {mutation_trigger:?}: {plan:#?}")
        });
    assert_eq!(
        cell.technique,
        Technique::DeleteInsert,
        "raw.user_status is read in the join's ON predicate (both `s.user_id` and \
         `s.changed_at`) — a row-admission read — so the {{status}} group is \
         membership-sensitive and must admit the recompute family, never column-scoped MERGE"
    );
    assert_eq!(cell.corner, Corner::RecomputeRegion);
    assert_eq!(
        cell.partition_local,
        PartitionLocal::Yes,
        "raw.user_status is clocked with an explicit, derivable window predicate — this must \
         be the genuine scan-clamp corner, not the accepted-full-scan corner \
         daily_events_enriched.sql exercises. The clocked-source scan-locality derivation is \
         orthogonal to the family (value vs membership) sensitivity this phase adds — a \
         membership-sensitive cell still carries its own derived scan bound, even though \
         today's runtime dispatch for it (whole-model recompute) does not yet consume it."
    );
    let scan = cell
        .scans
        .iter()
        .find(|s| s.source == "raw.user_status")
        .unwrap_or_else(|| panic!("no scan clamp for 'raw.user_status': {:?}", cell.scans));
    assert_eq!(scan.column, "changed_at");

    // The mechanism this fixture feeds is unit-tested directly against
    // `dimension_join_contribution` (`maintenance_driver_tests` below) and
    // exercised end-to-end against a real DuckDB backend in
    // `yes_corner_matches_full_refresh_after_dimension_mutation`.
    let dimension_unique_key = source_infos
        .iter()
        .find(|s| s.address_segments == ["sources", "raw", "user_status"])
        .and_then(|s| s.unique_key.clone())
        .unwrap_or_default();
    assert_eq!(dimension_unique_key, vec!["user_id".to_string()]);
    let contribution = smelt_runtime::maintenance_driver::dimension_join_contribution(
        sql_body,
        "raw.user_status",
        &dimension_unique_key,
    );
    assert!(
        contribution.is_monotone(),
        "the fact->dimension join must be provable one-to-one: {contribution:?}"
    );
}

/// Phase 1 (`docs/plans/20260809-keyed-frontier.md`): the order-monotone
/// overwrite family's (`MAX_BY`/`MIN_BY`) rendered `MERGE` compares ordering
/// values with strict `>` — incumbent wins on a tie
/// (`docs/specs/incremental_shapes.md` §"Ordering ties") — and carries the
/// ordering column (the companion running-`MAX` tracking column, Phase 1's
/// storage decision) in the same statement.
#[test]
fn max_by_merge_renders_incumbent_comparison() {
    use smelt_core::config::{Granularity, TimeseriesConfig};
    use smelt_logical::{
        AggregatorColumn, CrossPartitionCombiner, CumulativeClassification, DrivingSource,
    };
    use smelt_runtime::cumulative::build_cumulative_merge_sql;

    let classification = CumulativeClassification {
        unique_key: vec!["device_id".to_string()],
        aggregator_columns: vec![
            AggregatorColumn {
                output_name: "status".to_string(),
                per_partition_agg: "MAX_BY".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::OrderMonotone {
                    ordering_column: "updated_at".to_string(),
                    prefer_greater: true,
                },
                state: None,
            },
            AggregatorColumn {
                output_name: "updated_at".to_string(),
                per_partition_agg: "MAX".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::Max,
                state: None,
            },
        ],
        driving_source: DrivingSource {
            name: "smelt.sources.raw.events".to_string(),
            timeseries: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
        },
    };
    let delta_sql = "SELECT device_id, MAX_BY(status, updated_at) AS status, \
                      MAX(updated_at) AS updated_at FROM events GROUP BY device_id";

    let sql = build_cumulative_merge_sql(
        "main",
        "device_latest",
        delta_sql,
        &classification,
        None,
        &unconditional(),
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    );

    assert!(
        sql.contains(
            "status = CASE WHEN delta.updated_at > target.updated_at THEN delta.status \
             ELSE target.status END"
        ),
        "expected the incumbent-wins CASE comparison over the ordering column, got: {sql}"
    );
    assert!(
        sql.contains("updated_at = GREATEST(target.updated_at, delta.updated_at)"),
        "expected the companion tracking column's own running-MAX update, got: {sql}"
    );
}

/// Phase 4 (`docs/plans/20260809-keyed-frontier.md`): the once-write
/// family's (`COALESCE`) rendered `MERGE` sets each column to
/// `COALESCE(target.<col>, delta.<col>)` — the target's already-set value
/// wins; the delta only ever fills a `NULL` target
/// (`docs/specs/incremental_shapes.md` §"The column-family catalogue").
#[test]
fn once_write_renders_coalesce_target_first() {
    use smelt_core::config::{Granularity, TimeseriesConfig};
    use smelt_logical::{
        AggregatorColumn, CrossPartitionCombiner, CumulativeClassification, DrivingSource,
    };
    use smelt_runtime::cumulative::build_cumulative_merge_sql;

    let classification = CumulativeClassification {
        unique_key: vec!["device_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "signup_referrer".to_string(),
            per_partition_agg: "COALESCE".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::OnceWrite,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.sources.raw.events".to_string(),
            timeseries: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
        },
    };
    let delta_sql = "SELECT device_id, COALESCE(MAX(signup_referrer)) AS \
                      signup_referrer FROM events GROUP BY device_id";

    let sql = build_cumulative_merge_sql(
        "main",
        "device_first_touch",
        delta_sql,
        &classification,
        None,
        &unconditional(),
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    );

    assert!(
        sql.contains("signup_referrer = COALESCE(target.signup_referrer, delta.signup_referrer)"),
        "expected the target-first COALESCE merge, got: {sql}"
    );
}

/// Phase 3 (`docs/plans/20260809-keyed-frontier.md`): the snapshot-reconcile
/// run shape's `MERGE` — a whole-source `USING` select (no window predicate
/// injected into `delta_sql`, unlike the window-forward per-partition
/// driver), plain-overwrite columns assign `delta.<col>` unconditionally
/// (incoming row wins, no target comparison), and — critically — no
/// `DELETE` of departed keys: a key present in the target but absent from
/// the incoming scan is retained unchanged
/// (`docs/specs/incremental_shapes.md` §"The two run shapes").
#[test]
fn snapshot_reconcile_merges_whole_source_no_window() {
    use smelt_core::config::TimeseriesConfig;
    use smelt_logical::{
        AggregatorColumn, CrossPartitionCombiner, CumulativeClassification, DrivingSource,
    };
    use smelt_runtime::cumulative::build_cumulative_merge_sql;

    let classification = CumulativeClassification {
        unique_key: vec!["id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "current_val".to_string(),
            per_partition_agg: "ANY_VALUE".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::PlainOverwrite,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.sources.raw.dim".to_string(),
            // Snapshot-reconcile: no clocked driving source.
            timeseries: None::<TimeseriesConfig>,
        },
    };
    // The whole-source scan, unmodified — no `[run_start, run_end)` window
    // predicate injected anywhere.
    let delta_sql =
        "SELECT id, ANY_VALUE(current_val) AS current_val FROM main.sources_dim GROUP BY id";

    let sql = build_cumulative_merge_sql(
        "main",
        "snapshot_dim",
        delta_sql,
        &classification,
        None,
        &unconditional(),
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    );

    assert!(
        sql.contains(&format!("USING ({delta_sql}) AS delta")),
        "expected the whole-source select verbatim as the USING clause, no window predicate: \
         {sql}"
    );
    assert!(
        sql.contains("current_val = delta.current_val"),
        "expected the plain-overwrite family's unconditional incoming-row-wins assignment: {sql}"
    );
    assert!(
        !sql.to_uppercase().contains("DELETE"),
        "snapshot-reconcile must never delete a departed key — retained unchanged: {sql}"
    );
    assert!(
        sql.contains("WHEN NOT MATCHED THEN INSERT *"),
        "expected the ordinary unmatched-insert arm: {sql}"
    );
}
