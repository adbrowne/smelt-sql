//! The pinned catalogue-coverage + hazard-schedule family — the
//! target-parametrized twin of `maintenance_conformance/pinned.rs`:
//! reproduces the same construct × posture catalogue coverage and hazard
//! schedules, reusing the already-generic staging/drive helpers from
//! [`crate::families::gate`], [`crate::families::gate_keyed`], and
//! [`crate::families::gate_mixed`] rather than duplicating rendering logic.
//!
//! `hazard::keyed_merge_reprocessed_window` is the ONE hazard case NOT
//! ported: it specifically exercises `KeyedCombiner::Additive`'s
//! never-fold-twice REFUSAL, which requires the reconciliation ledger that
//! is DuckDB-only today (no non-DuckDB dialect for MP12's ledger DDL yet) —
//! on a non-DuckDB backend the very first fold in that hazard's own schedule
//! would fail loud with the ledger's own `BackendError::unsupported` rather
//! than the hazard's own `KeyedReprocessedWindow` refusal, so the case does
//! not reproduce (there is no non-DuckDB equivalent to pin).

use anyhow::Result;
use chrono::NaiveDate;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use crate::families::gate::{drive_and_assert_for, stage_recipe_for};
use crate::families::gate_keyed::stage_keyed_recipe_for;
use crate::families::gate_mixed::{classify_mixed, insert_fact_row_for, stage_mixed_recipe_for};
use crate::families::ConformanceBackend;
use crate::oracle::multiset_equal_via_backend_with_diff;
use crate::recipe::{
    arb_recipe, BodyConstruct, KeyedCombiner, KeyedRecipe, ModelRecipe, MutableEnrichedRecipe,
    RecipePool,
};
use crate::schedule_gen::{ConformanceSchedule, ConformanceStep, GenRow};
use crate::verdict::{classify, classify_keyed, Verdict};

fn day(offset: i64) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, 1)
        .expect("valid base date")
        .checked_add_signed(chrono::Duration::days(offset))
        .expect("valid offset date")
}

/// Pure generator draw, no backend dependency, so identical regardless of
/// target.
fn pinned_body_recipe(pred: impl Fn(&BodyConstruct) -> bool) -> ModelRecipe {
    let mut runner = TestRunner::deterministic();
    let strat = arb_recipe(RecipePool::partition_append_only());
    for _ in 0..500 {
        let recipe = strat.new_tree(&mut runner).unwrap().current();
        if pred(&recipe.construct) {
            return recipe;
        }
    }
    panic!("deterministic sample never produced a matching recipe in 500 draws");
}

/// Pure data, no backend dependency.
fn minimal_schedule() -> ConformanceSchedule {
    ConformanceSchedule(vec![ConformanceStep::RunWindow {
        start: day(0),
        end: day(1),
        rows: vec![
            GenRow {
                d: day(0),
                id: 1,
                val: Some(10),
            },
            GenRow {
                d: day(0),
                id: 2,
                val: Some(-5),
            },
        ],
    }])
}

/// `case` selects the physical target this ONE recipe stages into
/// (`ConformanceBackend::target`/`schema`) — every independent
/// `assert_pinned_body_recipe_is_green` call in this family drives its own
/// physical project, so callers must give each call a case DISTINCT from
/// every other physical staging in the same sweep (mirrors
/// `families::dags::stage_pair_for`'s `target`/`twin_target` split: a
/// per-case-dataset backend, e.g. BigQuery, has no drop-before-create on its
/// source-table staging, so two stagings sharing one case collide with `409
/// Already Exists` on the second `CREATE TABLE` — see
/// `docs/plans/20260817-bigquery-generative-conformance.md` Phase 7's
/// "Measured result").
async fn assert_pinned_body_recipe_is_green(
    b: &dyn ConformanceBackend,
    case: usize,
    recipe: &ModelRecipe,
) {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe_for(recipe, &tmp, b.target(case))
        .unwrap_or_else(|e| panic!("pinned recipe {recipe:?} failed to stage: {e}"));
    let verdict = classify(&project, recipe)
        .unwrap_or_else(|e| panic!("pinned recipe {recipe:?} classify failed: {e}"));
    assert!(
        matches!(verdict, Verdict::Admitted(_)),
        "pinned recipe {recipe:?} did not admit: {verdict:?}"
    );

    drive_and_assert_for(b, case, &project, recipe, &minimal_schedule())
        .await
        .unwrap_or_else(|e| panic!("pinned recipe {recipe:?} equivalence check failed: {e}"));
}

