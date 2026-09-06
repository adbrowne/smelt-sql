use super::*;

use std::collections::BTreeSet;

use smelt_core::config::{MaintenanceCellConfig, PerSourceScanBounds};

/// The PostgreSQL emission dialect is retired (#181): `Target::backend_type`
/// already rejects `type: postgres` at the declaration boundary, and these
/// two name-keyed resolvers must not resurrect it as a second, unguarded
/// entry point — an unrecognised name resolves conservatively, matching the
/// fail-loud posture both functions already take for any other unknown name.
#[test]
fn retired_backend_names_resolve_to_nothing() {
    for name in ["postgres", "postgresql"] {
        assert_eq!(
            backend_dialect_for(name),
            None,
            "{name} must not resolve to a SqlDialect"
        );
        let caps = backend_write_capabilities_for(name);
        assert_eq!(
            caps,
            smelt_logical::maintenance::BackendWriteCapabilities::default(),
            "{name} must resolve to the conservative default, not a real capability set"
        );
    }
}

fn group(columns: &[&str], sensitivity: &[&str]) -> ColumnGroup {
    ColumnGroup {
        columns: columns.iter().map(|s| s.to_string()).collect(),
        mutation_sensitivity: sensitivity.iter().map(|s| s.to_string()).collect(),
        membership_sensitivity: BTreeSet::new(),
    }
}

fn source_info_with_mutation(kind: Option<SourceMutationKind>) -> SourceInfo {
    SourceInfo {
        path: std::path::PathBuf::from("/tmp/s.yml"),
        address_segments: vec!["sources".to_string(), "s".to_string()],
        columns: vec![],
        description: None,
        name_override: None,
        tags: vec![],
        timeseries: None,
        mutation_profile: kind.map(smelt_core::sources::SourceMutationProfile::from_kind),
        source_lateness: None,
        watermark: None,
        unique_key: None,
        retention: None,
        referential_integrity: None,
    }
}

/// Phase 28c: a declared `mutation_profile: change_feed` source facts to
/// `PlanMutationProfile::ChangeFeed` — while undeclared and `mutable_snapshot`
/// both still fail closed to the stricter `MutableSnapshot` posture.
#[test]
fn source_facts_maps_declared_change_feed() {
    let feed = source_info_with_mutation(Some(SourceMutationKind::ChangeFeed));
    assert_eq!(
        source_facts("feed", Some(&feed), true).mutation,
        PlanMutationProfile::ChangeFeed
    );

    let mutable = source_info_with_mutation(Some(SourceMutationKind::Mutable));
    assert_eq!(
        source_facts("mutable", Some(&mutable), true).mutation,
        PlanMutationProfile::MutableSnapshot
    );

    assert_eq!(
        source_facts("undeclared", None, true).mutation,
        PlanMutationProfile::MutableSnapshot
    );
}

#[test]
fn keyed_fold_write_pin_matches_on_the_driving_source_address() {
    let metadata = ModelMetadata {
        maintenance: Some(MaintenanceConfig {
            defaults: None,
            cells: vec![MaintenanceCellConfig {
                columns: vec![],
                on: "sources.events".to_string(),
                prefer: None,
                technique: None,
                write: Some("staged_candidate".to_string()),
            }],
            scan_bounds: None,
        }),
        ..Default::default()
    };
    assert_eq!(
        keyed_fold_write_pin(&metadata, "sources.events"),
        Some("staged_candidate".to_string())
    );
}

#[test]
fn keyed_fold_write_pin_ignores_a_cell_addressed_at_another_source() {
    let metadata = ModelMetadata {
        maintenance: Some(MaintenanceConfig {
            defaults: None,
            cells: vec![MaintenanceCellConfig {
                columns: vec![],
                on: "sources.other".to_string(),
                prefer: None,
                technique: None,
                write: Some("staged_candidate".to_string()),
            }],
            scan_bounds: None,
        }),
        ..Default::default()
    };
    assert_eq!(keyed_fold_write_pin(&metadata, "sources.events"), None);
}

