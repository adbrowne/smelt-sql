use super::*;

/// `docs/specs/model_properties.md` §Surface "Derived proofs": for a real
/// fixture model with a known GROUP BY grain (`daily_revenue` groups by
/// `revenue_date, user_id` and aggregates with `COUNT`/`SUM`/`AVG`/`MIN`/
/// `MAX`), assert every field `PropertySet` actually carries — columns,
/// grain, functional dependencies, determinism, comparability,
/// discriminants, row identity, and source bounds, i.e. everything
/// reachable from the three already-derived per-model calls
/// (`model_property_vector`, `row_identity`, `derive_model_bounds`) — is
/// present and non-trivial on the returned `ModelDiagnostics`.
///
/// This is **not** the full `model_properties.md` catalogue: event-time
/// monotonicity trace, partition alignment, skeleton-role, footprint
/// reflection, faithful-fold conditions, grain-alignment, standalone
/// fingerprint/cardinality derivation, and model-scoped declarations are
/// not yet covered by `PropertySet` (`docs/specs/ui_model_diagnostics.md`
/// §Known Divergences).
#[test]
fn properties_cover_derivable_catalogue_subset() {
    let (models, source_infos, config) = load_fixture();
    let model = find_model(&models, "daily_revenue");
    let graph = DependencyGraph::build(models.clone(), None).expect("graph builds");
    let upstream = graph.get_upstream(&model.canonical_path());
    let bound_ctx = bound_ctx_for(model, &source_infos);
    let cf = compile_fixture(&config);
    let source_timeseries = build_source_timeseries_map(&graph, &source_infos);

    // `daily_revenue` declares no frontmatter (no maintenance plan) — this
    // test only exercises the property-set half of the builder.
    let diagnostics = build_model_diagnostics(
        model,
        &models,
        &upstream,
        &source_infos,
        &bound_ctx,
        &[],
        "main",
        "dev",
        &cf.registry,
        &cf.resolver,
        MaintenanceDialect::DuckDb,
        &source_timeseries,
        &[],
        &[],
        &[],
        &[],
        None,
    )
    .expect("diagnostics build succeeds for a real fixture model");

    let props = &diagnostics.profile.properties;

    // Columns: every projected output column.
    assert!(
        props.columns.contains(&"revenue_date".to_string())
            && props.columns.contains(&"user_id".to_string())
            && props.columns.contains(&"total_revenue".to_string()),
        "columns should list the model's projected outputs, got {:?}",
        props.columns
    );

    // Grain: `GROUP BY 1, 2` proves a `(revenue_date, user_id)` key.
    assert!(
        !props.grain.keys.is_empty(),
        "grain should be proven from the model's GROUP BY 1, 2"
    );

    // Functional dependencies: grain implies key -> every other column.
    assert!(
        !props.functional_dependencies.is_empty(),
        "functional dependencies should be implied by the proven grain"
    );

    // Determinism: every projected column gets a verdict.
    assert_eq!(
        props.determinism.len(),
        props.columns.len(),
        "every column should carry a determinism verdict"
    );

    // Comparability: every projected column gets a verdict, and a plain
    // aggregate output (`total_revenue`) is `Comparable` (pure function of
    // processed inputs).
    assert_eq!(
        props.comparability.len(),
        props.columns.len(),
        "every column should carry a comparability verdict"
    );
    let total_revenue_comparability = props
        .comparability
        .iter()
        .find(|c| c.output == "total_revenue")
        .expect("total_revenue should have a comparability verdict");
    assert_eq!(
        total_revenue_comparability.comparability,
        Comparability::Comparable
    );

    // Discriminants: the five aggregate outputs (COUNT/SUM/AVG/MIN/MAX) each
    // carry algebraic discriminants.
    assert_eq!(
        props.discriminants.len(),
        5,
        "each of COUNT/SUM/AVG/MIN/MAX should carry discriminants, got {:?}",
        props.discriminants
    );

    // Region row identity: no declared unique_key on this model, so the
    // proven GROUP BY grain becomes the row identity.
    assert!(
        matches!(props.row_identity.identity, RowIdentity::Key(_)),
        "row identity should fall back to the proven grain key, got {:?}",
        props.row_identity
    );

    // Unified bound/reach: the model's one timeseries source
    // (`sources.raw.transactions`) should have a bound-derivation entry.
    assert!(
        props.source_bounds.contains_key("sources.raw.transactions"),
        "source_bounds should carry an entry for the model's timeseries source, got {:?}",
        props.source_bounds.keys().collect::<Vec<_>>()
    );
}

