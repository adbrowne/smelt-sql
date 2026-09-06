use smelt_db::queries::maintenance::derive_fold_spec;
use smelt_types::SqlFunction;

/// Phase 4 (`docs/plans/20260809-keyed-frontier.md`) plan/runtime agreement:
/// a `COALESCE(MAX(col))` once-write column with a declared functional
/// dependency is admitted into a `FoldSpec` (`SqlFunction::Coalesce`) —
/// exactly the shape `rules::cumulative::classify_cumulative` admits with
/// `CrossPartitionCombiner::OnceWrite` — via the SAME shared helper
/// (`rules::cumulative::classify_once_write`), so `smelt explain`/LSP
/// diagnostics never show an admission the runtime refuses.
#[test]
fn once_write_with_declared_fd_is_admitted_into_fold_spec() {
    use smelt_core::config::FunctionalDependency;

    // The declaration names the coalesced value's SOURCE column
    // (`signup_referrer`), never the projection's output alias
    // (`first_referrer`) — an alias-matched declaration would be vacuous
    // (the model's own GROUP BY key determines its aliases by construction).
    let sql = "SELECT device_id, COALESCE(MAX(signup_referrer)) AS first_referrer \
               FROM smelt.sources.events GROUP BY device_id";
    let fds = vec![FunctionalDependency {
        key: vec!["device_id".to_string()],
        determines: "signup_referrer".to_string(),
    }];
    let spec =
        derive_fold_spec(sql, &fds).expect("declared-FD-backed once-write column must be admitted");
    assert!(spec
        .add_columns
        .iter()
        .any(|(alias, f)| alias == "first_referrer" && *f == SqlFunction::Coalesce));

    // The same declaration naming the ALIAS instead proves nothing and must
    // NOT admit.
    let alias_fds = vec![FunctionalDependency {
        key: vec!["device_id".to_string()],
        determines: "first_referrer".to_string(),
    }];
    assert!(
        derive_fold_spec(sql, &alias_fds).is_none(),
        "an alias-matched functional dependency is vacuous and must not admit once-write"
    );
}

/// Plan/runtime agreement: a fallback argument (`COALESCE(MAX(col),
/// <literal>)`) admits onto decomposed `(value, written)` state
/// (`docs/outcomes/20260809-rung2-state-shapes` row 6) — the plan layer
/// admits it exactly as the runtime classifier does, via the same shared
/// `classify_once_write` helper.
#[test]
fn once_write_with_a_literal_fallback_is_admitted_into_fold_spec() {
    use smelt_core::config::FunctionalDependency;

    let sql = "SELECT device_id, COALESCE(MAX(signup_referrer), 'unknown') AS first_referrer \
               FROM smelt.sources.events GROUP BY device_id";
    let fds = vec![FunctionalDependency {
        key: vec!["device_id".to_string()],
        determines: "signup_referrer".to_string(),
    }];
    let spec = derive_fold_spec(sql, &fds)
        .expect("a fallback-bearing once-write column must be admitted onto decomposed state");
    assert!(spec
        .add_columns
        .iter()
        .any(|(alias, f)| alias == "first_referrer" && *f == SqlFunction::Coalesce));
}

/// Multi-candidate spellings (`COALESCE(MAX(a), MAX(b))`) also admit — the
/// plan layer and runtime classifier stay in lockstep since both delegate
/// to `classify_once_write`.
#[test]
fn once_write_multi_candidate_is_admitted_into_fold_spec() {
    use smelt_core::config::FunctionalDependency;

    let sql = "SELECT device_id, COALESCE(MAX(signup_referrer), MAX(fallback_referrer)) \
               AS first_referrer FROM smelt.sources.events GROUP BY device_id";
    let fds = vec![
        FunctionalDependency {
            key: vec!["device_id".to_string()],
            determines: "signup_referrer".to_string(),
        },
        FunctionalDependency {
            key: vec!["device_id".to_string()],
            determines: "fallback_referrer".to_string(),
        },
    ];
    let spec = derive_fold_spec(sql, &fds)
        .expect("a multi-candidate once-write column must be admitted onto decomposed state");
    assert!(spec
        .add_columns
        .iter()
        .any(|(alias, f)| alias == "first_referrer" && *f == SqlFunction::Coalesce));
}