#[test]
fn keyed_fold_effective_override_matches_by_on_address() {
    let metadata = ModelMetadata {
        maintenance: Some(MaintenanceConfig {
            defaults: None,
            cells: vec![MaintenanceCellConfig {
                columns: vec![],
                on: "sources.events".to_string(),
                prefer: None,
                technique: Some(smelt_core::config::CellTechnique::Unconditional),
                write: None,
            }],
            scan_bounds: None,
        }),
        ..Default::default()
    };
    let effective = keyed_fold_effective_override(&metadata, "sources.events");
    assert_eq!(
        effective.technique,
        Some(smelt_core::config::CellTechnique::Unconditional)
    );

    let non_matching = keyed_fold_effective_override(&metadata, "sources.other");
    assert_eq!(non_matching.technique, None);
    assert_eq!(non_matching.prefer, None);
}

#[test]
fn cells_columns_spanning_groups_error() {
    let groups = vec![
        group(&["converted"], &["payments"]),
        group(&["shipped"], &["shipments"]),
    ];
    let maintenance = MaintenanceConfig {
        defaults: None,
        cells: vec![MaintenanceCellConfig {
            columns: vec!["converted".to_string(), "shipped".to_string()],
            on: "sources.payments".to_string(),
            prefer: None,
            technique: None,
            write: None,
        }],
        scan_bounds: None,
    };
    let violations = cell_column_group_violations(&maintenance, &groups);
    assert_eq!(
        violations.len(),
        1,
        "expected exactly one violation, got {violations:?}"
    );
    assert!(violations[0].contains("sources.payments"));
}

#[test]
fn cells_columns_within_one_group_ok() {
    let groups = vec![group(&["converted", "converted_at"], &["payments"])];
    let maintenance = MaintenanceConfig {
        defaults: None,
        cells: vec![MaintenanceCellConfig {
            columns: vec!["converted".to_string(), "converted_at".to_string()],
            on: "sources.payments".to_string(),
            prefer: None,
            technique: None,
            write: None,
        }],
        scan_bounds: None,
    };
    assert!(cell_column_group_violations(&maintenance, &groups).is_empty());
}

#[test]
fn allow_full_scan_true_clears_scan_unbounded() {
    let sources = [source_facts("sources.enrichment", None, false)];
    assert!(!sources[0].allow_full_scan);
    let sources = [source_facts("sources.enrichment", None, true)];
    assert!(sources[0].allow_full_scan);
}

#[test]
fn effective_scan_bounds_model_overrides_project() {
    let mut project = ScanBoundsConfig::default();
    project.per_source.insert(
        "sources.enrichment".to_string(),
        PerSourceScanBounds {
            max_lookback: None,
            allow_full_scan: false,
        },
    );
    let mut model = ScanBoundsConfig::default();
    model.per_source.insert(
        "sources.enrichment".to_string(),
        PerSourceScanBounds {
            max_lookback: None,
            allow_full_scan: true,
        },
    );
    let (allow, require, on_violation) =
        effective_scan_bounds("sources.enrichment", Some(&model), Some(&project));
    assert!(allow);
    assert_eq!(require, ScanBoundsRequire::PartitionLocal);
    assert_eq!(on_violation, ScanBoundsViolation::Error);
}

#[test]
fn effective_scan_bounds_on_violation_model_overrides_project() {
    let project = ScanBoundsConfig {
        on_violation: Some(ScanBoundsViolation::Error),
        ..Default::default()
    };
    let model = ScanBoundsConfig {
        on_violation: Some(ScanBoundsViolation::Warn),
        ..Default::default()
    };
    let (_, _, on_violation) =
        effective_scan_bounds("sources.enrichment", Some(&model), Some(&project));
    assert_eq!(on_violation, ScanBoundsViolation::Warn);
}

