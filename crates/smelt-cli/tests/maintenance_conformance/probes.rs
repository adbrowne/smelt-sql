//! Plan-claim probes (design doc
//! `docs/research/20260711-generative-maintenance-conformance.md` §7
//! "Plan-claim probes — checking that derived properties hold";
//! `docs/plans/20260712-generative-maintenance-conformance.md` Phase 4): a
//! direct runtime check that a derived plan claim actually holds, beyond
//! end-state equivalence alone — end-state equivalence can miss a claim
//! being wrong in a compensating way.

use chrono::NaiveDate;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_logical::maintenance::{Technique, Trigger};
use smelt_maintenance_testkit::link_c_harness::base_request;
use smelt_maintenance_testkit::recipe::{
    arb_recipe, ConstructKind, KeyedCombiner, KeyedRecipe, MutableEnrichedRecipe, RecipePool,
};
use smelt_maintenance_testkit::schedule_gen::GenRow;
use smelt_maintenance_testkit::verdict::{classify, Verdict};
use smelt_state::reconciliation::Region;

use crate::gate::{
    classify_keyed, classify_mixed, insert_fact_row, insert_row_keyed, stage_keyed_recipe,
    stage_mixed_recipe, stage_recipe,
};

/// Read back `SELECT d, id, val, attr FROM main.<model_name> ORDER BY id` —
/// every output column, ordered by the fact row's own key for stable
/// row-by-row comparison.
fn read_maintained_rows(
    project: &smelt_maintenance_testkit::link_c_harness::LinkCProject,
    recipe: &MutableEnrichedRecipe,
) -> Vec<(String, i64, i64, i64)> {
    let conn = project.connect().expect("connect for probe read-back");
    let mut stmt = conn
        .prepare(&format!(
            "SELECT CAST({d} AS VARCHAR), {id}, {val}, {attr} FROM main.{model} ORDER BY {id}",
            d = recipe.fact.clock_column,
            id = recipe.fact.key_column,
            val = recipe.fact.payload_column,
            attr = recipe.dimension.payload_column,
            model = recipe.model_name,
        ))
        .expect("prepare maintained read-back");
    stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })
    .expect("query maintained rows")
    .collect::<Result<Vec<_>, _>>()
    .expect("collect maintained rows")
}

/// `dimension_mutation_touches_only_sensitive_groups` (plan Phase 4 TDD
/// list; design §7 row 3): for an admitted column-scoped-merge cell,
/// mutating only the dimension leaves columns in groups not sensitive to it
/// unchanged. Two fact rows land in the SAME window, referencing two
/// DIFFERENT dimension keys, so a single catch-up run's column-scoped merge
/// (full-input read under `allow_full_scan`) recomputes BOTH rows' `attr` —
/// but only the mutated key's value should actually change.
#[tokio::test]
async fn dimension_mutation_touches_only_sensitive_groups() {
    let recipe = MutableEnrichedRecipe::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_mixed_recipe(&recipe, &tmp).expect("stage mixed recipe");

    // A probe that can't structurally apply is skipped explicitly, counted
    // — never silently vacuous (design §7: "Probes are per-case opt-in...
    // skipped explicitly").
    let plan = classify_mixed(&project, &recipe).expect("classify mixed recipe");
    let cell = match plan.cell_for(&Trigger::UpstreamMutation {
        source: recipe.dimension.name.clone(),
    }) {
        Some(cell) => cell,
        None => {
            eprintln!(
                "SKIP dimension_mutation_touches_only_sensitive_groups: no UpstreamMutation \
                 cell admitted for {:?} — probe structurally does not apply",
                recipe.model_name
            );
            return;
        }
    };
    assert_eq!(
        cell.technique,
        Technique::ColumnScopedMerge,
        "the admitted UpstreamMutation cell must be the column-scoped merge this probe checks"
    );

    let d: NaiveDate = NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    insert_fact_row(&project, &recipe, &GenRow { d, id: 1, val: 11 }).expect("insert fact row 1");
    insert_fact_row(&project, &recipe, &GenRow { d, id: 2, val: 22 }).expect("insert fact row 2");

    let mut request = base_request("dev");
    request.start = Some("2024-01-01".to_string());
    request.end = Some("2024-01-02".to_string());
    project
        .run_quiet("probe-create", request.clone())
        .await
        .expect("creation run");

    let before = read_maintained_rows(&project, &recipe);

    // Mutate ONLY the dimension row id=1 references.
    {
        let conn = project.connect().expect("connect for mutation");
        conn.execute(
            &format!(
                "UPDATE main.sources_{} SET {} = 999 WHERE {} = 1",
                recipe.dimension.name, recipe.dimension.payload_column, recipe.dimension.key_column,
            ),
            [],
        )
        .expect("mutate dimension id=1");
    }

    // The catch-up run: same window, so the column-scoped merge resyncs it.
    project
        .run_quiet("probe-catchup", request)
        .await
        .expect("catch-up run");

    let after = read_maintained_rows(&project, &recipe);

    let before_1 = before.iter().find(|r| r.1 == 1).expect("id=1 row before");
    let after_1 = after.iter().find(|r| r.1 == 1).expect("id=1 row after");
    let before_2 = before.iter().find(|r| r.1 == 2).expect("id=2 row before");
    let after_2 = after.iter().find(|r| r.1 == 2).expect("id=2 row after");

    assert_eq!(
        (&before_1.0, before_1.1, before_1.2),
        (&after_1.0, after_1.1, after_1.2),
        "the {{d, id, val}} group (never sensitive to the dimension) must stay byte-unchanged \
         for the mutated row"
    );
    assert_ne!(
        before_1.3, after_1.3,
        "the {{attr}} group (sensitive to the dimension) must reflect the mutation"
    );
    assert_eq!(
        after_1.3, 999,
        "id=1's attr must pick up the mutated dimension value"
    );

    assert_eq!(
        (&before_2.0, before_2.1, before_2.2, before_2.3),
        (&after_2.0, after_2.1, after_2.2, after_2.3),
        "a row referencing an UNMUTATED dimension key must be byte-unchanged in every \
         column group, even though the merge recomputed it"
    );
}

