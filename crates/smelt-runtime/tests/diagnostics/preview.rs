use super::*;

/// A synthetic `PlanCell` for tests that need a specific `RowIdentity`/
/// `Technique` shape more directly than hunting a fixture for it — mirrors
/// `smelt_logical::maintenance::choice`'s own test helpers
/// (`admitted_plan` in `choice.rs`'s `#[cfg(test)] mod tests`).
fn synthetic_cell(technique: Technique, row_identity: RowIdentity) -> PlanCell {
    PlanCell {
        group: "{status}".to_string(),
        trigger: smelt_logical::maintenance::Trigger::UpstreamMutation {
            source: "raw.user_status".to_string(),
        },
        corner: smelt_logical::maintenance::Corner::ColumnMerge,
        technique,
        partition_local: smelt_logical::maintenance::PartitionLocal::Yes,
        scans: vec![],
        ledger_catch_up: false,
        row_identity: smelt_logical::maintenance::RowIdentityVerdict {
            identity: row_identity,
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: Default::default(),
        key_scope: None,
        state_downgrade: None,
    }
}

/// `docs/specs/ui_model_diagnostics.md` §Constraints: "the technique-preview
/// builder must call the same pure emitters … that a live run uses for the
/// admitted technique — a preview's statements for the `Admitted` entry
/// must be identical to what `incremental_models.md`'s 'Statement emission
/// (single owner)' rule already guarantees a run executes." `daily_cube_
/// metrics`'s single creation cell (`Trigger::NewData`, `Technique::
/// DeleteInsert`) always builds cleanly (a declared `timeseries.
/// partition_column`, no `unique_key`/keyed-fold shape needed) — its
/// `Admitted` preview must render real, non-empty statements, and
/// `smelt-cli::explain::build_admitted_statement_group` — the CLI's own
/// thin reader over this same entry — must reproduce it byte-identically,
/// substituting real `--period` literals for the builder's symbolic
/// `{{window_start}}`/`{{window_end}}` placeholders when a concrete period
/// is given (`smelt-cli` no longer keeps a second, independent derivation
/// for this cell — `docs/plans/20260725-ui-model-diagnostics.md`).
#[test]
fn admitted_preview_matches_live_run_statements() {
    let (models, source_infos, config) = load_fixture();
    let model = find_model(&models, "daily_cube_metrics");
    let cf = compile_fixture(&config);
    let graph = DependencyGraph::build(models.clone(), None).expect("graph builds");
    let source_timeseries = build_source_timeseries_map(&graph, &source_infos);
    let (cells, column_groups) = derive_plan_cells(model, &source_infos);
    let cell = cells
        .iter()
        .find(|c| c.technique == Technique::DeleteInsert)
        .expect("daily_cube_metrics must admit a DeleteInsert creation cell");

    let diagnostics = build_plan_cell_diagnostics(
        cell,
        model,
        "main",
        "dev",
        &cf.registry,
        &cf.resolver,
        MaintenanceDialect::DuckDb,
        &[],
        &source_timeseries,
        &column_groups,
    );

    let admitted = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.admissibility == Admissibility::Admitted)
        .expect("exactly one preview must be Admitted");
    assert_eq!(admitted.technique, Technique::DeleteInsert);
    assert!(
        !admitted.statements.is_empty(),
        "the Admitted DeleteInsert preview must render real statements"
    );

    // No `--period`: the CLI's reader must reproduce the preview's own
    // symbolic-placeholder statements verbatim, byte-identical.
    let placeholders = smelt_cli::explain::build_admitted_statement_group(
        &diagnostics,
        &smelt_cli::explain::RegionLiterals::Placeholders,
    )
    .expect("smelt-cli's reader must succeed for an Admitted preview with real statements");
    let admitted_sql: Vec<&str> = admitted.statements.iter().map(|s| s.sql.as_str()).collect();
    let placeholder_sql: Vec<&str> = placeholders
        .statements
        .iter()
        .map(|s| s.sql.as_str())
        .collect();
    assert_eq!(
        admitted_sql, placeholder_sql,
        "smelt-cli::explain::build_admitted_statement_group must reproduce the shared \
         builder's own Admitted preview statements byte-identically when no --period is given"
    );
    assert_eq!(admitted.transactional, placeholders.transactional);

    // A concrete `--period`: every `{{window_start}}`/`{{window_end}}`
    // token in the preview's own statements must be replaced by the real
    // literal, nothing else touched.
    let with_period = smelt_cli::explain::build_admitted_statement_group(
        &diagnostics,
        &smelt_cli::explain::RegionLiterals::Period {
            start: "2024-01-01".to_string(),
            end: "2024-01-03".to_string(),
        },
    )
    .expect("smelt-cli's reader must succeed under a concrete --period");
    for stmt in &with_period.statements {
        assert!(
            !stmt.sql.contains("{{window_start}}") && !stmt.sql.contains("{{window_end}}"),
            "expected the real --period literals substituted in, no placeholders left: {}",
            stmt.sql
        );
    }
    assert!(
        with_period
            .statements
            .iter()
            .any(|s| s.sql.contains("2024-01-01") && s.sql.contains("2024-01-03")),
        "expected the real --period literals present in the substituted statements: {:?}",
        with_period.statements
    );
}