#[test]
fn grain_mismatch_never_admits_fold_without_aggregate() {
    // `grain: key` with a body that never aggregates has no fold
    // candidate — `derive_fold_spec` must return `None`, not fabricate
    // one, so the derivation's own admission refuses honestly.
    let sql = "SELECT user_id, amount FROM smelt.sources.payments";
    assert!(derive_fold_spec(sql, &[]).is_none());
}

#[test]
fn grain_mismatch_detects_single_aggregate() {
    let sql = "SELECT user_id, SUM(amount) AS total FROM smelt.sources.payments GROUP BY user_id";
    let fold = derive_fold_spec(sql, &[]).expect("single SUM aggregate should be a fold candidate");
    assert_eq!(
        fold.add_columns,
        vec![("total".to_string(), SqlFunction::Sum)]
    );
}

#[test]
fn grain_mismatch_detects_multiple_aggregates_with_mixed_combiners() {
    let sql = "SELECT user_id, COUNT(*) AS n, MIN(event_ts) AS first_seen, \
                MAX(event_ts) AS last_seen FROM smelt.sources.events GROUP BY user_id";
    let fold =
        derive_fold_spec(sql, &[]).expect("multi-aggregate SELECT should be a fold candidate");
    assert_eq!(
        fold.add_columns,
        vec![
            ("n".to_string(), SqlFunction::Count),
            ("first_seen".to_string(), SqlFunction::Min),
            ("last_seen".to_string(), SqlFunction::Max),
        ]
    );
}

#[test]
fn grain_mismatch_single_aggregate_shape_unchanged_at_n_equals_1() {
    // Regression: a single-aggregate SELECT derives the exact same
    // `FoldSpec` shape it did before multi-column folds were supported.
    let sql = "SELECT user_id, SUM(amount) AS total FROM smelt.sources.payments GROUP BY user_id";
    let fold = derive_fold_spec(sql, &[]).expect("single SUM aggregate should be a fold candidate");
    assert_eq!(fold.add_columns.len(), 1);
    assert_eq!(fold.add_columns[0], ("total".to_string(), SqlFunction::Sum));
}

#[test]
fn grain_mismatch_unrecognized_aggregate_among_set_refuses_whole_derivation() {
    // Fail-closed: one recognised aggregate (SUM) alongside one
    // unresolvable projection (an unregistered window function — the
    // classifier's window-item branch admits it into `OtherAggregate`
    // without requiring the function name to resolve, unlike the
    // aggregate branch) must refuse the *whole* derivation, not
    // silently fold only the recognised column.
    let sql = "SELECT user_id, SUM(amount) AS total, \
                NOT_A_REAL_FUNCTION() OVER (ORDER BY amount) AS weird \
                FROM smelt.sources.payments GROUP BY user_id";
    assert!(
        derive_fold_spec(sql, &[]).is_none(),
        "an unresolvable aggregate/window item among the set must refuse the whole \
         derivation, not a partial fold"
    );
}

/// A `grain: key` model's derived plan carries the classifier's real
/// `unique_key` (its own GROUP BY columns) — not a hardcoded empty vec.
#[test]
fn keyed_plan_carries_real_unique_key() {
    let sql = "SELECT device_id, user_id, COUNT(*) AS n \
                FROM smelt.sources.events GROUP BY device_id, user_id";
    let metadata = ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(ConfigGrain::Key),
        ..Default::default()
    };
    let result = derive_model_maintenance_plan(
        sql,
        "main.device_user",
        &metadata,
        &[],
        &std::collections::HashSet::new(),
        None,
        &[],
        &[],
        &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
        None,
        None,
        &[],
    )
    .expect("grain: key model must derive a plan");
    // `derive_model_maintenance_plan` threads `derive_group_by_unique_key`
    // into `PlanGrain::Key` — assert the same derivation directly (the
    // plan itself does not yet re-expose the grain on a public surface,
    // `MaintenancePlanResult` carries only cells/refusals/column_groups).
    assert_eq!(
        derive_group_by_unique_key(sql),
        vec!["device_id".to_string(), "user_id".to_string()]
    );
    // Sanity: this model has no timeseries: block, so it must NOT hit
    // the locality refusal — it derives ordinary cells/no-cells like
    // any other grain: key model (no admission assertion beyond "no
    // locality refusal" — the fold/aggregate admission is exercised by
    // other tests).
    assert!(
        !result.plan.refusals.iter().any(|r| matches!(
            r,
            smelt_logical::maintenance::Refusal::LocalityNotEstablished { .. }
        )),
        "no timeseries: block declared — must not hit the locality gate: {:?}",
        result.plan.refusals
    );
}

