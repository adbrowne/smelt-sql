//! BigQuery twin of `maintenance_conformance/harness_self_check.rs` plus the
//! two BigQuery-specific harness properties Phase 5 requires proved before
//! any family wrapper is trusted
//! (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 5 TDD
//! list): a fresh dataset per case, and a green skip — never a failure —
//! when credentials are absent. `oracle_flags_a_seeded_divergence_on_bigquery`
//! is written and observed FIRST, before any `<family>_on_bigquery` wrapper,
//! per the plan's explicit ordering: without it a green leg is
//! indistinguishable from a vacuous one.

use smelt_maintenance_testkit::families::{harness_self_check, ConformanceBackend};
use smelt_maintenance_testkit::recipe::bq_project;

use crate::backend::{skip_reason_for_project, BigQueryConformanceBackend};

/// `oracle_flags_a_seeded_divergence_on_bigquery`.
///
/// Order matters (plan Phase 5): this is the first BigQuery test written and
/// must be observed FAILING the oracle (i.e. the corruption is actually
/// caught) before any `<family>_on_bigquery` wrapper is trusted — otherwise a
/// standing green leg could just mean the oracle is vacuously true on
/// BigQuery too. Corruption goes through `Backend::execute_sql`
/// (`BigQueryConformanceBackend::corrupt_sql`), never a raw write.
///
/// **Live run (2026-08-17): FAILED — and NOT on the corruption check.** The
/// pre-corruption green run itself fails, on `STracker::materialize_s_as_view`'s
/// `VALUES`-table-constructor SQL (GoogleSQL rejects it — see `main.rs`'s
/// doc comment point 1), before `run_oracle_flags_a_seeded_divergence` ever
/// reaches the `execute_sql` corruption step. This means the leg's
/// non-vacuousness is NOT yet established on BigQuery — the test correctly
/// failed loud rather than passing vacuously, but it has not yet DONE its
/// job of proving the oracle catches a real divergence there. Re-run this
/// test alone, first, once the `VALUES` gap is fixed.
#[test]
fn oracle_flags_a_seeded_divergence_on_bigquery() {
    let b = BigQueryConformanceBackend::new("harness_self_check");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping oracle_flags_a_seeded_divergence_on_bigquery");
        return;
    }
    b.preflight_or_panic();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(harness_self_check::run_oracle_flags_a_seeded_divergence(&b))
        .expect("harness self-check failed on BigQuery");
}

/// `skips_green_when_SMELT_BQ_PROJECT_is_unset`.
///
/// Asserts the pure predicate behind `ConformanceBackend::skip_reason`
/// directly, rather than mutating the process's `SMELT_BQ_PROJECT`
/// environment variable — this binary runs with `--test-threads=1`, but a
/// single process-global env var is still the wrong knob to poke from a unit
/// test when the underlying logic is already a pure function
/// (`backend::skip_reason_for_project`). Also asserts the CURRENT ambient
/// environment agrees, so the property is checked against what this test run
/// is actually doing, not only in the abstract.
#[test]
fn skips_green_when_smelt_bq_project_is_unset() {
    assert_eq!(
        skip_reason_for_project(None),
        Some("SMELT_BQ_PROJECT unset".to_string()),
        "an absent project must be a named skip, never a silent pass or a failure"
    );
    assert_eq!(
        skip_reason_for_project(Some("some-project")),
        None,
        "a present project must not itself trigger a skip — only an absent one does; a bad \
         token surfaces as a loud failure elsewhere, never here"
    );

    let ambient = bq_project();
    let b = BigQueryConformanceBackend::new("skip-check");
    assert_eq!(
        b.skip_reason(),
        skip_reason_for_project(ambient.as_deref()),
        "the live ConformanceBackend's skip_reason must agree with the pure predicate for \
         whatever SMELT_BQ_PROJECT is ACTUALLY set to in this test run"
    );
}

