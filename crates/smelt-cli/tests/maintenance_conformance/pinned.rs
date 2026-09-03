//! Graduation & consolidation
//! (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 11):
//! one rendering path, one standing gate. This module pins two things as
//! deterministic, always-reproducible cases so they survive independently
//! of proptest's own draw sequence:
//!
//! 1. `pinned_recipes_reproduce_catalogue_coverage` — for every construct ×
//!    posture cell the disposable `property_discovery` catalogue
//!    (`smelt_maintenance_testkit::model_shapes`, now retired) used to
//!    exercise by hand-written SQL, a [`ModelRecipe`]/[`MutableEnrichedRecipe`]/
//!    [`KeyedRecipe`] value (recipe *data*, never a raw SQL string) drawn
//!    deterministically from the same typed generator the standing gate
//!    (`gate.rs`) uses, staged, classified as admitted, and driven through
//!    the real `execute_project` pipeline to a green equivalence check.
//! 2. `hazard_schedules_are_pinned` — one deterministic case per retired
//!    `crates/smelt-cli/tests/property_discovery/{g_*,sc_*,p0_*}.rs` probe
//!    whose construct × hazard the generic gate now subsumes. Each case is
//!    doc-commented with the probe it replaces.
//!
//! ## Retired-probe → pinned-case mapping
//!
//! | Retired probe | Construct | Hazard | Pinned as |
//! |---|---|---|---|
//! | `g_01_additive_agg_append_only` | `BodyConstruct::AdditiveAgg` | none (control) | [`hazard::additive_agg_append_only_control`] |
//! | `g_02_additive_agg_redelivery` | `BodyConstruct::AdditiveAgg` | re-delivery of a processed window | [`hazard::additive_agg_redelivery`] |
//! | `g_03_idempotent_agg_append_only` | `BodyConstruct::IdempotentAgg` | none (control) | [`hazard::idempotent_agg_append_only_control`] |
//! | `g_05_join_enrichment_mutable_dimension` | `MutableEnrichedRecipe` | dimension mutated in place between runs | [`hazard::join_enrichment_mutable_dimension`] |
//! | `g_07_holistic_agg_append_only` | `BodyConstruct::HolisticAgg` | none (control) | [`hazard::holistic_agg_append_only_control`] |
//! | `g_10_composite_key_join_fan_out` | composite-key join (self-rendered, [`hazard::g10`]) | none (control; the fan-out shape itself is the point) | [`hazard::composite_key_join_fan_out`] |
//! | `g_12_keyed_merge_reprocessed_window` | `KeyedRecipe` (additive) | re-delivery of an already-folded keyed window | [`hazard::keyed_merge_reprocessed_window`] |
//! | n/a — self-inverse additive fold (`BIT_XOR`, self-rendered) | keyed `BIT_XOR` fold | re-delivery of an already-folded keyed window | [`hazard::keyed_bit_xor_merge_reprocessed_window`] |
//! | `p0_2_run_schedule`, `p0_4_mutation_profile_selfcheck`, `smoke` | n/a — self-checks of the retired `model_shapes`/`run_schedule` generator infrastructure itself | n/a | superseded outright by `s_tracker.rs`'s own unit tests, `schedule_gen.rs::check_profile`'s unit tests, and this whole standing gate (every case already proves `execute_project` derives a real filter and produces a correct result) |
//!
//! Every OTHER retired-probe construct not in this table (self-referential
//! models, `UNION ALL`, `LEFT JOIN`, correlated `EXISTS`, stacked window
//! frames, cross-source column-name collision, a mutable source aggregated
//! directly) has no equivalent in `smelt_maintenance_testkit::recipe`'s
//! typed vocabulary — those probes were NOT retired; they were made
//! independently compilable (`crates/smelt-cli/tests/property_discovery/shapes.rs`
//! now hosts their model shapes locally) and remain in
//! `tests/property_discovery/` per this phase's "anything the conformance
//! gate does not subsume" carve-out.