/// `pinned_recipes_reproduce_catalogue_coverage` — the twin of
/// `maintenance_conformance::pinned::pinned_recipes_reproduce_catalogue_coverage`.
pub async fn run_pinned_recipes_reproduce_catalogue_coverage(
    b: &dyn ConformanceBackend,
) -> Result<()> {
    assert_pinned_body_recipe_is_green(
        b,
        0,
        &pinned_body_recipe(|c| matches!(c, BodyConstruct::PassThrough)),
    )
    .await;
    assert_pinned_body_recipe_is_green(
        b,
        1,
        &pinned_body_recipe(|c| matches!(c, BodyConstruct::Filter { .. })),
    )
    .await;
    assert_pinned_body_recipe_is_green(
        b,
        2,
        &pinned_body_recipe(|c| matches!(c, BodyConstruct::AdditiveAgg)),
    )
    .await;
    assert_pinned_body_recipe_is_green(
        b,
        3,
        &pinned_body_recipe(|c| matches!(c, BodyConstruct::IdempotentAgg)),
    )
    .await;
    assert_pinned_body_recipe_is_green(
        b,
        4,
        &pinned_body_recipe(|c| matches!(c, BodyConstruct::DecomposedAgg)),
    )
    .await;
    assert_pinned_body_recipe_is_green(
        b,
        5,
        &pinned_body_recipe(|c| matches!(c, BodyConstruct::HolisticAgg)),
    )
    .await;

    let mixed = MutableEnrichedRecipe::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_mixed_recipe_for(b, 6, &mixed, &tmp)
        .await
        .unwrap_or_else(|e| panic!("stage mixed recipe: {e}"));
    let plan =
        classify_mixed(&project, &mixed).unwrap_or_else(|e| panic!("classify mixed recipe: {e}"));
    anyhow::ensure!(
        !plan.cells.is_empty(),
        "mutable-dimension-enrichment recipe admitted zero cells: {plan:#?}"
    );

    // Both `KeyedCombiner` families classify cleanly — admission is pure
    // classification, unaffected by the `Grade::Additive` ledger gap that
    // excludes execution of the Additive family elsewhere. Each combiner is
    // its own physical staging, so each gets its own case (7, 8) — see
    // `assert_pinned_body_recipe_is_green`'s doc comment for why a shared
    // case collides on a per-case-dataset backend.
    for (combiner, case) in [(KeyedCombiner::Additive, 7), (KeyedCombiner::Idempotent, 8)] {
        let recipe = KeyedRecipe::new_window_forward(combiner);
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_keyed_recipe_for(&recipe, &tmp, b.target(case))
            .unwrap_or_else(|e| panic!("stage keyed recipe {combiner:?}: {e}"));
        let verdict = classify_keyed(&project, &recipe)
            .unwrap_or_else(|e| panic!("classify keyed recipe {combiner:?}: {e}"));
        anyhow::ensure!(
            matches!(verdict, Verdict::Admitted(_)),
            "keyed recipe {combiner:?} did not admit: {verdict:?}"
        );
    }
    Ok(())
}

/// The retired hazard cases (see this module's doc comment for
/// `keyed_merge_reprocessed_window`'s exclusion).
mod hazard {
    use super::*;

    pub async fn additive_agg_append_only_control(b: &dyn ConformanceBackend, case: usize) {
        assert_pinned_body_recipe_is_green(
            b,
            case,
            &pinned_body_recipe(|c| matches!(c, BodyConstruct::AdditiveAgg)),
        )
        .await;
    }

    pub async fn additive_agg_redelivery(b: &dyn ConformanceBackend, case: usize) {
        let recipe = pinned_body_recipe(|c| matches!(c, BodyConstruct::AdditiveAgg));
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_recipe_for(&recipe, &tmp, b.target(case)).expect("stage recipe");
        let verdict = classify(&project, &recipe).expect("classify");
        assert!(
            matches!(verdict, Verdict::Admitted(_)),
            "expected admission"
        );

        let schedule = ConformanceSchedule(vec![
            ConformanceStep::RunWindow {
                start: day(0),
                end: day(1),
                rows: vec![GenRow {
                    d: day(0),
                    id: 1,
                    val: Some(10),
                }],
            },
            ConformanceStep::RerunWindow {
                start: day(0),
                end: day(1),
            },
        ]);
        drive_and_assert_for(b, case, &project, &recipe, &schedule)
            .await
            .unwrap_or_else(|e| panic!("redelivery schedule equivalence check failed: {e}"));
    }

