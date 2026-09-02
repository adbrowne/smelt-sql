//! `derive_fold_spec` (`crates/smelt-db/src/queries/maintenance.rs`) must
//! stay in lockstep with the runtime classifier
//! (`smelt_logical::rules::cumulative::classify_order_monotone_column`) on
//! whether an order-monotone (`MAX_BY`/`MIN_BY`, `ArgMax`/`ArgMin`) column
//! is admitted into a fold — otherwise `smelt explain`/LSP diagnostics
//! report a `KeyedFold` cell the runtime then refuses with
//! `KeyedUnknownCombiner` (a fail-loud discipline violation: a compile-time
//! false positive). Both layers admit an `ArgMax`/`ArgMin` column on hidden
//! `(v, o)` state (`docs/outcomes/20260809-rung2-state-shapes` row 5) — no
//! companion projection is required; only the wrong-arity shape refuses.
//!
//! Spec: `docs/specs/incremental_shapes.md` §"The column-family catalogue",
//! §"Statement emission (single owner)".

use smelt_db::queries::maintenance::derive_fold_spec;
use smelt_types::SqlFunction;

/// A `MAX_BY(value, ordering)` projection with NO companion `MAX(ordering)`
/// projection in the same SELECT list is admitted into the derived
/// `FoldSpec` — it decomposes to hidden state, the same shape the runtime
/// classifier now admits.
#[test]
fn max_by_without_companion_is_admitted() {
    let sql = "SELECT user_id, MAX_BY(status, updated_at) AS status \
               FROM smelt.sources.events GROUP BY user_id";
    let spec = derive_fold_spec(sql, &[]).expect("companion-less MAX_BY must be admitted");
    assert!(spec
        .add_columns
        .iter()
        .any(|(alias, f)| alias == "status" && *f == SqlFunction::ArgMax));
}

/// `MIN_BY` without a companion `MIN(ordering)` is admitted the same way.
#[test]
fn min_by_without_companion_is_admitted() {
    let sql = "SELECT user_id, MIN_BY(status, updated_at) AS status \
               FROM smelt.sources.events GROUP BY user_id";
    let spec = derive_fold_spec(sql, &[]).expect("companion-less MIN_BY must be admitted");
    assert!(spec
        .add_columns
        .iter()
        .any(|(alias, f)| alias == "status" && *f == SqlFunction::ArgMin));
}

/// A `MAX_BY`/`MIN_BY` call of the wrong arity still refuses (returns
/// `None`) — fail-closed survives the admission widen.
#[test]
fn max_by_wrong_arity_is_not_admitted() {
    let sql = "SELECT user_id, MAX_BY(status) AS status \
               FROM smelt.sources.events GROUP BY user_id";
    assert!(derive_fold_spec(sql, &[]).is_none());
}

/// Degenerate self-companion: `MAX_BY(x, x)` — the value expression IS the
/// ordering expression. Admitted identically to the runtime classifier.
#[test]
fn max_by_self_companion_is_admitted() {
    let sql = "SELECT user_id, MAX_BY(event_ts, event_ts) AS first_seen \
               FROM smelt.sources.events GROUP BY user_id";
    let spec = derive_fold_spec(sql, &[]).expect("self-companion MAX_BY must be admitted");
    assert!(spec
        .add_columns
        .iter()
        .any(|(alias, f)| alias == "first_seen" && *f == SqlFunction::ArgMax));
}