use chrono::NaiveDate;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject, SqlCapturingReporter};
use smelt_maintenance_testkit::oracle::multiset_equal_via_backend;
use smelt_maintenance_testkit::recipe::{
    arb_recipe, BodyConstruct, KeyedCombiner, KeyedRecipe, KeyedRunWindow, KeyedSchedule,
    ModelRecipe, MutableEnrichedRecipe, RecipePool,
};
use smelt_maintenance_testkit::schedule_gen::{ConformanceSchedule, ConformanceStep, GenRow};
use smelt_maintenance_testkit::verdict::{classify, Verdict};

use crate::gate::{
    classify_keyed, classify_mixed, drive_and_assert, drive_keyed_and_assert, insert_fact_row,
    snapshot_table_rows, stage_keyed_recipe, stage_mixed_recipe, stage_recipe,
};

fn day(offset: i64) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, 1)
        .expect("valid base date")
        .checked_add_signed(chrono::Duration::days(offset))
        .expect("valid offset date")
}

/// Draw the deterministic-seeded [`ModelRecipe`] whose `construct` first
/// matches `pred`, from `arb_recipe(RecipePool::partition_append_only())`'s
/// own draw sequence. `TestRunner::deterministic()` always replays the same
/// sequence, so the returned recipe is stable across runs — "pinned" by
/// reusing the canonical generator deterministically (design §4 "renders
/// once, serves three"), not by hand-duplicating its rendering logic.
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

/// A minimal, fixed two-row single-window schedule — every `ModelRecipe` in
/// the append-only pool shares the same `events(d, id, val)` source shape,
/// so one schedule shape suffices for every pinned construct case.
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

/// Stage `recipe`, assert it admits (the same construct × posture cell the
/// retired catalogue shape claimed), then drive [`minimal_schedule`]
/// through the real pipeline to a green equivalence check.
fn assert_pinned_body_recipe_is_green(recipe: &ModelRecipe) {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe(recipe, &tmp)
        .unwrap_or_else(|e| panic!("pinned recipe {recipe:?} failed to stage: {e}"));
    let verdict = classify(&project, recipe)
        .unwrap_or_else(|e| panic!("pinned recipe {recipe:?} classify failed: {e}"));
    assert!(
        matches!(verdict, Verdict::Admitted(_)),
        "pinned recipe {recipe:?} did not admit: {verdict:?}"
    );

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(drive_and_assert(&project, recipe, &minimal_schedule()))
        .unwrap_or_else(|e| panic!("pinned recipe {recipe:?} equivalence check failed: {e}"));
}

/// `pinned_recipes_reproduce_catalogue_coverage` (plan Phase 11 TDD list):
/// for each retired-catalogue construct × posture cell, a deterministically
/// drawn recipe renders, admits, and passes a real end-to-end equivalence
/// check.
#[test]
fn pinned_recipes_reproduce_catalogue_coverage() {
    // The six `BodyConstruct` cells (`batched_passthrough`, `G-01`, `G-03`,
    // `G-07`, plus the two recipe-native cells `model_shapes.rs` never had:
    // `Filter`, `DecomposedAgg` — bonus coverage the typed generator adds
    // beyond the retired hand-written catalogue).
    assert_pinned_body_recipe_is_green(&pinned_body_recipe(|c| {
        matches!(c, BodyConstruct::PassThrough)
    }));
    assert_pinned_body_recipe_is_green(&pinned_body_recipe(|c| {
        matches!(c, BodyConstruct::Filter { .. })
    }));
    assert_pinned_body_recipe_is_green(&pinned_body_recipe(|c| {
        matches!(c, BodyConstruct::AdditiveAgg)
    }));
    assert_pinned_body_recipe_is_green(&pinned_body_recipe(|c| {
        matches!(c, BodyConstruct::IdempotentAgg)
    }));
    assert_pinned_body_recipe_is_green(&pinned_body_recipe(|c| {
        matches!(c, BodyConstruct::DecomposedAgg)
    }));
    assert_pinned_body_recipe_is_green(&pinned_body_recipe(|c| {
        matches!(c, BodyConstruct::HolisticAgg)
    }));

    // `G-05`'s inner-join-enrichment-over-a-mutable-dimension cell:
    // `MutableEnrichedRecipe` is already a fixed (non-generated) value —
    // staging + admission is the reproduction, full equivalence is already
    // asserted generically by `gate.rs::mutable_pool_settles_to_full_refresh`
    // over this exact recipe.
    let mixed = MutableEnrichedRecipe::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project =
        stage_mixed_recipe(&mixed, &tmp).unwrap_or_else(|e| panic!("stage mixed recipe: {e}"));
    let plan =
        classify_mixed(&project, &mixed).unwrap_or_else(|e| panic!("classify mixed recipe: {e}"));
    assert!(
        !plan.cells.is_empty(),
        "mutable-dimension-enrichment recipe admitted zero cells: {plan:#?}"
    );

    // `G-12`'s keyed-merge cell: both `KeyedCombiner` families are fixed
    // (non-generated) recipe constructors — staging + admission is the
    // reproduction; `keyed_pool_upholds_end_state_equivalence` already
    // exercises full end-state equivalence generically for both families.
    for combiner in [KeyedCombiner::Additive, KeyedCombiner::Idempotent] {
        let recipe = KeyedRecipe::new_window_forward(combiner);
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_keyed_recipe(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("stage keyed recipe {combiner:?}: {e}"));
        let plan = classify_keyed(&project, &recipe)
            .unwrap_or_else(|e| panic!("classify keyed recipe {combiner:?}: {e}"));
        assert!(
            !plan.cells.is_empty(),
            "keyed recipe {combiner:?} admitted zero cells: {plan:#?}"
        );
    }
}