/// `docs/specs/ui_model_diagnostics.md` §Design "Why a shared smelt-runtime
/// builder": `build_relation_contract` is now derived exactly once, in this
/// crate — `smelt-cli::explain::build_relation_contract` is a `pub use`
/// re-export of the very same function, not a second implementation. This
/// test pins the exact shape produced for a real fixture model with a
/// declared source edge, guarding against a future accidental fork.
#[test]
fn relation_contract_matches_existing_explain_output() {
    let (models, source_infos, _config) = load_fixture();
    let model = find_model(&models, "daily_revenue");
    let graph = DependencyGraph::build(models.clone(), None).expect("graph builds");
    let upstream = graph.get_upstream(&model.canonical_path());

    let (own_contract, edges) = build_relation_contract(model, &models, &upstream, &source_infos);

    // `daily_revenue` declares no `timeseries:`/`unique_key` of its own.
    assert!(own_contract.clock.is_none());
    assert!(own_contract.identity.is_none());

    // Its one inbound edge is the declared `sources.raw.transactions`
    // source, which carries a `timeseries:` clock (`transaction_timestamp`,
    // day granularity).
    let edge = edges
        .iter()
        .find(|e| e.name == "sources.raw.transactions")
        .expect("daily_revenue should have a sources.raw.transactions inbound edge");
    let clock = edge
        .contract
        .clock
        .as_ref()
        .expect("the transactions source declares a timeseries clock");
    assert_eq!(clock.event_time_column, "transaction_timestamp");
    assert_eq!(clock.partition_column, "transaction_timestamp");

    // `smelt-cli::explain::build_relation_contract` is a `pub use`
    // re-export of this exact function (not a second implementation) —
    // calling it on the same inputs must produce an identical result,
    // byte-for-byte.
    let (cli_own_contract, cli_edges) =
        smelt_cli::explain::build_relation_contract(model, &models, &upstream, &source_infos);
    assert_eq!(own_contract, cli_own_contract);
    assert_eq!(edges, cli_edges);
}

/// `docs/specs/ui_model_diagnostics.md` §Constraints: "must not require a
/// live backend connection or ledger state". `build_model_diagnostics`'s
/// signature takes no target/backend/connection argument at all — this test
/// documents and locks that shape by calling it with only disk-derived
/// facts and asserting success.
#[test]
fn no_live_backend_required() {
    let (models, source_infos, config) = load_fixture();
    let model = find_model(&models, "daily_revenue");
    let graph = DependencyGraph::build(models.clone(), None).expect("graph builds");
    let upstream = graph.get_upstream(&model.canonical_path());
    let bound_ctx = bound_ctx_for(model, &source_infos);
    let cf = compile_fixture(&config);
    let source_timeseries = build_source_timeseries_map(&graph, &source_infos);

    let result = build_model_diagnostics(
        model,
        &models,
        &upstream,
        &source_infos,
        &bound_ctx,
        &[],
        "main",
        "dev",
        &cf.registry,
        &cf.resolver,
        MaintenanceDialect::DuckDb,
        &source_timeseries,
        &[],
        &[],
        &[],
        &[],
        None,
    );
    assert!(
        result.is_ok(),
        "diagnostics build must succeed without any live backend/target: {:?}",
        result.err()
    );
}

// ---------------------------------------------------------------------
// Phase 2b: technique previews + admissibility
// (`docs/plans/20260725-ui-model-diagnostics.md` §"Phase 2b";
// `docs/specs/ui_model_diagnostics.md` §Semantics "Technique preview set" /
// "Admissibility verdict")
// ---------------------------------------------------------------------
