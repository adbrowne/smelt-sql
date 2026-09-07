use super::*;

// ---------------------------------------------------------------------
// Phase 27a: `--show-sql` previews the change-suppressed matched arm
// (`docs/outcomes/20260815-definition-delta-migrate/phases/27a-plan.md`;
// `docs/specs/incremental_models.md` §"Statement emission (single owner)")
// ---------------------------------------------------------------------

/// A minimal synthetic `ColumnScopedMerge` model: a fact joined to a
/// mutable dimension, projecting `expr_sql AS user_name`. Controlling the
/// projected expression lets each test choose whether `user_name` is P3
/// change-comparable (a bare column reference) or not (`RANDOM()`, a
/// row-nondeterministic function — `walk.rs::expr_comparability`'s own
/// fail-closed case).
fn merge_probe_model(expr_sql: &str) -> ModelFile {
    let content = format!(
        "SELECT e.event_id, e.user_id, {expr_sql} AS user_name FROM smelt.sources.raw.events e \
         JOIN smelt.sources.raw.users u ON e.user_id = u.user_id"
    );
    let path: PathBuf = "merge_probe.sql".into();
    ModelFile {
        name: "merge_probe".to_string(),
        model_id: smelt_core::ModelId::from_path(path.clone()),
        path,
        content,
        refs: vec![
            RefInfo {
                has_named_params: false,
                range: Default::default(),
                smelt_ref: SmeltRef::Path(vec![
                    "sources".to_string(),
                    "raw".to_string(),
                    "events".to_string(),
                ]),
            },
            RefInfo {
                has_named_params: false,
                range: Default::default(),
                smelt_ref: SmeltRef::Path(vec![
                    "sources".to_string(),
                    "raw".to_string(),
                    "users".to_string(),
                ]),
            },
        ],
        parse_errors: Vec::new(),
        metadata: Some(Box::new(ModelMetadata {
            materialization: Some(Materialization::Table),
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(ConfigGrain::Partition),
            timeseries: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            ..Default::default()
        })),
        kind: smelt_core::ModelKind::Sql,
        address_segments: vec!["merge_probe".to_string()],
    }
}

/// A synthetic `Technique::ColumnScopedMerge` cell over `merge_probe_model`'s
/// `{user_name}` column group, keyed on `event_id`.
fn merge_probe_cell(trigger: Trigger, ledger_catch_up: bool) -> PlanCell {
    PlanCell {
        group: "{user_name}".to_string(),
        trigger,
        corner: Corner::ColumnMerge,
        technique: Technique::ColumnScopedMerge,
        partition_local: PartitionLocal::Yes,
        scans: vec![],
        ledger_catch_up,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::Key(vec!["event_id".to_string()]),
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: Default::default(),
        key_scope: None,
        state_downgrade: None,
    }
}

fn merge_probe_column_groups() -> Vec<ColumnGroup> {
    vec![ColumnGroup {
        columns: vec!["user_name".to_string()],
        mutation_sensitivity: Default::default(),
        membership_sensitivity: Default::default(),
    }]
}

/// `27a-plan.md`'s `column_scoped_merge_preview_renders_the_suppressed_matched_arm`:
/// a suppressible `ColumnScopedMerge` cell's preview statements carry the
/// `IS DISTINCT FROM` matched-arm guard — `user_name` is a bare pass-through
/// column (P3 `Comparable`), the row identity is a proven key (P2), and the
/// steady-state `UpstreamMutation` trigger over prior state (`ledger_catch_up:
/// false`) is the structural default `resolve_write_variant` prefers.
#[test]
fn column_scoped_merge_preview_renders_the_suppressed_matched_arm() {
    let model = merge_probe_model("u.user_name");
    let cell = merge_probe_cell(
        Trigger::UpstreamMutation {
            source: "raw.users".to_string(),
        },
        false,
    );
    let column_groups = merge_probe_column_groups();
    let config = load_fixture().2;
    let registry = CompilerRegistry::new(&config, &config.targets);
    let resolver = registry
        .get("dev")
        .build_ephemeral_resolver(&[], "main")
        .expect("no ephemerals");
    let source_timeseries = smelt_planner::SourceTimeseriesMap::new();

    let diagnostics = build_plan_cell_diagnostics(
        &cell,
        &model,
        "main",
        "dev",
        &registry,
        &resolver,
        MaintenanceDialect::DuckDb,
        &["event_id".to_string()],
        &source_timeseries,
        &column_groups,
    );

    let preview = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == Technique::ColumnScopedMerge)
        .expect("ColumnScopedMerge must have a preview entry");
    assert_eq!(preview.admissibility, Admissibility::Admitted);
    let sql = preview
        .statements
        .first()
        .expect("the Admitted ColumnScopedMerge preview must render a statement")
        .sql
        .clone();
    assert!(
        sql.contains("IS DISTINCT FROM") && sql.contains("user_name"),
        "expected the change-suppressed matched-arm guard over `user_name`: {sql}"
    );
}