/// The W1 flagship repro (`docs/plans/20260715-composed-axes-conditional-maintenance.md`
/// Blocked-phases entry): six `MIN`-folded payload columns over one key. Before this
/// phase, `derive_fold_spec` only admitted a single aggregate column, so `inputs.fold`
/// stayed `None` and the `NewData` cell refused with "keyed grain with no fold
/// specification" — reproduced here at the unit level (no example workspace staged).
#[test]
fn keyed_six_column_extremal_fold_no_longer_refuses_for_missing_fold_spec() {
    let sql = "SELECT event_id, MIN(device_id) AS device_id, MIN(user_id) AS user_id, \
                MIN(event_time) AS event_ts, MIN(event_date) AS first_seen_date, \
                MIN(utm_campaign) AS utm_campaign, MIN(payload) AS payload \
                FROM smelt.sources.raw.events GROUP BY event_id";
    let fold = derive_fold_spec(sql, &[]).expect("six-column MIN fold should be a fold candidate");
    assert_eq!(fold.add_columns.len(), 6);
    assert!(fold.add_columns.iter().all(|(_, c)| *c == SqlFunction::Min));

    let metadata = ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(ConfigGrain::Key),
        ..Default::default()
    };
    let result = derive_model_maintenance_plan(
        sql,
        "main.events_deduped",
        &metadata,
        &[],
        &std::collections::HashSet::new(),
        None,
        &[],
        &[],
        &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
        None,
        None,
        &[],
    )
    .expect("grain: key model must derive a plan");
    assert!(
        !result.plan.refusals.iter().any(|r| matches!(
            r,
            smelt_logical::maintenance::Refusal::NoAdmissibleTechnique { why, .. }
                if why.contains("no fold specification")
        )),
        "multi-column fold must be derived — the 'no fold specification' refusal must not \
         recur: {:?}",
        result.plan.refusals
    );
}

/// A `grain: key` model that also declares a `timeseries:` block, but
/// whose `partition_column` is not a `unique_key` column and has no
/// resolvable driving source, is refused by the key-temporal-locality
/// gate (`docs/specs/incremental_shapes.md` §"Key temporal locality
/// (the time-partitioned output)") — no route admits it.
#[test]
fn keyed_with_timeseries_refuses_via_locality_gate() {
    use smelt_core::config::{Granularity, TimeseriesConfig};

    let sql = "SELECT device_id, COUNT(*) AS n FROM smelt.sources.events GROUP BY device_id";
    let metadata = ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(ConfigGrain::Key),
        timeseries: Some(TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        ..Default::default()
    };
    let result = derive_model_maintenance_plan(
        sql,
        "main.device_daily",
        &metadata,
        &[],
        &std::collections::HashSet::new(),
        None,
        &[],
        &[],
        &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
        None,
        None,
        &[],
    )
    .expect("grain: key + timeseries: must still derive a (refused) plan");
    assert!(
        result.plan.cells.is_empty(),
        "a locality-refused model must admit no cells: {:?}",
        result.plan.cells
    );
    assert_eq!(result.plan.refusals.len(), 1, "{:?}", result.plan.refusals);
    match &result.plan.refusals[0] {
        smelt_logical::maintenance::Refusal::LocalityNotEstablished { message } => {
            assert!(
                message.contains("KeyedForbidsTimeseries"),
                "message: {message}"
            );
        }
        other => panic!("expected LocalityNotEstablished, got {other:?}"),
    }
}