/// `docs/specs/ui_model_diagnostics.md` §Semantics "Admissibility verdict":
/// "Region recompute is always `InterchangeableAlternative` when not itself
/// the admitted technique". A cell whose admitted technique is
/// `ColumnScopedMerge` still has a declared timeseries partition axis, so
/// its `DeleteInsert` preview always builds — and must never be `Admitted`
/// or `NotApplicable` here.
#[test]
fn recompute_is_always_interchangeable_when_not_admitted() {
    let (models, source_infos, config) = load_fixture();
    let model = find_model(&models, "daily_cube_metrics");
    let cf = compile_fixture(&config);
    let graph = DependencyGraph::build(models.clone(), None).expect("graph builds");
    let source_timeseries = build_source_timeseries_map(&graph, &source_infos);

    // A synthetic cell borrowing `daily_cube_metrics`'s own model (which
    // declares a `timeseries.partition_column`, so `DeleteInsert` always
    // builds) but a different admitted technique, so the `DeleteInsert`
    // preview is never the `Admitted` entry.
    let cell = synthetic_cell(
        Technique::ColumnScopedMerge,
        RowIdentity::Key(vec!["k".into()]),
    );

    let diagnostics = build_plan_cell_diagnostics(
        &cell,
        model,
        "main",
        "dev",
        &cf.registry,
        &cf.resolver,
        MaintenanceDialect::DuckDb,
        &["k".to_string()],
        &source_timeseries,
        &[],
    );

    let delete_insert_preview = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == Technique::DeleteInsert)
        .expect("every_known_technique_has_an_entry: DeleteInsert must have an entry");
    assert_eq!(
        delete_insert_preview.admissibility,
        Admissibility::InterchangeableAlternative,
        "region recompute must be InterchangeableAlternative when not the admitted technique, \
         got {:?}",
        delete_insert_preview.admissibility
    );
    assert!(
        !delete_insert_preview.statements.is_empty(),
        "an InterchangeableAlternative preview must still carry real illustrative SQL"
    );
}

/// `docs/specs/ui_model_diagnostics.md` §Semantics "Admissibility verdict":
/// a `NotApplicable` preview must carry a non-empty `reason` and must still
/// show illustrative SQL where the emitter can render it. A cell with
/// `RowIdentity::WholeRow` cannot structurally support a keyed-fold's
/// per-row addressing — `daily_events_status`'s real `{status}` cell is
/// exactly this shape (its own doc comment: "this cell's own row identity
/// resolves `WholeRow`").
#[test]
fn not_applicable_carries_reason() {
    let (models, source_infos, config) = load_fixture();
    let model = find_model(&models, "daily_events_status");
    let cf = compile_fixture(&config);
    let graph = DependencyGraph::build(models.clone(), None).expect("graph builds");
    let source_timeseries = build_source_timeseries_map(&graph, &source_infos);
    let (cells, column_groups) = derive_plan_cells(model, &source_infos);
    let cell = cells
        .iter()
        .find(|c| matches!(c.row_identity.identity, RowIdentity::WholeRow))
        .expect("daily_events_status must have a WholeRow-identity cell");

    let diagnostics = build_plan_cell_diagnostics(
        cell,
        model,
        "main",
        "dev",
        &cf.registry,
        &cf.resolver,
        MaintenanceDialect::DuckDb,
        &[],
        &source_timeseries,
        &column_groups,
    );

    let keyed_fold_preview = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == Technique::KeyedFold)
        .expect("KeyedFold must have an entry");
    match &keyed_fold_preview.admissibility {
        Admissibility::NotApplicable { reason } => {
            assert!(
                !reason.is_empty(),
                "a NotApplicable preview must always carry a non-empty reason"
            );
        }
        other => panic!("expected NotApplicable for a WholeRow-identity cell, got {other:?}"),
    }
}