/// `27a-plan.md`'s `first_build_cell_preview_keeps_the_unconditional_matched_arm`:
/// a cell with no prior stored state to diff against (`Trigger::Backfill`)
/// resolves the unconditional matched arm by default, matching
/// `resolve_write_variant`'s `FirstBuildPosture` branch — even though the
/// P2/P3 proof itself is admitted.
#[test]
fn first_build_cell_preview_keeps_the_unconditional_matched_arm() {
    let model = merge_probe_model("u.user_name");
    let cell = merge_probe_cell(Trigger::Backfill, false);
    let column_groups = merge_probe_column_groups();
    let config = load_fixture().2;
    let registry = CompilerRegistry::new(&config, &config.targets);
    let resolver = registry
        .get("dev")
        .build_ephemeral_resolver(&[], "main")
        .expect("no ephemerals");
    let source_timeseries = smelt_planner::SourceTimeseriesMap::new();

    let diagnostics = build_plan_cell_diagnostics(
        &cell,
        &model,
        "main",
        "dev",
        &registry,
        &resolver,
        MaintenanceDialect::DuckDb,
        &["event_id".to_string()],
        &source_timeseries,
        &column_groups,
    );

    let preview = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == Technique::ColumnScopedMerge)
        .expect("ColumnScopedMerge must have a preview entry");
    let sql = preview
        .statements
        .first()
        .expect("the Admitted ColumnScopedMerge preview must render a statement")
        .sql
        .clone();
    assert!(
        !sql.contains("IS DISTINCT FROM"),
        "a first-build cell (no prior stored state) must keep the unconditional matched arm: {sql}"
    );
}

/// `27a-plan.md`'s `incomparable_group_preview_keeps_the_unconditional_matched_arm`:
/// a P3 refusal (a row-nondeterministic `RANDOM()` projection) renders the
/// plain unconditional arm, even over a steady-state trigger with prior
/// state.
#[test]
fn incomparable_group_preview_keeps_the_unconditional_matched_arm() {
    let model = merge_probe_model("RANDOM()");
    let cell = merge_probe_cell(
        Trigger::UpstreamMutation {
            source: "raw.users".to_string(),
        },
        false,
    );
    let column_groups = merge_probe_column_groups();
    let config = load_fixture().2;
    let registry = CompilerRegistry::new(&config, &config.targets);
    let resolver = registry
        .get("dev")
        .build_ephemeral_resolver(&[], "main")
        .expect("no ephemerals");
    let source_timeseries = smelt_planner::SourceTimeseriesMap::new();

    let diagnostics = build_plan_cell_diagnostics(
        &cell,
        &model,
        "main",
        "dev",
        &registry,
        &resolver,
        MaintenanceDialect::DuckDb,
        &["event_id".to_string()],
        &source_timeseries,
        &column_groups,
    );

    let preview = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == Technique::ColumnScopedMerge)
        .expect("ColumnScopedMerge must have a preview entry");
    let sql = preview
        .statements
        .first()
        .expect("the Admitted ColumnScopedMerge preview must render a statement")
        .sql
        .clone();
    assert!(
        !sql.contains("IS DISTINCT FROM"),
        "an incomparable compared column must fail closed to the unconditional matched arm: {sql}"
    );
}