/// Route 1 (key-embedded) admits through the full `smelt-db` plumbing:
/// `partition_column` (`event_date`) is a `unique_key` column, the
/// single referenced source is clocked at the same (day) granularity —
/// the model derives an ordinary plan with no `LocalityNotEstablished`
/// refusal (`docs/specs/incremental_shapes.md` §"Key temporal
/// locality").
#[test]
fn keyed_with_timeseries_admits_via_route1_key_embedded() {
    use smelt_core::config::{Granularity, TimeseriesConfig};

    let sql = "SELECT device_id, event_date, COUNT(*) AS n \
                FROM smelt.sources.events GROUP BY device_id, event_date";
    let metadata = ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(ConfigGrain::Key),
        timeseries: Some(TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        ..Default::default()
    };
    let sources = vec![SourceFacts {
        // `SourceFacts::name` is the *bare* source name (`sources.`
        // breadcrumb stripped) — the real convention `smelt-db::lib`
        // builds (`ref_string.strip_prefix("smelt.").and_then(|s|
        // s.strip_prefix("sources."))`), which `locality::
        // resolve_driving_source` matches against.
        name: "events".to_string(),
        mutation: PlanMutationProfile::AppendOnly,
        partition_col: Some("event_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }];
    let result = derive_model_maintenance_plan(
        sql,
        "main.device_daily",
        &metadata,
        &sources,
        &std::collections::HashSet::new(),
        Some(Granularity::Day),
        &[],
        &[],
        &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
        None,
        None,
        &[],
    )
    .expect("route 1 must derive a plan");
    assert!(
        !result.plan.refusals.iter().any(|r| matches!(
            r,
            smelt_logical::maintenance::Refusal::LocalityNotEstablished { .. }
        )),
        "route 1 must admit — no locality refusal expected: {:?}",
        result.plan.refusals
    );
}

/// A source declaring `referential_integrity` in its `.yml` reaches
/// `derive_model_maintenance_plan` as a real `SourceReferentialIntegrity`
/// entry (`docs/outcomes/20260809-probe-backed-facts/phases/03-plan.md`
/// test 9) — the production Salsa call site's own always-empty map
/// (before this phase) is replaced by [`build_source_referential_
/// integrity`], threaded from `source_refs`. A `dim` source declaring
/// both `unique_key` and `referential_integrity` closes its own
/// `UpstreamMutation` cell's P1 verdict; the same call with an empty map
/// (byte-identical to the pre-phase-3 default) leaves it unattempted.
#[test]
fn source_declared_referential_integrity_reaches_the_derivation() {
    let sql = "SELECT fact.event_id, fact.event_date, dim.tier \
                FROM smelt.sources.fact fact \
                LEFT JOIN smelt.sources.dim dim ON fact.dim_id = dim.id";
    let metadata = ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(ConfigGrain::Partition),
        timeseries: Some(smelt_core::config::TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        ..Default::default()
    };
    let sources = vec![
        SourceFacts {
            name: "fact".to_string(),
            mutation: PlanMutationProfile::AppendOnly,
            partition_col: Some("event_date".to_string()),
            unique_key: vec![],
            allow_full_scan: true,
        },
        SourceFacts {
            name: "dim".to_string(),
            mutation: PlanMutationProfile::MutableSnapshot,
            partition_col: None,
            unique_key: vec!["id".to_string()],
            allow_full_scan: true,
        },
    ];
    let explicitly_mutable: std::collections::HashSet<String> =
        std::collections::HashSet::from(["dim".to_string()]);
    let dim_source_info = SourceInfo {
        path: std::path::PathBuf::from("/tmp/dim.yml"),
        address_segments: vec!["sources".to_string(), "dim".to_string()],
        columns: vec![],
        description: None,
        name_override: None,
        tags: vec![],
        timeseries: None,
        mutation_profile: None,
        source_lateness: None,
        watermark: None,
        unique_key: Some(vec!["id".to_string()]),
        retention: None,
        referential_integrity: Some(vec!["id".to_string()]),
    };
    let source_refs: Vec<(String, Option<SourceInfo>)> =
        vec![("dim".to_string(), Some(dim_source_info))];
    let real_ri = build_source_referential_integrity(&source_refs);
    assert_eq!(
        real_ri.get("dim"),
        Some(&vec!["id".to_string()]),
        "build_source_referential_integrity must surface dim's declared \
         referential_integrity, got {real_ri:?}"
    );

    let trigger = smelt_logical::maintenance::Trigger::UpstreamMutation {
        source: "dim".to_string(),
    };
    let with_real_ri = derive_model_maintenance_plan(
        sql,
        "main.t",
        &metadata,
        &sources,
        &explicitly_mutable,
        None,
        &[],
        &[],
        &real_ri,
        None,
        None,
        &[],
    )
    .expect("model must derive a plan");
    let cell = with_real_ri
        .plan
        .cell_for(&trigger)
        .expect("expected an UpstreamMutation cell for dim");
    assert_eq!(
        cell.skeleton_source_closure.as_ref().map(|c| c.is_closed()),
        Some(true),
        "a LEFT JOIN, payload-only, declared-unique_key dimension must close once its \
         declared referential_integrity reaches the derivation, got {:?}",
        cell.skeleton_source_closure
    );

    let with_empty_ri = derive_model_maintenance_plan(
        sql,
        "main.t",
        &metadata,
        &sources,
        &explicitly_mutable,
        None,
        &[],
        &[],
        &SourceReferentialIntegrity::new(),
        None,
        None,
        &[],
    )
    .expect("model must derive a plan");
    let cell = with_empty_ri
        .plan
        .cell_for(&trigger)
        .expect("expected an UpstreamMutation cell for dim");
    assert_eq!(
        cell.skeleton_source_closure, None,
        "an empty referential-integrity map (the pre-phase-3 default) must leave the \
         closure proof unattempted, got {:?}",
        cell.skeleton_source_closure
    );
}

/// Multi-source regression for the driving-source resolution this
/// phase's review fixed: `smelt-db`'s plan-derivation call site and
/// `smelt-runtime`'s runtime execution path (`classify_cumulative`)
/// must agree on which source drives a model. A clocked source
/// referenced only inside a CTE — never joined into the outer
/// SELECT's FROM/JOIN — must not count as a second driving-source
/// candidate here, exactly as it would not for the runtime's
/// alias-scoped `classify_cumulative` resolution
/// (`smelt_logical::maintenance::locality::resolve_driving_source`).
/// Before that shared resolution existed, this call site resolved the
/// driving source over *every* referenced source — seeing two clocked
/// sources here, it would treat the driving source as unresolved and
/// refuse route 1 (`KeyedForbidsTimeseries` via `smelt explain`) even
/// though `smelt build` would actually admit and execute the model.
#[test]
fn multi_source_model_agrees_with_runtime_alias_scoped_driving_source() {
    use smelt_core::config::{Granularity, TimeseriesConfig};

    let sql = "WITH other AS ( \
                   SELECT device_id, event_date FROM smelt.sources.other_stream \
               ) \
               SELECT device_id, event_date, COUNT(*) AS n \
               FROM smelt.sources.events \
               GROUP BY device_id, event_date";
    let metadata = ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(ConfigGrain::Key),
        timeseries: Some(TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        ..Default::default()
    };
    let sources = vec![
        // `SourceFacts::name` is the bare source name — the real
        // convention `smelt-db::lib` builds and `locality::
        // resolve_driving_source` matches against.
        SourceFacts {
            name: "events".to_string(),
            mutation: PlanMutationProfile::AppendOnly,
            partition_col: Some("event_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        },
        // Clocked, but only ever referenced inside the CTE — never
        // joined into the outer SELECT's FROM/JOIN. Must NOT be
        // treated as a second driving-source candidate.
        SourceFacts {
            name: "other_stream".to_string(),
            mutation: PlanMutationProfile::AppendOnly,
            partition_col: Some("event_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        },
    ];
    let result = derive_model_maintenance_plan(
        sql,
        "main.device_daily",
        &metadata,
        &sources,
        &std::collections::HashSet::new(),
        Some(Granularity::Day),
        &[],
        &[],
        &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
        None,
        None,
        &[],
    )
    .expect("route 1 must derive a plan");
    assert!(
        !result.plan.refusals.iter().any(|r| matches!(
            r,
            smelt_logical::maintenance::Refusal::LocalityNotEstablished { .. }
        )),
        "the CTE-only clocked source must not defeat route 1 admission — the driving \
         source must resolve to the outer FROM/JOIN's alias-scoped `sources.events` alone, \
         matching the runtime's `classify_cumulative` resolution: {:?}",
        result.plan.refusals
    );
}

const SUCCESSION_SQL: &str = "SELECT \
     customer_id, \
     changed_at, \
     name, \
     LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_changed_at \
     FROM smelt.sources.customer_changes";

fn succession_source_info() -> SourceInfo {
    SourceInfo {
        path: std::path::PathBuf::from("/tmp/customer_changes.yml"),
        address_segments: vec!["sources".to_string(), "customer_changes".to_string()],
        columns: vec![
            smelt_core::sources::SourceColumn {
                name: "customer_id".to_string(),
                data_type: smelt_types::DataType::Integer,
                nullable: false,
                description: None,
            },
            smelt_core::sources::SourceColumn {
                name: "changed_at".to_string(),
                data_type: smelt_types::DataType::Timestamp {
                    with_timezone: false,
                },
                nullable: false,
                description: None,
            },
        ],
        description: None,
        name_override: None,
        tags: vec![],
        timeseries: Some(smelt_core::config::TimeseriesConfig {
            event_time_column: "changed_at".to_string(),
            partition_column: "changed_at".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        mutation_profile: Some(smelt_core::sources::SourceMutationProfile::from_kind(
            SourceMutationKind::AppendOnly,
        )),
        source_lateness: None,
        watermark: None,
        unique_key: None,
        retention: None,
        referential_integrity: None,
    }
}

fn succession_metadata() -> ModelMetadata {
    ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        ..Default::default()
    }
}

#[test]
fn undeclared_grain_incremental_model_derives_the_succession_plan() {
    let metadata = succession_metadata();
    let source_refs = vec![(
        "customer_changes".to_string(),
        Some(succession_source_info()),
    )];
    let result = derive_model_maintenance_plan(
        SUCCESSION_SQL,
        "main.customer_history",
        &metadata,
        &[],
        &std::collections::HashSet::new(),
        None,
        &[],
        &[],
        &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
        None,
        None,
        &source_refs,
    )
    .expect("undeclared-grain incremental model with a succession-shaped SQL must derive a plan");
    assert!(
        result.plan.refusals.is_empty(),
        "expected the succession cell to admit cleanly: {:?}",
        result.plan.refusals
    );
    assert_eq!(result.plan.cells.len(), 1);
    assert_eq!(
        result.plan.cells[0].technique,
        smelt_logical::maintenance::Technique::SuccessionPatch
    );
    assert_eq!(
        result.plan.cells[0].trigger,
        Trigger::NewData {
            source: "customer_changes".to_string()
        }
    );
}

#[test]
fn undeclared_grain_unrecognised_shape_derives_the_succession_refusal() {
    let metadata = succession_metadata();
    let sql = "SELECT customer_id, COUNT(*) AS n FROM smelt.sources.customer_changes GROUP BY customer_id";
    let result = derive_model_maintenance_plan(
        sql,
        "main.customer_counts",
        &metadata,
        &[],
        &std::collections::HashSet::new(),
        None,
        &[],
        &[],
        &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
        None,
        None,
        &[],
    )
    .expect("undeclared-grain incremental model must still derive a (refused) plan");
    assert!(result.plan.cells.is_empty());
    assert_eq!(result.plan.refusals.len(), 1);
    assert!(matches!(
        result.plan.refusals[0],
        smelt_logical::maintenance::Refusal::SuccessionNotRecognized { .. }
    ));
}

#[test]
fn succession_context_is_built_from_the_source_declarations() {
    let source_refs = vec![(
        "customer_changes".to_string(),
        Some(succession_source_info()),
    )];
    let ctx = build_succession_context(SUCCESSION_SQL, &source_refs);
    assert_eq!(ctx.source_name, "sources.customer_changes");
    assert_eq!(ctx.event_time_column.as_deref(), Some("changed_at"));
    assert!(ctx.not_null_columns.contains("customer_id"));
    assert!(ctx.not_null_columns.contains("changed_at"));

    // Undeclared profile fails closed: no `SourceInfo` for the driving
    // source resolves to an empty/`None`-carrying context, never a panic.
    let ctx_undeclared = build_succession_context(SUCCESSION_SQL, &[]);
    assert_eq!(ctx_undeclared.mutation_profile, None);
    assert_eq!(ctx_undeclared.event_time_column, None);
    assert!(ctx_undeclared.not_null_columns.is_empty());
}

#[test]
fn declared_grain_models_are_unchanged() {
    // A `grain: partition` model.
    let partition_sql =
        "SELECT event_date, COUNT(*) AS n FROM smelt.sources.events GROUP BY event_date";
    let partition_metadata = ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(ConfigGrain::Partition),
        timeseries: Some(smelt_core::config::TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        ..Default::default()
    };
    let partition_sources = vec![SourceFacts {
        name: "events".to_string(),
        mutation: PlanMutationProfile::AppendOnly,
        partition_col: Some("event_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }];
    let with_refs = derive_model_maintenance_plan(
        partition_sql,
        "main.events_daily",
        &partition_metadata,
        &partition_sources,
        &std::collections::HashSet::new(),
        None,
        &[],
        &[],
        &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
        None,
        None,
        &[(
            "customer_changes".to_string(),
            Some(succession_source_info()),
        )],
    )
    .expect("grain: partition model must derive a plan");
    let without_refs = derive_model_maintenance_plan(
        partition_sql,
        "main.events_daily",
        &partition_metadata,
        &partition_sources,
        &std::collections::HashSet::new(),
        None,
        &[],
        &[],
        &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
        None,
        None,
        &[],
    )
    .expect("grain: partition model must derive a plan");
    assert_eq!(with_refs.plan.cells.len(), without_refs.plan.cells.len());
    assert_eq!(
        with_refs.plan.cells[0].technique,
        without_refs.plan.cells[0].technique
    );

    // A `grain: key` model.
    let key_sql = "SELECT user_id, SUM(amount) AS lifetime_spend FROM smelt.sources.payments GROUP BY user_id";
    let key_metadata = ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(ConfigGrain::Key),
        ..Default::default()
    };
    let key_sources = vec![SourceFacts {
        name: "payments".to_string(),
        mutation: PlanMutationProfile::AppendOnly,
        partition_col: Some("pay_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }];
    let key_result = derive_model_maintenance_plan(
        key_sql,
        "main.lifetime_spend",
        &key_metadata,
        &key_sources,
        &std::collections::HashSet::new(),
        None,
        &[],
        &[],
        &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
        None,
        None,
        &[(
            "customer_changes".to_string(),
            Some(succession_source_info()),
        )],
    )
    .expect("grain: key model must derive a plan");
    assert!(!key_result.plan.cells.is_empty());
}