    pub async fn idempotent_agg_append_only_control(b: &dyn ConformanceBackend, case: usize) {
        assert_pinned_body_recipe_is_green(
            b,
            case,
            &pinned_body_recipe(|c| matches!(c, BodyConstruct::IdempotentAgg)),
        )
        .await;
    }

    pub async fn join_enrichment_mutable_dimension(b: &dyn ConformanceBackend, case: usize) {
        let recipe = MutableEnrichedRecipe::new();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_mixed_recipe_for(b, case, &recipe, &tmp)
            .await
            .expect("stage mixed recipe");
        let plan = classify_mixed(&project, &recipe).expect("classify mixed recipe");
        assert!(!plan.cells.is_empty(), "expected admission: {plan:#?}");

        let target = b.target(case);
        let schema = b.schema(case);
        let backend = project
            .backend_for_target(target.clone())
            .await
            .expect("open backend");

        insert_fact_row_for(
            backend.as_ref(),
            &schema,
            &recipe,
            &GenRow {
                d: day(0),
                id: 1,
                val: Some(10),
            },
        )
        .await
        .expect("insert fact row");
        let mut request = crate::link_c_harness::base_request(b.engine_name());
        request.start = Some("2024-01-01".to_string());
        request.end = Some("2024-01-02".to_string());
        project
            .run_with_target(
                target.clone(),
                &format!("pinned-g05-{}-run-1", b.engine_name()),
                request,
                &smelt_runtime::NoOpReporter,
            )
            .await
            .expect("first run");

        // Mutate the dimension in place — the exact hazard `G-05` named.
        backend
            .execute_sql(&format!(
                "UPDATE {schema}.sources_{name} SET {attr} = 999 WHERE {key} = 1",
                name = recipe.dimension.name,
                attr = recipe.dimension.payload_column,
                key = recipe.dimension.key_column,
            ))
            .await
            .expect("mutate dimension");

        // Re-run the SAME window: a catch-up run must reflect the CURRENT
        // dimension contents, not the stale one.
        let mut request = crate::link_c_harness::base_request(b.engine_name());
        request.start = Some("2024-01-01".to_string());
        request.end = Some("2024-01-02".to_string());
        project
            .run_with_target(
                target.clone(),
                &format!("pinned-g05-{}-run-2", b.engine_name()),
                request,
                &smelt_runtime::NoOpReporter,
            )
            .await
            .expect("catch-up run after dimension mutation");

        let maintained_sql = format!("SELECT * FROM {schema}.{}", recipe.model_name);
        let oracle_sql = recipe
            .model_body()
            .replace(
                &format!("smelt.sources.{}", recipe.dimension.name),
                &format!("{schema}.sources_{}", recipe.dimension.name),
            )
            .replace(
                &format!("smelt.sources.{}", recipe.fact.name),
                &format!("{schema}.sources_{}", recipe.fact.name),
            );
        assert!(
            multiset_equal_via_backend_with_diff(
                backend.as_ref(),
                &maintained_sql,
                &oracle_sql,
                |l, r| b.multiset_diff_sql(l, r)
            )
            .await
            .expect("catch-up multiset comparison"),
            "catch-up run after in-place dimension mutation diverged from a full-refresh \
             oracle over the CURRENT dimension contents"
        );
    }

    pub async fn holistic_agg_append_only_control(b: &dyn ConformanceBackend, case: usize) {
        assert_pinned_body_recipe_is_green(
            b,
            case,
            &pinned_body_recipe(|c| matches!(c, BodyConstruct::HolisticAgg)),
        )
        .await;
    }