/// `docs/specs/ui_model_diagnostics.md` §Semantics "Admissibility verdict":
/// "Exactly one preview entry per cell is `Admitted`." Checked across every
/// cell of several real fixture models with distinct admitted-technique
/// shapes.
#[test]
fn exactly_one_admitted_per_cell() {
    let (models, source_infos, config) = load_fixture();
    let cf = compile_fixture(&config);
    let graph = DependencyGraph::build(models.clone(), None).expect("graph builds");
    let source_timeseries = build_source_timeseries_map(&graph, &source_infos);

    for model_name in [
        "daily_cube_metrics",
        "daily_events_enriched",
        "daily_events_status",
    ] {
        let model = find_model(&models, model_name);
        let (cells, column_groups) = derive_plan_cells(model, &source_infos);
        assert!(
            !cells.is_empty(),
            "{model_name} must derive at least one maintenance-plan cell"
        );
        for cell in &cells {
            let diagnostics = build_plan_cell_diagnostics(
                cell,
                model,
                "main",
                "dev",
                &cf.registry,
                &cf.resolver,
                MaintenanceDialect::DuckDb,
                &[],
                &source_timeseries,
                &column_groups,
            );
            let admitted_count = diagnostics
                .technique_previews
                .iter()
                .filter(|p| p.admissibility == Admissibility::Admitted)
                .count();
            assert_eq!(
                admitted_count, 1,
                "{model_name} cell {:?} must have exactly one Admitted preview, got {}: {:?}",
                diagnostics.group, admitted_count, diagnostics.technique_previews
            );
        }
    }
}

/// `docs/specs/ui_model_diagnostics.md` §Semantics "Technique preview set":
/// "never partial by omission" — every cell's preview set carries one entry
/// per technique the emitters implement, regardless of the cell's own
/// admitted technique.
#[test]
fn every_known_technique_has_an_entry() {
    let (models, source_infos, config) = load_fixture();
    let model = find_model(&models, "daily_events_enriched");
    let cf = compile_fixture(&config);
    let graph = DependencyGraph::build(models.clone(), None).expect("graph builds");
    let source_timeseries = build_source_timeseries_map(&graph, &source_infos);
    let (cells, column_groups) = derive_plan_cells(model, &source_infos);
    let cell = cells.first().expect("must have at least one cell");

    let diagnostics = build_plan_cell_diagnostics(
        cell,
        model,
        "main",
        "dev",
        &cf.registry,
        &cf.resolver,
        MaintenanceDialect::DuckDb,
        &[],
        &source_timeseries,
        &column_groups,
    );

    let techniques: std::collections::BTreeSet<String> = diagnostics
        .technique_previews
        .iter()
        .map(|p| format!("{:?}", p.technique))
        .collect();
    assert_eq!(
        techniques,
        std::collections::BTreeSet::from([
            "DeleteInsert".to_string(),
            "KeyedFold".to_string(),
            "ColumnScopedMerge".to_string(),
            "InPlaceUpdate".to_string(),
            "PerGroupRecompute".to_string(),
        ]),
        "every known technique must have exactly one preview entry, got {:?}",
        techniques
    );
}