/// The mirror negative case: with NO declared functional dependency, the
/// same `COALESCE(MAX(col))` column has no once-write provenance proof
/// and must NOT be admitted into a `FoldSpec` — mirroring the runtime
/// classifier's `KeyedOnceWriteUnproven` refusal.
#[test]
fn once_write_without_declared_fd_is_not_admitted_into_fold_spec() {
    let sql = "SELECT device_id, COALESCE(MAX(signup_referrer)) AS signup_referrer \
               FROM smelt.sources.events GROUP BY device_id";
    assert!(
        derive_fold_spec(sql, &[]).is_none(),
        "an undeclared once-write column has no fold candidate — the runtime classifier \
         refuses this exact SQL with KeyedOnceWriteUnproven"
    );

    // The discriminating case: a genuine fold column (`SUM`) alongside the
    // unproven once-write column. Dropping the unproven column silently would
    // leave `Some(FoldSpec { total })` here — a plan-layer `KeyedFold`
    // admission the runtime classifier refuses with `KeyedOnceWriteUnproven`.
    // An unproven once-write column must refuse the WHOLE derivation, exactly
    // like an unrecognised combiner does.
    let mixed = "SELECT device_id, SUM(amount) AS total, \
                 COALESCE(MAX(signup_referrer)) AS first_referrer \
                 FROM smelt.sources.events GROUP BY device_id";
    assert!(
        derive_fold_spec(mixed, &[]).is_none(),
        "an unproven once-write column must refuse the whole FoldSpec, not be dropped \
         while the SUM column still derives a KeyedFold admission the runtime refuses"
    );
}

/// A key-derived once-write column (`COALESCE(<unique_key column>, ...)`) is
/// admitted into a `FoldSpec` without any declared functional dependency —
/// mirrors `classify_cumulative`'s key-derived route.
#[test]
fn key_derived_once_write_is_admitted_into_fold_spec_without_declaration() {
    let sql = "SELECT device_id, COALESCE(device_id, 'n/a') AS first_seen_device \
               FROM smelt.sources.events GROUP BY device_id";
    let spec = derive_fold_spec(sql, &[]).expect("key-derived once-write column must be admitted");
    assert!(spec
        .add_columns
        .iter()
        .any(|(alias, f)| alias == "first_seen_device" && *f == SqlFunction::Coalesce));
}

/// End-to-end plan-layer agreement: a window-forward keyed model with a
/// declared-FD-backed once-write column derives a real `Technique::
/// KeyedFold` cell — the plan admits the same shape the runtime executes,
/// not merely a `FoldSpec` in isolation.
#[test]
fn once_write_column_derives_keyed_fold_cell_at_plan_layer() {
    use smelt_core::config::{FunctionalDependency, Grain, Granularity, RefreshStrategy};
    use smelt_core::ModelMetadata;
    use smelt_logical::maintenance::{MutationProfile, SourceFacts, Technique};

    let sql = "SELECT device_id, COALESCE(MAX(signup_referrer)) AS signup_referrer \
               FROM smelt.sources.events GROUP BY device_id";
    let metadata = ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(Grain::Key),
        functional_dependencies: vec![FunctionalDependency {
            key: vec!["device_id".to_string()],
            determines: "signup_referrer".to_string(),
        }],
        ..Default::default()
    };
    let sources = vec![SourceFacts {
        name: "events".to_string(),
        mutation: MutationProfile::AppendOnly,
        partition_col: Some("event_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }];

    let result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        sql,
        "main.device_first_touch",
        &metadata,
        &sources,
        &std::collections::HashSet::new(),
        Some(Granularity::Day),
        &[],
        &[],
        &std::collections::BTreeMap::new(),
        None,
        None,
        &[],
    )
    .expect("refresh: incremental model must derive a plan");

    assert!(
        result
            .plan
            .cells
            .iter()
            .any(|c| c.technique == Technique::KeyedFold),
        "expected a KeyedFold cell for the declared-FD-backed once-write column; got cells \
         {:?} and refusals {:?}",
        result.plan.cells,
        result.plan.refusals
    );
}