    /// `g_10_composite_key_join_fan_out`: a real fact/dim project where
    /// `dims` is unique only on the PAIR `(user_id, dt)`. Self-contained
    /// (mirrors `pinned.rs`'s own self-staged project).
    pub async fn composite_key_join_fan_out(b: &dyn ConformanceBackend, case: usize) {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().to_path_buf();
        let db_path = project_dir.join("unused.duckdb");
        std::fs::create_dir_all(project_dir.join("models/sources")).expect("mkdir");

        std::fs::write(
            project_dir.join("models/facts_enriched.sql"),
            r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: incremental
grain: partition
maintenance:
  scan_bounds:
    per_source:
      dims:
        allow_full_scan: true
---
SELECT f.d, f.user_id, f.dt, f.val, dm.payload
FROM smelt.sources.facts f
JOIN smelt.sources.dims dm ON f.user_id = dm.user_id AND f.dt = dm.dt
"#,
        )
        .expect("write model");
        std::fs::write(
            project_dir.join("models/sources/facts.yml"),
            "description: pinned g-10 fact source.\ncolumns:\n  - name: d\n    type: DATE\n  - name: user_id\n    type: BIGINT\n  - name: dt\n    type: BIGINT\n  - name: val\n    type: DOUBLE\ntimeseries:\n  event_time_column: d\n  partition_column: d\n  granularity: day\n",
        )
        .expect("write facts source");
        std::fs::write(
            project_dir.join("models/sources/dims.yml"),
            "description: pinned g-10 composite-key dimension source.\ncolumns:\n  - name: user_id\n    type: BIGINT\n  - name: dt\n    type: BIGINT\n  - name: payload\n    type: BIGINT\n",
        )
        .expect("write dims source");
        let target = b.target(case);
        let schema = b.schema(case);
        std::fs::write(
            project_dir.join("smelt.yml"),
            crate::render::render_smelt_yml_for(target.clone(), &db_path),
        )
        .expect("write smelt.yml");

        let backend = b
            .open_backend(case, &db_path)
            .await
            .expect("open backend for g-10 staging");
        let storage_clause = b.storage_clause();
        let int_type = b.int_type();
        let double_type = b.double_type();
        for table in ["facts_enriched", "sources_facts", "sources_dims"] {
            backend
                .execute_sql(&format!("DROP TABLE IF EXISTS {schema}.{table}"))
                .await
                .expect("drop stale g-10 table");
        }
        backend
            .execute_sql(&format!(
                "CREATE TABLE {schema}.sources_facts (d DATE, user_id {int_type}, dt \
                 {int_type}, val {double_type}){storage_clause}"
            ))
            .await
            .expect("create g-10 facts table");
        backend
            .execute_sql(&format!(
                "INSERT INTO {schema}.sources_facts VALUES (DATE '2024-01-01', 1, 100, 10.0), \
                 (DATE '2024-01-01', 2, 200, 20.0)"
            ))
            .await
            .expect("seed g-10 facts");
        backend
            .execute_sql(&format!(
                "CREATE TABLE {schema}.sources_dims (user_id {int_type}, dt {int_type}, \
                 payload {int_type}){storage_clause}"
            ))
            .await
            .expect("create g-10 dims table");
        backend
            .execute_sql(&format!(
                "INSERT INTO {schema}.sources_dims VALUES (1, 100, 111), (1, 200, 222), (2, \
                 100, 333), (2, 200, 444)"
            ))
            .await
            .expect("seed g-10 dims");

        let project = crate::link_c_harness::LinkCProject::load(project_dir, db_path)
            .expect("load g-10 project");
        let mut request = crate::link_c_harness::base_request(b.engine_name());
        request.start = Some("2024-01-01".to_string());
        request.end = Some("2024-01-02".to_string());
        project
            .run_with_target(
                target,
                &format!("pinned-g10-{}", b.engine_name()),
                request,
                &smelt_runtime::NoOpReporter,
            )
            .await
            .expect("g-10 run must succeed");

        let maintained_sql = format!(
            "SELECT d, user_id, dt, val, payload FROM {schema}.facts_enriched WHERE d = DATE \
             '2024-01-01'"
        );
        let oracle_sql = format!(
            "SELECT f.d, f.user_id, f.dt, f.val, dm.payload FROM {schema}.sources_facts f JOIN \
             {schema}.sources_dims dm ON f.user_id = dm.user_id AND f.dt = dm.dt WHERE f.d = \
             DATE '2024-01-01'"
        );
        assert!(
            multiset_equal_via_backend_with_diff(
                backend.as_ref(),
                &maintained_sql,
                &oracle_sql,
                |l, r| b.multiset_diff_sql(l, r)
            )
            .await
            .expect("composite-key multiset comparison"),
            "composite-key (user_id, dt) join fan-out diverged from the full-refresh oracle"
        );
    }
}