/// `redelivered_window_refuses_for_additive_keyed` (plan Phase 5 TDD list):
/// re-running a folded window refuses (`KeyedReprocessedWindow`) before the
/// action re-runs (`keyed_models.md` §"Reprocessing"; `maintenance_plan.md`
/// §"The reconciliation ledger" — never-fold-twice).
#[tokio::test]
async fn redelivered_window_refuses_for_additive_keyed() {
    let recipe = KeyedRecipe::new_window_forward(KeyedCombiner::Additive);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_keyed_recipe(&recipe, &tmp).expect("stage keyed recipe");

    let plan = classify_keyed(&project, &recipe).expect("classify additive keyed recipe");
    assert!(
        !plan.cells.is_empty(),
        "expected the additive keyed recipe to admit at least one cell: {plan:#?}"
    );

    let d: NaiveDate = NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    insert_row_keyed(&project, &recipe, &GenRow { d, id: 1, val: 5 }).expect("insert row");

    let mut request = base_request("dev");
    request.start = Some("2024-01-01".to_string());
    request.end = Some("2024-01-02".to_string());
    project
        .run_quiet("keyed-redelivery-1", request.clone())
        .await
        .expect("first fold of the window must succeed");

    // Re-deliver the SAME window: never-fold-twice must refuse BEFORE the
    // action re-runs, never silently double-count.
    let rerun = project.run_quiet("keyed-redelivery-2", request).await;
    let err = rerun.expect_err(
        "re-running an already-folded additive keyed window must be refused \
         (KeyedReprocessedWindow) — never-fold-twice",
    );
    let message = format!("{err:#}");
    assert!(
        message.contains("already reflected"),
        "refusal must name the never-fold-twice reason, got: {message}"
    );
}

/// `persisted_reconciliation_store_reflects_recompute_reset` (plan Phase 5
/// TDD list): after two `execute_project` runs of a partition-grain recipe,
/// `.smelt/reconciliation.json` contains recompute-reset entries for
/// exactly the recomputed regions (closes design §2 gap 6 — zero
/// integration coverage of reconciliation-ledger persistence).
#[tokio::test]
async fn persisted_reconciliation_store_reflects_recompute_reset() {
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe(&recipe, &tmp).expect("stage recipe");

    let verdict = classify(&project, &recipe).expect("classify");
    assert!(
        matches!(verdict, Verdict::Admitted(_)),
        "expected the additive-agg append-only recipe to admit: {verdict:?}"
    );

    let d1: NaiveDate = NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    let d2: NaiveDate = NaiveDate::from_ymd_opt(2024, 1, 2).expect("valid date");

    let insert = |d: NaiveDate, id: i64, val: i64| {
        let conn = project.connect().expect("connect for insert");
        conn.execute(
            &format!(
                "INSERT INTO main.sources_{} VALUES (DATE '{}', {}, {})",
                recipe.source.name,
                d.format("%Y-%m-%d"),
                id,
                val,
            ),
            [],
        )
        .expect("insert source row");
    };

    // Run 1: recompute region [2024-01-01, 2024-01-02).
    insert(d1, 1, 10);
    let mut r1 = base_request("dev");
    r1.start = Some("2024-01-01".to_string());
    r1.end = Some("2024-01-02".to_string());
    project.run_quiet("recon-run-1", r1).await.expect("run 1");

    // Run 2: recompute a DISJOINT region [2024-01-02, 2024-01-03).
    insert(d2, 2, 20);
    let mut r2 = base_request("dev");
    r2.start = Some("2024-01-02".to_string());
    r2.end = Some("2024-01-03".to_string());
    project.run_quiet("recon-run-2", r2).await.expect("run 2");

    let store = smelt_state::file_store::FileStore::new(&project.project_dir)
        .load_reconciliation_store()
        .expect("load persisted reconciliation store");
    let ledger = store.get(&recipe.model_name).unwrap_or_else(|| {
        panic!(
            "no reconciliation ledger persisted for model {:?}: store={store:?}",
            recipe.model_name
        )
    });

    let region1 = Region::new("2024-01-01", "2024-01-02");
    let region2 = Region::new("2024-01-02", "2024-01-03");
    assert!(
        ledger.get(&region1, "{*}").is_some(),
        "expected a recompute-reset entry for the first recomputed region {region1:?}, \
         got records={:#?}",
        ledger.records
    );
    assert!(
        ledger.get(&region2, "{*}").is_some(),
        "expected a recompute-reset entry for the second recomputed region {region2:?}, \
         got records={:#?}",
        ledger.records
    );
    assert_eq!(
        ledger.records.len(),
        2,
        "expected exactly the two recomputed regions' entries, got {:#?}",
        ledger.records
    );
}
