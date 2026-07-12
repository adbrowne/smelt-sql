//! Plan-claim probes (design doc
//! `docs/research/20260711-generative-maintenance-conformance.md` §7
//! "Plan-claim probes — checking that derived properties hold";
//! `docs/plans/20260712-generative-maintenance-conformance.md` Phase 4): a
//! direct runtime check that a derived plan claim actually holds, beyond
//! end-state equivalence alone — end-state equivalence can miss a claim
//! being wrong in a compensating way.

use chrono::NaiveDate;

use smelt_logical::maintenance::{Technique, Trigger};
use smelt_maintenance_testkit::link_c_harness::base_request;
use smelt_maintenance_testkit::recipe::MutableEnrichedRecipe;
use smelt_maintenance_testkit::schedule_gen::GenRow;

use crate::gate::{classify_mixed, insert_fact_row, stage_mixed_recipe};

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