/// `hazard_schedules_are_pinned` — the twin of
/// `maintenance_conformance::pinned::hazard_schedules_are_pinned`, minus
/// `keyed_merge_reprocessed_window` (see this module's doc comment).
pub async fn run_hazard_schedules_are_pinned(b: &dyn ConformanceBackend) -> Result<()> {
    // Each hazard case stages its own physical project, so each gets a
    // DISTINCT case — see `assert_pinned_body_recipe_is_green`'s doc comment
    // for why a shared case collides on a per-case-dataset backend.
    hazard::additive_agg_append_only_control(b, 0).await;
    hazard::additive_agg_redelivery(b, 1).await;
    hazard::idempotent_agg_append_only_control(b, 2).await;
    hazard::join_enrichment_mutable_dimension(b, 3).await;
    hazard::holistic_agg_append_only_control(b, 4).await;
    hazard::composite_key_join_fan_out(b, 5).await;
    Ok(())
}

#[cfg(test)]
mod case_regression_tests {
    use std::path::Path;

    fn read_source() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/families/pinned.rs");
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
    }

    /// Returns the `{ ... }` body text of `fn <fn_name>(` (brace-depth
    /// matched, mirrors `families::dags`'s own
    /// `stage_pair_for_reaches_twin_target_through_the_seam_not_a_second_target_call`
    /// convention).
    fn extract_fn_body<'a>(contents: &'a str, fn_name: &str) -> &'a str {
        let sig = format!("fn {fn_name}(");
        let start = contents
            .find(&sig)
            .unwrap_or_else(|| panic!("{fn_name} must still exist in this file"));
        let body_start = contents[start..]
            .find('{')
            .map(|i| start + i)
            .unwrap_or_else(|| panic!("{fn_name} must have a body"));
        let bytes = contents.as_bytes();
        let mut depth = 0i32;
        let mut i = body_start;
        loop {
            match bytes[i] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &contents[body_start..=i];
                    }
                }
                _ => {}
            }
            i += 1;
            if i >= bytes.len() {
                panic!("{fn_name}'s body never closes");
            }
        }
    }

    /// Extracts the top-level (paren-depth-0) argument at `arg_index` for
    /// every call to `call_name(...)` found in `body`, parsed as a leading
    /// integer literal — i.e. the CASE argument this family's calls pass to
    /// `assert_pinned_body_recipe_is_green`/`stage_mixed_recipe_for`/a
    /// `hazard::*` entry point.
    fn extract_case_args(body: &str, call_name: &str, arg_index: usize) -> Vec<usize> {
        let marker = format!("{call_name}(");
        let mut cases = Vec::new();
        let mut search_from = 0;
        while let Some(rel) = body[search_from..].find(&marker) {
            let call_start = search_from + rel + marker.len();
            let bytes = body.as_bytes();
            let mut depth = 0i32;
            let mut i = call_start;
            let mut arg_start = call_start;
            let mut args: Vec<&str> = Vec::new();
            loop {
                match bytes[i] {
                    b'(' | b'[' | b'{' => depth += 1,
                    b')' if depth == 0 => {
                        args.push(body[arg_start..i].trim());
                        break;
                    }
                    b')' | b']' | b'}' => depth -= 1,
                    b',' if depth == 0 => {
                        args.push(body[arg_start..i].trim());
                        arg_start = i + 1;
                    }
                    _ => {}
                }
                i += 1;
                if i >= bytes.len() {
                    panic!("call to {call_name} starting at byte {call_start} never closes its argument list");
                }
            }
            if let Some(arg) = args.get(arg_index) {
                let digits: String = arg.chars().take_while(|c| c.is_ascii_digit()).collect();
                assert!(
                    !digits.is_empty(),
                    "expected {call_name}'s argument {arg_index} to be a leading-digit case \
                     literal, got {arg:?}"
                );
                cases.push(digits.parse().expect("parse case literal"));
            }
            search_from = call_start;
        }
        cases
    }

    /// `each_physical_staging_in_the_catalogue_sweep_gets_a_distinct_case`
    /// (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 7's
    /// "Measured result" — `pinned_recipes_reproduce_catalogue_coverage_on_bigquery`
    /// failed with `409 Already Exists: Table …sources_events`): before this
    /// fix, every physical staging in
    /// `run_pinned_recipes_reproduce_catalogue_coverage` called
    /// `assert_pinned_body_recipe_is_green(b, &recipe)` — no `case` argument
    /// at all, so every one of the 9 independent projects this sweep stages
    /// resolved to `b.target(0)`. `render.rs`'s
    /// `create_bigquery_source_table` issues a bare `CREATE TABLE` with no
    /// drop-before-create (unlike Spark's `reset_and_create_spark_source_table`
    /// or DuckDB's own fresh-tempdir-per-project isolation), so the SECOND
    /// staging into the SAME per-case BigQuery dataset collided with `409
    /// Already Exists` on `sources_events` — structurally the same
    /// two-writers-one-target collision Phase 5b fixed for
    /// `families::dags::stage_pair_for`'s incremental/full-refresh twin, just
    /// with N independent stagings sharing one case instead of two. The fix
    /// threads a distinct `case` through every physical staging in this
    /// sweep, reached through the SAME `ConformanceBackend::target(case)`/
    /// `schema(case)` seam `families::dags` already uses — never a second
    /// mechanism, never a naming convention this test (or a future caller)
    /// would have to remember to honour.
    #[test]
    fn each_physical_staging_in_the_catalogue_sweep_gets_a_distinct_case() {
        let contents = read_source();
        let body = extract_fn_body(&contents, "run_pinned_recipes_reproduce_catalogue_coverage");

        let mut cases = extract_case_args(body, "assert_pinned_body_recipe_is_green", 1);
        cases.extend(extract_case_args(body, "stage_mixed_recipe_for", 1));
        // The keyed loop tuples each combiner with its case
        // (`(KeyedCombiner::X, N)`) rather than passing it as a direct call
        // argument — pull N out of each tuple by name instead.
        for marker in ["KeyedCombiner::Additive, ", "KeyedCombiner::Idempotent, "] {
            let start = body
                .find(marker)
                .unwrap_or_else(|| panic!("expected {marker:?} in the keyed loop"));
            let after = &body[start + marker.len()..];
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            assert!(
                !digits.is_empty(),
                "expected a case literal immediately after {marker:?}"
            );
            cases.push(digits.parse().expect("parse keyed combiner case literal"));
        }

        assert!(
            cases.len() >= 9,
            "expected at least 9 distinct physical stagings in the catalogue coverage sweep \
             (6 body-construct recipes + 1 mixed recipe + 2 keyed recipes), found {}: {cases:?}",
            cases.len()
        );
        let mut dedup = cases.clone();
        dedup.sort_unstable();
        dedup.dedup();
        assert_eq!(
            dedup.len(),
            cases.len(),
            "two distinct physical stagings in the catalogue coverage sweep share the same \
             case — on a per-case-dataset backend (BigQuery) this collides with `409 Already \
             Exists` on the second staging's `CREATE TABLE sources_<name>`: {cases:?}"
        );
    }

    /// The hazard-sweep twin of the test above — each of the 6 hazard cases
    /// in `run_hazard_schedules_are_pinned` stages its own physical project
    /// and must get its own case.
    #[test]
    fn each_hazard_case_gets_a_distinct_case() {
        let contents = read_source();
        let body = extract_fn_body(&contents, "run_hazard_schedules_are_pinned");
        let mut cases = Vec::new();
        for name in [
            "hazard::additive_agg_append_only_control",
            "hazard::additive_agg_redelivery",
            "hazard::idempotent_agg_append_only_control",
            "hazard::join_enrichment_mutable_dimension",
            "hazard::holistic_agg_append_only_control",
            "hazard::composite_key_join_fan_out",
        ] {
            cases.extend(extract_case_args(body, name, 1));
        }
        assert_eq!(
            cases.len(),
            6,
            "expected exactly 6 hazard-case call sites, each passing an explicit case \
             literal as its second argument, found {cases:?}"
        );
        let mut dedup = cases.clone();
        dedup.sort_unstable();
        dedup.dedup();
        assert_eq!(
            dedup.len(),
            cases.len(),
            "two hazard cases share the same case — on a per-case-dataset backend (BigQuery) \
             this collides with `409 Already Exists` on the second case's own source-table \
             staging: {cases:?}"
        );
    }
}