/// Plan/classifier agreement for the snapshot-reconcile run shape
/// (`docs/specs/incremental_shapes.md` §"The two run shapes"): a keyed
/// model with a `SUM` (additive-fold) column, over a source declared
/// `mutation_profile: append_only` but with NO clocked source anywhere in
/// the model (`SourceFacts::partition_col` is `None` for every declared
/// source) — the snapshot-reconcile shape — must be refused at the plan
/// layer exactly as `rules::cumulative::classify_cumulative` refuses it
/// (`KeyedSnapshotSourceUnsupportedColumn`: "re-folding state
/// double-counts — a mutable snapshot is not a replayable,
/// retraction-free event feed"). Before the fix, `derive_new_data`'s
/// `Grain::Key` arm consulted only the triggering source's declared
/// `MutationProfile` (append-only passes the faithful-fold source-posture
/// condition on posture alone, clock-independent) and admitted a
/// `Technique::KeyedFold` cell here — `smelt explain` showed an admitted
/// cell the runtime classifier refuses outright, a plan/runtime-agreement
/// violation (execution stayed safe only because both dispatch paths
/// re-classify independently).
#[test]
fn sum_over_unclocked_append_only_source_is_refused_at_plan_layer() {
    use smelt_core::config::{Grain, RefreshStrategy};
    use smelt_core::ModelMetadata;
    use smelt_logical::maintenance::{MutationProfile, Refusal, SourceFacts, Technique};

    let sql = "SELECT user_id, SUM(amount) AS total FROM smelt.sources.dim_table \
               GROUP BY user_id";
    let metadata = ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(Grain::Key),
        ..Default::default()
    };
    let sources = vec![SourceFacts {
        name: "dim_table".to_string(),
        mutation: MutationProfile::AppendOnly,
        partition_col: None,
        unique_key: vec![],
        allow_full_scan: false,
    }];

    let result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        sql,
        "main.dim_table_totals",
        &metadata,
        &sources,
        &std::collections::HashSet::new(),
        None,
        &[],
        &[],
        &std::collections::BTreeMap::new(),
        None,
    )
    .expect("refresh: incremental model must derive a plan");

    assert!(
        !result
            .plan
            .cells
            .iter()
            .any(|c| c.technique == Technique::KeyedFold),
        "a SUM fold over a wholly-unclocked (snapshot-reconcile) model must never admit \
         a KeyedFold cell, regardless of the triggering source's declared posture; got \
         cells {:?}",
        result.plan.cells
    );

    let refusal_names_snapshot_reconcile = result.plan.refusals.iter().any(|r| match r {
        Refusal::NoAdmissibleTechnique { why, .. } => {
            let why_lower = why.to_lowercase();
            why_lower.contains("snapshot-reconcile")
                && (why_lower.contains("double-count") || why_lower.contains("double count"))
        }
        #[allow(unreachable_patterns)]
        _ => false,
    });
    assert!(
        refusal_names_snapshot_reconcile,
        "expected a refusal naming the snapshot-reconcile double-count reason \
         (mirroring KeyedSnapshotSourceUnsupportedColumn); got refusals {:?}",
        result.plan.refusals
    );
}

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

/// `derive_fold_spec` must stay in lockstep with the runtime classifier on
/// the decomposed-fold family (`AVG`/`STDDEV_*`/`VAR_*`,
/// `docs/outcomes/20260809-rung2-state-shapes` row 7) exactly as it already
/// does for `ArgMax`/`ArgMin` above: an `AVG` column is admitted into the
/// `FoldSpec`, a wrong-arity or `DISTINCT` `AVG` refuses the whole
/// derivation, and the derived plan carries a real `Technique::KeyedFold`
/// cell (no `NoAdmissibleTechnique` refusal) for it.
#[test]
fn avg_model_derives_fold_spec_and_keyed_fold_cell() {
    use smelt_core::config::{Grain, Granularity, RefreshStrategy};
    use smelt_core::ModelMetadata;
    use smelt_logical::maintenance::{MutationProfile, SourceFacts, Technique};

    let sql = "SELECT device_id, AVG(amount) AS avg_amount \
               FROM smelt.sources.events GROUP BY device_id";
    let spec = derive_fold_spec(sql, &[]).expect("AVG must be admitted into the FoldSpec");
    assert!(spec
        .add_columns
        .iter()
        .any(|(alias, f)| alias == "avg_amount" && *f == SqlFunction::Avg));

    let metadata = ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(Grain::Key),
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
        "main.device_avg_amount",
        &metadata,
        &sources,
        &std::collections::HashSet::new(),
        Some(Granularity::Day),
        &[],
        &[],
        &std::collections::BTreeMap::new(),
        None,
    )
    .expect("refresh: incremental model must derive a plan");
    assert!(
        result
            .plan
            .cells
            .iter()
            .any(|c| c.technique == Technique::KeyedFold),
        "expected a KeyedFold cell for the AVG column; got cells {:?} and refusals {:?}",
        result.plan.cells,
        result.plan.refusals
    );
}

/// A wrong-arity `AVG` call refuses the whole `FoldSpec` derivation — fail-
/// closed, mirroring [`max_by_wrong_arity_is_not_admitted`].
#[test]
fn avg_wrong_arity_is_not_admitted_into_fold_spec() {
    let sql = "SELECT device_id, AVG(amount, weight) AS avg_amount \
               FROM smelt.sources.events GROUP BY device_id";
    assert!(derive_fold_spec(sql, &[]).is_none());
}

/// `AVG(DISTINCT ...)` is holistic — refuses the whole derivation, the same
/// as the runtime classifier's `decompose_to_state` refusal.
#[test]
fn avg_distinct_is_not_admitted_into_fold_spec() {
    let sql = "SELECT device_id, AVG(DISTINCT amount) AS avg_amount \
               FROM smelt.sources.events GROUP BY device_id";
    assert!(derive_fold_spec(sql, &[]).is_none());
}