/// The retired hazard cases (see this module's doc comment table).
mod hazard {
    use super::*;

    /// `g_01_additive_agg_append_only`: the fold-delta happy-path control —
    /// disjoint append-only deltas, no re-delivery, no mutation.
    pub fn additive_agg_append_only_control() {
        assert_pinned_body_recipe_is_green(&pinned_body_recipe(|c| {
            matches!(c, BodyConstruct::AdditiveAgg)
        }));
    }

    /// `g_02_additive_agg_redelivery`: after a window is processed once, the
    /// exact same `[start, end)` window is re-run — never-fold-twice must
    /// hold under the partition-grain DELETE+INSERT full-replace technique
    /// (predicted and confirmed HOLDS: re-delivery is a no-op, not a
    /// double-count).
    pub fn additive_agg_redelivery() {
        let recipe = pinned_body_recipe(|c| matches!(c, BodyConstruct::AdditiveAgg));
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_recipe(&recipe, &tmp).expect("stage recipe");
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
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(drive_and_assert(&project, &recipe, &schedule))
            .unwrap_or_else(|e| panic!("redelivery schedule equivalence check failed: {e}"));
    }

    /// `g_03_idempotent_agg_append_only`: the same happy path as `G-01`, but
    /// the combiner is `MAX` — an idempotent, non-invertible monoid.
    pub fn idempotent_agg_append_only_control() {
        assert_pinned_body_recipe_is_green(&pinned_body_recipe(|c| {
            matches!(c, BodyConstruct::IdempotentAgg)
        }));
    }

