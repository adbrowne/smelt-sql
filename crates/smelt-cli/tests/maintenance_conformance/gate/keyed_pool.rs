//! The keyed pool's own gates: end-state equivalence, the retained-departed-keys oracle adjustment, the unclocked-append-only refusal, and order-monotone redelivery idempotence.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use super::keyed_oracle::{classify_keyed, classify_keyed_full, drive_keyed_and_assert};
use super::keyed_support::{
    insert_row_keyed, keyed_case_count, stage_keyed_recipe, stage_keyed_unclocked_append_only,
};
use super::support::snapshot_table_rows;
use smelt_logical::maintenance::Trigger;
use smelt_maintenance_testkit::link_c_harness::base_request;
use smelt_maintenance_testkit::oracle_modes::{
    keyed_end_state_with_retained_departed_keys, KeyedOracleRow,
};
use smelt_maintenance_testkit::recipe::{
    arb_keyed_combiner, arb_keyed_schedule, KeyedCombiner, KeyedRecipe,
};
use smelt_maintenance_testkit::schedule_gen::GenRow;

/// `keyed_pool_upholds_end_state_equivalence` (plan Phase 5 TDD list):
/// keyed recipes (additive + idempotent combiner families, key re-touch
/// across windows) equal the oracle's end state at settled points.
#[test]
fn keyed_pool_upholds_end_state_equivalence() {
    let n = keyed_case_count();
    let mut runner = TestRunner::deterministic();
    let combiner_strat = arb_keyed_combiner();
    let schedule_strat = arb_keyed_schedule();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    let mut admitted_cases = 0;
    for i in 0..n {
        let combiner = combiner_strat.new_tree(&mut runner).unwrap().current();
        let schedule = schedule_strat.new_tree(&mut runner).unwrap().current();
        let recipe = KeyedRecipe::new_window_forward(combiner);

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_keyed_recipe(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("case {i}: keyed recipe {recipe:?} failed to stage: {e}"));

        let plan = classify_keyed(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: keyed recipe {recipe:?} classify failed: {e}"));
        assert!(
            !plan.cells.is_empty(),
            "case {i}: keyed recipe {recipe:?} admitted zero cells — generator/derivation \
             regression"
        );
        admitted_cases += 1;

        rt.block_on(drive_keyed_and_assert(&project, &recipe, &schedule))
            .unwrap_or_else(|e| {
                panic!(
                    "case {i}: keyed recipe {recipe:?} schedule {schedule:?} equivalence check \
                     failed: {e}"
                )
            });
    }

    assert!(
        admitted_cases > 0,
        "N={n} deterministic keyed sample admitted zero cases — generator/derivation regression"
    );
}

/// `retained_departed_keys_adjusts_the_oracle` (plan Phase 5 TDD list):
/// snapshot-reconcile schedules generating deletes compare against oracle
/// rows ∪ retained departed keys (`incremental_shapes.md` §"End-state
/// equivalence"). Two halves: (1) an ADDITIVE-combiner (fold-family) keyed
/// recipe over an unclocked (zero-clocked-driving-source) source still
/// refuses its *targeted* keyed-fold cell fail-loud
/// (`Refusal::NoAdmissibleTechnique`/`Refusal::ScanUnbounded`, named on the
/// plan itself — `maintenance-plan purity`: consumed, not re-derived) —
/// the snapshot-reconcile run shape (`incremental_shapes.md` §"The two run
/// shapes") is supportable now (Phase 3, `docs/plans/20260809-keyed-
/// frontier.md`), but a fold-family column is refused under it per the
/// admission matrix (double-count/observer-semantics reasons) regardless —
/// the universal `Trigger::Backfill`/whole-table-recompute cell every model
/// admits (`incremental_models.md` §"Per-cell admission" — "a recompute is
/// the universal ground-truth reset") stays available as the escape hatch,
/// but no `Trigger::NewData` cell is ever admitted for this source; (2) the
/// pure oracle adjustment that refusal defers to is independently pinned as
/// data (`oracle_modes::keyed_end_state_with_retained_departed_keys`) — the
/// SAME formula [`snapshot_reconcile_plain_overwrite_settles_with_retained_
/// departed_keys`] below exercises end-to-end against the real, now-built
/// executor for the one family the matrix actually admits
/// (plain-overwrite).
#[test]
fn retained_departed_keys_adjusts_the_oracle() {
    let recipe = KeyedRecipe::new_snapshot_reconcile(KeyedCombiner::Additive);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_keyed_recipe(&recipe, &tmp).expect("stage unclocked keyed recipe");

    let (plan, _diags) =
        classify_keyed_full(&project, &recipe).expect("classify unclocked keyed recipe");
    let plan = plan.expect(
        "maintenance_plan_report must still return a plan (the universal \
         Backfill cell), even when the targeted keyed fold is refused",
    );

    assert!(
        !plan.cells.iter().any(
            |c| matches!(&c.trigger, Trigger::NewData { source } if source == &recipe.source.name)
        ),
        "an unclocked (snapshot-reconcile) keyed model must never admit a targeted NewData \
         fold cell today: {plan:#?}"
    );
    assert!(
        plan.refusals.iter().any(|r| matches!(
            r,
            smelt_logical::maintenance::Refusal::NoAdmissibleTechnique { trigger, .. }
                if trigger.contains(&recipe.source.name)
        )),
        "expected a named NoAdmissibleTechnique refusal naming the unclocked driving source, \
         got: {:#?}",
        plan.refusals
    );

    // The pure carve-out formula this refusal defers to.
    let oracle_rows = vec![
        KeyedOracleRow { key: 1, value: 10 },
        KeyedOracleRow { key: 2, value: 20 },
    ];
    let stored_before_snapshot = [
        KeyedOracleRow { key: 1, value: 999 }, // present in both — oracle wins
        KeyedOracleRow { key: 3, value: 30 },  // departed — retained
    ];
    let retained_departed: Vec<KeyedOracleRow> = stored_before_snapshot
        .iter()
        .filter(|stored| !oracle_rows.iter().any(|o| o.key == stored.key))
        .copied()
        .collect();

    let adjusted = keyed_end_state_with_retained_departed_keys(&oracle_rows, &retained_departed);
    assert_eq!(
        adjusted,
        vec![
            KeyedOracleRow { key: 1, value: 10 },
            KeyedOracleRow { key: 2, value: 20 },
            KeyedOracleRow { key: 3, value: 30 },
        ],
        "stored table must equal the oracle's rows plus retained departed keys, exactly \
         once each"
    );
}

/// Plan/classifier-agreement review finding (`docs/plans/
/// 20260809-keyed-frontier.md` Phase 3): `retained_departed_keys_adjusts_
/// the_oracle` (above) already covers an ADDITIVE (`SUM`) keyed recipe over
/// [`KeyedRecipe::new_snapshot_reconcile`]'s `mutable_snapshot`-postured,
/// unclocked driving source — that case refuses via the pre-existing
/// faithful-fold source-posture obligation (`MutableSnapshot` fails
/// obligation 2 regardless of clock), so it never actually exercised the
/// run-shape gate itself. This case swaps the driving source's declared
/// posture to `append_only` (`SourceRecipe::unclocked_append_only_dimension`)
/// while keeping it unclocked — a posture that passes the faithful-fold
/// source-posture obligation on its own — so the ONLY thing that can still
/// refuse a `SUM` fold here is the whole-model run-shape check: this model
/// has no clocked source anywhere (`incremental_shapes.md` §"The two run
/// shapes"), deriving snapshot-reconcile, under which every fold-family
/// column is refused (double-count) regardless of source posture. Before
/// the gate landed, `derive_new_data`'s `Grain::Key` arm admitted a
/// `Technique::KeyedFold` cell here — `smelt explain` showed a cell the
/// runtime classifier (`rules::cumulative::classify_cumulative`,
/// `KeyedSnapshotSourceUnsupportedColumn`) refuses outright.
#[test]
fn snapshot_reconcile_unclocked_append_only_source_with_sum_is_refused() {
    let recipe = KeyedRecipe::new_snapshot_reconcile_unclocked_append_only(KeyedCombiner::Additive);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_keyed_unclocked_append_only(&recipe, &tmp)
        .expect("stage unclocked append-only keyed recipe");

    let (plan, _diags) = classify_keyed_full(&project, &recipe)
        .expect("classify unclocked append-only keyed recipe");
    let plan = plan.expect(
        "maintenance_plan_report must still return a plan (the universal \
         Backfill cell), even when the targeted keyed fold is refused",
    );

    assert!(
        !plan.cells.iter().any(|c| matches!(
            &c.trigger,
            Trigger::NewData { source } if source == &recipe.source.name
        ) || c.technique
            == smelt_logical::maintenance::Technique::KeyedFold),
        "an unclocked (snapshot-reconcile) append-only-postured keyed model must never \
         admit a KeyedFold/NewData cell for a SUM column, regardless of the source's \
         declared MutationProfile: {plan:#?}"
    );
    let refusal_names_snapshot_reconcile_double_count = plan.refusals.iter().any(|r| {
        matches!(
            r,
            smelt_logical::maintenance::Refusal::NoAdmissibleTechnique { trigger, why }
                if trigger.contains(&recipe.source.name)
                    && why.to_lowercase().contains("snapshot-reconcile")
                    && (why.to_lowercase().contains("double-count")
                        || why.to_lowercase().contains("double count"))
        )
    });
    assert!(
        refusal_names_snapshot_reconcile_double_count,
        "expected a NoAdmissibleTechnique refusal naming the snapshot-reconcile \
         double-count reason for source '{}', got: {:#?}",
        recipe.source.name, plan.refusals
    );
}

/// Phase 1 (`docs/plans/20260809-keyed-frontier.md`): the order-monotone
/// overwrite family (`MAX_BY`) grades `Grade::Idempotent`
/// (`crates/smelt-runtime/src/cumulative.rs`'s `WindowedKeyedRule::
/// ledger_grade` doc comment — incumbent-wins re-merge of an
/// already-reflected delta converges) — unlike the additive family
/// (`redelivered_window_refuses_for_additive_keyed`,
/// `crates/smelt-cli/tests/maintenance_conformance/probes.rs`), re-running
/// the SAME window must NOT be refused: no ledger exists for an
/// idempotent-graded cell, and re-merging is harmless by construction.
#[tokio::test]
async fn order_monotone_redelivery_is_idempotent_no_ledger_refusal() {
    let recipe = KeyedRecipe::new_window_forward(KeyedCombiner::OrderMonotone);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_keyed_recipe(&recipe, &tmp).expect("stage order-monotone keyed recipe");

    let plan = classify_keyed(&project, &recipe).expect("classify order-monotone keyed recipe");
    assert!(
        !plan.cells.is_empty(),
        "expected the order-monotone keyed recipe to admit at least one cell: {plan:#?}"
    );

    let d = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    insert_row_keyed(
        &project,
        &recipe,
        &GenRow {
            d,
            id: 1,
            val: Some(5),
        },
    )
    .expect("insert row");

    let mut request = base_request("dev");
    request.start = Some("2024-01-01".to_string());
    request.end = Some("2024-01-02".to_string());
    project
        .run_quiet("keyed-order-monotone-1", request.clone())
        .await
        .expect("first fold of the window must succeed");

    let maintained_after_first = {
        let backend = project.backend().await.expect("backend");
        snapshot_table_rows(backend.as_ref(), &recipe.model_name)
            .await
            .expect("snapshot after first fold")
    };

    // Re-deliver the SAME window: an idempotent-graded cell has no ledger
    // and must succeed, converging to the same stored state.
    project
        .run_quiet("keyed-order-monotone-2", request)
        .await
        .expect(
            "re-running an already-folded order-monotone keyed window must succeed — \
             idempotent-graded cells carry no reprocessing ledger",
        );

    let maintained_after_redelivery = {
        let backend = project.backend().await.expect("backend");
        snapshot_table_rows(backend.as_ref(), &recipe.model_name)
            .await
            .expect("snapshot after redelivery")
    };
    assert_eq!(
        maintained_after_first, maintained_after_redelivery,
        "redelivering an already-folded window must converge to byte-identical state, never \
         double-apply the overwrite"
    );
}