/// `27a-plan.md`'s `suppress_pin_over_a_refused_proof_yields_no_preview_statements`:
/// a `technique: suppress` pin whose write-suppression proof refused (P3
/// incomparable) surfaces as a build error — empty statements, non-`Admitted`
/// admissibility — never a silent unconditional fallback (a live run over
/// this same cell would hard-fail the whole run, `maintenance_driver.rs`'s
/// own `ChoiceRefusal` propagation).
#[test]
fn suppress_pin_over_a_refused_proof_yields_no_preview_statements() {
    let mut model = merge_probe_model("RANDOM()");
    model.metadata.as_mut().unwrap().maintenance = Some(MaintenanceConfig {
        defaults: None,
        cells: vec![MaintenanceCellConfig {
            columns: vec!["user_name".to_string()],
            on: "raw.users".to_string(),
            prefer: None,
            technique: Some(CellTechnique::Suppress),
            write: None,
        }],
        scan_bounds: None,
    });
    let cell = merge_probe_cell(
        Trigger::UpstreamMutation {
            source: "raw.users".to_string(),
        },
        false,
    );
    let column_groups = merge_probe_column_groups();
    let config = load_fixture().2;
    let registry = CompilerRegistry::new(&config, &config.targets);
    let resolver = registry
        .get("dev")
        .build_ephemeral_resolver(&[], "main")
        .expect("no ephemerals");
    let source_timeseries = smelt_planner::SourceTimeseriesMap::new();

    let diagnostics = build_plan_cell_diagnostics(
        &cell,
        &model,
        "main",
        "dev",
        &registry,
        &resolver,
        MaintenanceDialect::DuckDb,
        &["event_id".to_string()],
        &source_timeseries,
        &column_groups,
    );

    let preview = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == Technique::ColumnScopedMerge)
        .expect("ColumnScopedMerge must have a preview entry");
    assert!(
        preview.statements.is_empty(),
        "a refused suppress pin must yield no preview statements: {:?}",
        preview.statements
    );
    match &preview.admissibility {
        Admissibility::NotApplicable { reason } => {
            assert!(
                !reason.is_empty(),
                "the refusal reason must be surfaced, not empty"
            );
        }
        other => panic!(
            "expected NotApplicable for a technique:suppress pin over a refused proof, got {other:?}"
        ),
    }
}

/// `27a-plan.md`'s `keyed_fold_preview_renders_the_suppressed_matched_arm`:
/// a suppressible `KeyedFold` cell's preview statements carry the guard
/// comparing the stored value against the fold's own combine expression —
/// `total_amount` (a plain `SUM`) is P3 `Comparable`
/// (`walk.rs::expr_comparability`: a registry-backed, non-nondeterministic
/// aggregate taints nothing) and the derived key is a proven `RowIdentity::Key`.
#[test]
fn keyed_fold_preview_renders_the_suppressed_matched_arm() {
    let content =
        "SELECT device_id, SUM(amount) AS total_amount FROM smelt.events GROUP BY device_id";
    let path: PathBuf = "device_total.sql".into();
    let model = ModelFile {
        name: "device_total".to_string(),
        model_id: smelt_core::ModelId::from_path(path.clone()),
        path,
        content: content.to_string(),
        refs: vec![RefInfo {
            has_named_params: false,
            range: Default::default(),
            smelt_ref: SmeltRef::Path(vec!["events".to_string()]),
        }],
        parse_errors: Vec::new(),
        metadata: Some(Box::new(ModelMetadata {
            materialization: Some(Materialization::Table),
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(ConfigGrain::Key),
            ..Default::default()
        })),
        kind: smelt_core::ModelKind::Sql,
        address_segments: vec!["device_total".to_string()],
    };
    let metadata = model.metadata.as_deref().unwrap();
    let stripped_sql = smelt_parser::strip_frontmatter(&model.content).to_string();

    let sources = vec![smelt_logical::maintenance::SourceFacts {
        name: "events".to_string(),
        mutation: smelt_logical::maintenance::MutationProfile::AppendOnly,
        partition_col: Some("event_date".to_string()),
        unique_key: vec![],
        allow_full_scan: true,
    }];
    let plan_result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        &stripped_sql,
        "device_total",
        metadata,
        &sources,
        &std::collections::HashSet::new(),
        None,
        &[],
        &[],
        &std::collections::BTreeMap::new(),
        None,
        None,
        &[],
    )
    .expect("device_total must derive a maintenance plan");
    let cell = plan_result
        .plan
        .cells
        .iter()
        .find(|c| c.technique == Technique::KeyedFold)
        .expect("device_total must admit a KeyedFold cell");

    let mut source_timeseries = smelt_planner::SourceTimeseriesMap::new();
    source_timeseries.insert(
        "smelt.events".to_string(),
        TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        },
    );

    let config = load_fixture().2;
    let registry = CompilerRegistry::new(&config, &config.targets);
    let resolver = registry
        .get("dev")
        .build_ephemeral_resolver(&[], "main")
        .expect("no ephemerals");

    let diagnostics = build_plan_cell_diagnostics(
        cell,
        &model,
        "main",
        "dev",
        &registry,
        &resolver,
        MaintenanceDialect::DuckDb,
        &[],
        &source_timeseries,
        &plan_result.column_groups,
    );

    let preview = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == Technique::KeyedFold)
        .expect("KeyedFold must have a preview entry");
    assert_eq!(preview.admissibility, Admissibility::Admitted);
    let sql = preview
        .statements
        .first()
        .expect("the Admitted KeyedFold preview must render a statement")
        .sql
        .clone();
    assert!(
        sql.contains("IS DISTINCT FROM"),
        "expected the change-suppressed matched-arm guard over `total_amount`: {sql}"
    );
}