    /// `g_05_join_enrichment_mutable_dimension`: a fact/dimension inner
    /// join, batched per partition, where the dimension is mutated in place
    /// between runs — the maintained output must reflect the CURRENT
    /// dimension contents at the next settled point.
    pub fn join_enrichment_mutable_dimension() {
        let recipe = MutableEnrichedRecipe::new();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_mixed_recipe(&recipe, &tmp).expect("stage mixed recipe");
        let plan = classify_mixed(&project, &recipe).expect("classify mixed recipe");
        assert!(!plan.cells.is_empty(), "expected admission: {plan:#?}");

        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            insert_fact_row(
                &project,
                &recipe,
                &GenRow {
                    d: day(0),
                    id: 1,
                    val: Some(10),
                },
            )
            .expect("insert fact row");
            let mut request = base_request("dev");
            request.start = Some("2024-01-01".to_string());
            request.end = Some("2024-01-02".to_string());
            project
                .run_quiet("pinned-g05-run-1", request)
                .await
                .expect("first run");

            // Mutate the dimension in place — the exact hazard `G-05` named.
            let conn = project.connect().expect("connect");
            conn.execute(
                &format!(
                    "UPDATE main.sources_{} SET {} = 999 WHERE {} = 1",
                    recipe.dimension.name,
                    recipe.dimension.payload_column,
                    recipe.dimension.key_column,
                ),
                [],
            )
            .expect("mutate dimension");
            drop(conn);

            // Re-run the SAME window: a catch-up run must reflect the
            // CURRENT dimension contents, not the stale one.
            let mut request = base_request("dev");
            request.start = Some("2024-01-01".to_string());
            request.end = Some("2024-01-02".to_string());
            project
                .run_quiet("pinned-g05-run-2", request)
                .await
                .expect("catch-up run after dimension mutation");

            let backend = project.backend().await.expect("backend");
            let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
            let oracle_sql = recipe.oracle_body_over(&format!("main.sources_{}", recipe.fact.name));
            assert!(
                multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql)
                    .await
                    .expect("catch-up multiset comparison"),
                "catch-up run after in-place dimension mutation diverged from a full-refresh \
                 oracle over the CURRENT dimension contents"
            );
        });
    }

    /// `g_07_holistic_agg_append_only`: `MEDIAN`/exact `COUNT(DISTINCT ...)`
    /// — holistic, non-monoid combiners needing the full per-partition row
    /// set.
    pub fn holistic_agg_append_only_control() {
        assert_pinned_body_recipe_is_green(&pinned_body_recipe(|c| {
            matches!(c, BodyConstruct::HolisticAgg)
        }));
    }

    /// `g_10_composite_key_join_fan_out`: a real fact/dim project where
    /// `dims` is unique only on the PAIR `(user_id, dt)` — neither column
    /// alone is unique. Self-contained (not routed through `ModelRecipe`,
    /// which has no composite-key-join construct): stages its own tiny
    /// project directly, mirroring the retired probe's own staging code
    /// exactly.
    pub fn composite_key_join_fan_out() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().to_path_buf();
        let db_path = project_dir.join("dev.duckdb");
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
        std::fs::write(
            project_dir.join("smelt.yml"),
            format!(
                "name: pinned_g10\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {}\n    schema: main\ndefault_materialization: table\n",
                db_path.display()
            ),
        )
        .expect("write smelt.yml");

        let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
        conn.execute_batch(
            r#"
            CREATE SCHEMA IF NOT EXISTS main;
            CREATE OR REPLACE TABLE main.sources_facts AS
            SELECT * FROM (VALUES
                (DATE '2024-01-01', 1, 100, 10.0),
                (DATE '2024-01-01', 2, 200, 20.0)
            ) AS t(d, user_id, dt, val);
            CREATE OR REPLACE TABLE main.sources_dims AS
            SELECT * FROM (VALUES
                (1, 100, 111),
                (1, 200, 222),
                (2, 100, 333),
                (2, 200, 444)
            ) AS t(user_id, dt, payload);
            "#,
        )
        .expect("seed sources");
        drop(conn);

        let project = LinkCProject::load(project_dir, db_path).expect("load project");
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async {
            let mut request = base_request("dev");
            request.start = Some("2024-01-01".to_string());
            request.end = Some("2024-01-02".to_string());
            let reporter = SqlCapturingReporter::new();
            project
                .run("pinned-g10", request, &reporter)
                .await
                .expect("run must succeed");

            let backend = project.backend().await.expect("backend after run");
            let maintained_sql =
                "SELECT d, user_id, dt, val, payload FROM main.facts_enriched WHERE d = DATE '2024-01-01'";
            let oracle_sql = "SELECT f.d, f.user_id, f.dt, f.val, dm.payload \
                 FROM main.sources_facts f \
                 JOIN main.sources_dims dm ON f.user_id = dm.user_id AND f.dt = dm.dt \
                 WHERE f.d = DATE '2024-01-01'";
            assert!(
                multiset_equal_via_backend(backend.as_ref(), maintained_sql, oracle_sql)
                    .await
                    .expect("composite-key multiset comparison"),
                "composite-key (user_id, dt) join fan-out diverged from the full-refresh oracle"
            );
        });
    }

    /// `g_12_keyed_merge_reprocessed_window`: re-running an already-folded
    /// additive keyed window must be refused (`KeyedReprocessedWindow`)
    /// before the action re-runs — never-fold-twice.
    pub fn keyed_merge_reprocessed_window() {
        let recipe = KeyedRecipe::new_window_forward(KeyedCombiner::Additive);
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_keyed_recipe(&recipe, &tmp).expect("stage keyed recipe");
        let plan = classify_keyed(&project, &recipe).expect("classify keyed recipe");
        assert!(!plan.cells.is_empty(), "expected admission: {plan:#?}");

        let schedule = KeyedSchedule(vec![KeyedRunWindow {
            start: day(0),
            end: day(1),
            rows: vec![GenRow {
                d: day(0),
                id: 1,
                val: Some(5),
            }],
        }]);
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(drive_keyed_and_assert(&project, &recipe, &schedule))
            .expect("first fold must succeed");

        let maintained_before = rt.block_on(async {
            let backend = project.backend().await.expect("backend");
            snapshot_table_rows(backend.as_ref(), &recipe.model_name)
                .await
                .expect("snapshot before the refused redelivery")
        });

        let mut request = base_request("dev");
        request.start = Some("2024-01-01".to_string());
        request.end = Some("2024-01-02".to_string());
        let rerun = rt.block_on(project.run_quiet("pinned-g12-redelivery", request));
        let err = rerun.expect_err(
            "re-running an already-folded additive keyed window must be refused \
             (KeyedReprocessedWindow) — never-fold-twice",
        );
        let message = format!("{err:#}");
        assert!(
            message.contains("already reflected"),
            "refusal must name the never-fold-twice reason, got: {message}"
        );
        assert!(
            message.contains("KeyedReprocessedWindow"),
            "refusal must name the diagnostic code KeyedReprocessedWindow, got: {message}"
        );
        assert!(
            message.contains("2024-01-01"),
            "refusal must name the reprocessed window's bounds, got: {message}"
        );
        assert!(
            message.contains("--full-refresh"),
            "refusal must point at the --full-refresh remedy, got: {message}"
        );

        let maintained_after = rt.block_on(async {
            let backend = project.backend().await.expect("backend");
            snapshot_table_rows(backend.as_ref(), &recipe.model_name)
                .await
                .expect("snapshot after the refused redelivery")
        });
        assert_eq!(
            maintained_before, maintained_after,
            "the refused redelivery must leave the maintained table's contents byte-identical \
             — the refusal happens before any write"
        );
    }

    /// `BIT_XOR` is an **additive fold** — a commutative *group*, not an
    /// idempotent lattice (`docs/specs/incremental_shapes.md` §"The
    /// column-family catalogue"). Because XOR is self-inverse, re-merging an
    /// already-reflected delta computes `x XOR d XOR d == x`: it CANCELS the
    /// window's contribution instead of converging, so `incremental_state(S)
    /// == full_refresh(inputs ∈ S)` would be violated silently. The
    /// reconciliation ledger must therefore cover this family exactly as it
    /// covers `SUM` — the redelivery is refused (`KeyedReprocessedWindow`)
    /// before any write, leaving state unchanged
    /// (§"The transactional merge ledger").
    ///
    /// Hand-staged rather than drawn from [`KeyedRecipe`]: the typed keyed
    /// pool renders the additive family as `SUM` only, so no generated case
    /// reaches the `BIT_XOR` combiner.
    pub fn keyed_bit_xor_merge_reprocessed_window() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().to_path_buf();
        let db_path = project_dir.join("dev.duckdb");
        std::fs::create_dir_all(project_dir.join("models/sources")).expect("mkdir");

        std::fs::write(
            project_dir.join("models/device_bits.sql"),
            r#"---
materialization: table
refresh: incremental
grain: key
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
maintenance:
  scan_bounds:
    per_source:
      events:
        allow_full_scan: true
---
SELECT
    device_id,
    d,
    BIT_XOR(bits) AS xor_bits
FROM smelt.sources.events
GROUP BY 1, 2
"#,
        )
        .expect("write model");
        std::fs::write(
            project_dir.join("models/sources/events.yml"),
            "description: pinned BIT_XOR keyed source.\ncolumns:\n  - name: device_id\n    type: BIGINT\n  - name: d\n    type: DATE\n  - name: bits\n    type: BIGINT\ntimeseries:\n  event_time_column: d\n  partition_column: d\n  granularity: day\nmutation_profile:\n  kind: append_only\n",
        )
        .expect("write source");
        std::fs::write(
            project_dir.join("smelt.yml"),
            format!(
                "name: pinned_bit_xor\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {}\n    schema: main\ndefault_materialization: table\n",
                db_path.display()
            ),
        )
        .expect("write smelt.yml");

        let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
        conn.execute_batch(
            r#"
            CREATE SCHEMA IF NOT EXISTS main;
            CREATE OR REPLACE TABLE main.sources_events AS
            SELECT * FROM (VALUES
                (1, DATE '2024-01-01', 12),
                (1, DATE '2024-01-01', 10)
            ) AS t(device_id, d, bits);
            "#,
        )
        .expect("seed source");
        drop(conn);

        let project = LinkCProject::load(project_dir, db_path).expect("load project");
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

        let mut first = base_request("dev");
        first.start = Some("2024-01-01".to_string());
        first.end = Some("2024-01-02".to_string());
        rt.block_on(project.run_quiet("pinned-bitxor-first", first))
            .expect("first fold must succeed");

        let maintained_before = rt.block_on(async {
            let backend = project.backend().await.expect("backend");
            snapshot_table_rows(backend.as_ref(), "device_bits")
                .await
                .expect("snapshot before the refused redelivery")
        });
        assert_eq!(
            maintained_before,
            vec![vec![
                "1".to_string(),
                "2024-01-01".to_string(),
                "6".to_string()
            ]],
            "first fold must equal the full-refresh oracle (12 ^ 10 = 6)"
        );

        let mut rerun = base_request("dev");
        rerun.start = Some("2024-01-01".to_string());
        rerun.end = Some("2024-01-02".to_string());
        let err = rt
            .block_on(project.run_quiet("pinned-bitxor-redelivery", rerun))
            .expect_err(
                "re-running an already-folded BIT_XOR keyed window must be refused \
                 (KeyedReprocessedWindow) — XOR is self-inverse, so the re-merge would cancel \
                 the window's contribution rather than converge",
            );
        let message = format!("{err:#}");
        assert!(
            message.contains("KeyedReprocessedWindow"),
            "refusal must name the diagnostic code KeyedReprocessedWindow, got: {message}"
        );
        assert!(
            message.contains("2024-01-01"),
            "refusal must name the reprocessed window's bounds, got: {message}"
        );

        let maintained_after = rt.block_on(async {
            let backend = project.backend().await.expect("backend");
            snapshot_table_rows(backend.as_ref(), "device_bits")
                .await
                .expect("snapshot after the refused redelivery")
        });
        assert_eq!(
            maintained_before, maintained_after,
            "the refused redelivery must leave the maintained table unchanged — never the \
             cancelled `x XOR d XOR d == x` state"
        );
    }
}

/// `hazard_schedules_are_pinned` (plan Phase 11 TDD list): each retired
/// probe's seeded hazard schedule exists as a deterministic pinned case
/// (see this module's doc comment table).
#[test]
fn hazard_schedules_are_pinned() {
    hazard::additive_agg_append_only_control();
    hazard::additive_agg_redelivery();
    hazard::idempotent_agg_append_only_control();
    hazard::join_enrichment_mutable_dimension();
    hazard::holistic_agg_append_only_control();
    hazard::composite_key_join_fan_out();
    hazard::keyed_merge_reprocessed_window();
    hazard::keyed_bit_xor_merge_reprocessed_window();
}