/// `each_case_gets_a_fresh_dataset`.
///
/// The dataset-naming half (pure, no credentials needed) proves two cases
/// resolve to different datasets — mirrors
/// `crate::recipe::conformance_dataset_is_derived_not_threaded`, scoped to
/// THIS family's own `BigQueryConformanceBackend`. The live half (gated on
/// credentials, like every other test here) additionally proves each
/// dataset actually gets created and dropped — `BigQueryBackend::new`
/// creates the dataset on connect (`requires_schema_init`,
/// `docs/specs/multi_backend.md` §Semantics "Session initialization"), and
/// this test drops both explicitly on the way out, mirroring
/// `crates/smelt-cli/tests/common/mod.rs::drop_bq_dataset`'s `DROP SCHEMA IF
/// EXISTS ... CASCADE` shape — the same two-layer cleanup (explicit drop
/// backstopped by the dataset's own default table expiration) an
/// interrupted run relies on.
#[test]
fn each_case_gets_a_fresh_dataset() {
    let b = BigQueryConformanceBackend::new("fresh-dataset-check");

    // Pure half: always runs, no credentials required.
    let case0 = b.target(0);
    let case1 = b.target(1);
    assert_ne!(
        case0, case1,
        "two cases in one run must resolve to different datasets"
    );

    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping the live half of each_case_gets_a_fresh_dataset");
        return;
    }
    b.preflight_or_panic();

    let smelt_maintenance_testkit::recipe::ConformanceTarget::BigQuery { dataset: dataset0 } =
        case0
    else {
        panic!("BigQueryConformanceBackend::target must return ConformanceTarget::BigQuery");
    };
    let smelt_maintenance_testkit::recipe::ConformanceTarget::BigQuery { dataset: dataset1 } =
        case1
    else {
        panic!("BigQueryConformanceBackend::target must return ConformanceTarget::BigQuery");
    };

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        // Connecting creates the dataset (session-init obligation) — proves
        // each case's dataset is independently creatable, then drops it.
        for dataset in [&dataset0, &dataset1] {
            let backend = smelt_maintenance_testkit::link_c_harness::open_bigquery_backend(dataset)
                .await
                .unwrap_or_else(|e| panic!("open BigQuery backend for dataset {dataset:?}: {e}"));
            let project =
                bq_project().expect("SMELT_BQ_PROJECT must be set (skip already checked)");
            backend
                .execute_sql(&format!(
                    "DROP SCHEMA IF EXISTS `{project}.{dataset}` CASCADE"
                ))
                .await
                .unwrap_or_else(|e| panic!("drop dataset {dataset:?} on the way out: {e}"));
        }
    });
}

/// `corrupt_sql_targets_the_case_under_test`.
///
/// Pure (no credentials): the seeded-divergence UPDATE must name the dataset
/// of the case it is handed, not a constant. BigQuery is the one backend
/// whose schema is per-case, so a hardcoded case here would aim the mutation
/// at another case's dataset — and the resulting failure would read as "the
/// oracle failed to catch a divergence" rather than "the divergence was
/// seeded in the wrong place". Guards the `case` parameter on
/// `ConformanceBackend::corrupt_sql` against being dropped again.
#[test]
fn corrupt_sql_targets_the_case_under_test() {
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::TestRunner;
    use smelt_maintenance_testkit::recipe::{arb_recipe, ConstructKind, RecipePool};

    let b = BigQueryConformanceBackend::new("corrupt-sql-check");
    let mut runner = TestRunner::deterministic();
    let recipe = arb_recipe(RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    })
    .new_tree(&mut runner)
    .unwrap()
    .current();

    for case in [0usize, 1, 7] {
        let sql = b.corrupt_sql(case, &recipe);
        assert!(
            sql.contains(&format!("{}.{}", b.schema(case), recipe.model_name)),
            "corrupt_sql({case}) must target case {case}'s own dataset \
             ({}), got: {sql}",
            b.schema(case)
        );
    }

    assert_ne!(
        b.corrupt_sql(0, &recipe),
        b.corrupt_sql(1, &recipe),
        "two cases must not share one corruption target"
    );
}