/// `docs/outcomes/20260815-definition-delta-migrate/phases/33-plan.md`:
/// `smelt explain --show-sql`'s keyed-fold preview must honour a
/// `maintenance.cells[].technique: unconditional` pin addressing the
/// keyed fold's driving source — preview/live parity, since the live
/// windowed-keyed-maintenance path now folds the same override ladder in
/// (`crate::cumulative::resolve_cumulative_write_suppression`). Otherwise
/// identical to `keyed_fold_preview_renders_the_suppressed_matched_arm`
/// above, whose model/plan this pin is layered onto — without the pin the
/// preview would render the change-suppressed guard.
#[test]
fn explain_show_sql_keyed_fold_honours_the_pin() {
    let content =
        "SELECT device_id, SUM(amount) AS total_amount FROM smelt.events GROUP BY device_id";
    let path: PathBuf = "device_total.sql".into();
    let model = ModelFile {
        name: "device_total".to_string(),
        model_id: smelt_core::ModelId::from_path(path.clone()),
        path,
        content: content.to_string(),
        refs: vec![RefInfo {
            has_named_params: false,
            range: Default::default(),
            smelt_ref: SmeltRef::Path(vec!["events".to_string()]),
        }],
        parse_errors: Vec::new(),
        metadata: Some(Box::new(ModelMetadata {
            materialization: Some(Materialization::Table),
            refresh: Some(RefreshStrategy::Incremental),
            grain: Some(ConfigGrain::Key),
            maintenance: Some(smelt_core::config::MaintenanceConfig {
                defaults: None,
                cells: vec![smelt_core::config::MaintenanceCellConfig {
                    columns: vec![],
                    on: "smelt.events".to_string(),
                    prefer: None,
                    technique: Some(smelt_core::config::CellTechnique::Unconditional),
                    write: None,
                }],
                scan_bounds: None,
            }),
            ..Default::default()
        })),
        kind: smelt_core::ModelKind::Sql,
        address_segments: vec!["device_total".to_string()],
    };
    let metadata = model.metadata.as_deref().unwrap();
    let stripped_sql = smelt_parser::strip_frontmatter(&model.content).to_string();

    let sources = vec![smelt_logical::maintenance::SourceFacts {
        name: "events".to_string(),
        mutation: smelt_logical::maintenance::MutationProfile::AppendOnly,
        partition_col: Some("event_date".to_string()),
        unique_key: vec![],
        allow_full_scan: true,
    }];
    let plan_result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        &stripped_sql,
        "device_total",
        metadata,
        &sources,
        &std::collections::HashSet::new(),
        None,
        &[],
        &[],
        &std::collections::BTreeMap::new(),
        None,
        None,
        &[],
    )
    .expect("device_total must derive a maintenance plan");
    let cell = plan_result
        .plan
        .cells
        .iter()
        .find(|c| c.technique == Technique::KeyedFold)
        .expect("device_total must admit a KeyedFold cell");

    let mut source_timeseries = smelt_planner::SourceTimeseriesMap::new();
    source_timeseries.insert(
        "smelt.events".to_string(),
        TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        },
    );

    let config = load_fixture().2;
    let registry = CompilerRegistry::new(&config, &config.targets);
    let resolver = registry
        .get("dev")
        .build_ephemeral_resolver(&[], "main")
        .expect("no ephemerals");

    let diagnostics = build_plan_cell_diagnostics(
        cell,
        &model,
        "main",
        "dev",
        &registry,
        &resolver,
        MaintenanceDialect::DuckDb,
        &[],
        &source_timeseries,
        &plan_result.column_groups,
    );

    let preview = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == Technique::KeyedFold)
        .expect("KeyedFold must have a preview entry");
    assert_eq!(preview.admissibility, Admissibility::Admitted);
    let sql = preview
        .statements
        .first()
        .expect("the Admitted KeyedFold preview must render a statement")
        .sql
        .clone();
    assert!(
        !sql.contains("IS DISTINCT FROM"),
        "the pinned technique: unconditional must suppress the change-suppressed guard: {sql}"
    );
}