/// `docs/plans/20260725-ui-model-diagnostics.md` §"Phase 2b" required
/// regression guard: `smelt_logical::maintenance::choice::resolve_cell_choice`
/// is unaffected by this phase's additions (the new `technique_requires_
/// row_identity` classifier is read-only and additive, consulted only by
/// the `smelt-runtime` technique-preview builder, never by `resolve_cell_
/// choice` itself). Mirrors `choice.rs`'s own `pin_bypasses_cost_model_but_
/// not_admission` test scenario as a pinned-output guard.
#[test]
fn choice_rs_execution_semantics_unchanged() {
    use smelt_core::config::CellTechnique;
    use smelt_logical::maintenance::choice::{
        resolve_cell_choice, ChosenTechnique, EffectiveOverride,
    };
    use smelt_logical::maintenance::{Corner, MaintenancePlan, PartitionLocal, Trigger};

    let plan = MaintenancePlan {
        cells: vec![PlanCell {
            group: "{tier}".to_string(),
            trigger: Trigger::UpstreamMutation {
                source: "users".to_string(),
            },
            corner: Corner::ColumnMerge,
            technique: Technique::ColumnScopedMerge,
            partition_local: PartitionLocal::Yes,
            scans: vec![],
            ledger_catch_up: false,
            row_identity: smelt_logical::maintenance::RowIdentityVerdict {
                identity: RowIdentity::WholeRow,
                proven_mismatch: None,
            },
            skeleton_source_closure: None,
            fingerprint_projections: Default::default(),
            key_scope: None,
            state_downgrade: None,
        }],
        refusals: vec![],
        key_locality: None,
    };
    let trigger = Trigger::UpstreamMutation {
        source: "users".to_string(),
    };

    // No override: an admitted+live technique is preferred over recompute.
    let resolved = resolve_cell_choice(
        plan.cell_for(&trigger),
        &trigger,
        &EffectiveOverride::default(),
        None,
        true,
    )
    .expect("no pin — never refuses");
    assert_eq!(
        resolved,
        ChosenTechnique::Admitted(Technique::ColumnScopedMerge)
    );

    // A pin naming a technique this cell did not admit still refuses —
    // unaffected by the technique-preview builder's wider display-only set.
    let bad_overrides = EffectiveOverride {
        prefer: None,
        technique: Some(CellTechnique::Fold),
    };
    let err = resolve_cell_choice(
        plan.cell_for(&trigger),
        &trigger,
        &bad_overrides,
        None,
        true,
    )
    .expect_err("pinning an unadmitted technique must still refuse");
    assert!(err.to_string().contains("MaintenanceUnboundedFootprint"));

    // `recompute` remains always resolvable.
    let recompute_overrides = EffectiveOverride {
        prefer: None,
        technique: Some(CellTechnique::Recompute),
    };
    let resolved = resolve_cell_choice(
        plan.cell_for(&trigger),
        &trigger,
        &recompute_overrides,
        None,
        true,
    )
    .expect("recompute is always resolvable");
    assert_eq!(resolved, ChosenTechnique::RegionRecompute);
}

/// The repair family's technique preview (`docs/specs/incremental_models.md`
/// §"The repair family"): an admitted `Technique::PerGroupRecompute` cell
/// renders real, illustrative statements built by the SAME emitter +
/// affected-key/candidate builders a live run uses
/// (`smelt_runtime::maintenance_driver::repair_affected_keys_select`/
/// `repair_candidate_select` → `emit_per_group_recompute`) — not the
/// "no live statement builder yet" refusal that stood in for it while the
/// family had no runtime lowering.
#[test]
fn per_group_recompute_preview_renders_statements_for_an_admitted_repair_cell() {
    let (models, source_infos, config) = load_fixture();
    let model = find_model(&models, "daily_events_status");
    let cf = compile_fixture(&config);
    let graph = DependencyGraph::build(models.clone(), None).expect("graph builds");
    let source_timeseries = build_source_timeseries_map(&graph, &source_infos);

    // A repair cell's shape, verbatim from
    // `smelt_logical::maintenance::repair::derive_repair_cell`: a proven
    // group key plus the bounded per-group read slice.
    let mut cell = synthetic_cell(
        Technique::PerGroupRecompute,
        RowIdentity::Key(vec!["user_id".to_string()]),
    );
    cell.scans = vec![smelt_logical::maintenance::ScanClamp {
        source: "raw.user_status".to_string(),
        column: "changed_at".to_string(),
        before: smelt_logical::analysis::source_bounds::Seconds::days(1),
        after: smelt_logical::analysis::source_bounds::Seconds::ZERO,
        write_footprint: None,
    }];

    let diagnostics = build_plan_cell_diagnostics(
        &cell,
        model,
        "main",
        "dev",
        &cf.registry,
        &cf.resolver,
        MaintenanceDialect::DuckDb,
        &["user_id".to_string()],
        &source_timeseries,
        &[],
    );

    let preview = diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == Technique::PerGroupRecompute)
        .expect("every technique has a preview entry");
    assert_eq!(
        preview.admissibility,
        Admissibility::Admitted,
        "the cell's own admitted technique must render as Admitted, got {:?}",
        preview.admissibility
    );
    assert!(
        !preview.statements.is_empty(),
        "the repair preview must render the emitter's statement group, not an empty refusal"
    );
    assert!(
        preview.transactional,
        "the repair group's DELETE+INSERT pair is transactional"
    );
    let joined = preview
        .statements
        .iter()
        .map(|s| s.sql.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("DELETE FROM main.daily_events_status USING")
            && joined.contains("__smelt_affected"),
        "the repair's DELETE must be restricted to the affected-key relation: {joined}"
    );
    assert!(
        joined.contains("main.sources_raw_user_status") && joined.contains("changed_at >= "),
        "the affected-key read must name the mutated source and carry the cell's clamp: {joined}"
    );
}
